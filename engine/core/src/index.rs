//! A scan's every point filed by cell, in an index file on disk.
//!
//! The file (little-endian):
//!
//! - a 512-byte head: what it is, the scan file it was made from (its size
//!   and date), the file's coordinate grid (scale and offset), the cells'
//!   size and origin (in the file's whole-number units), where the tables
//!   and points start, and a last flag written only once all is in;
//! - the cells, each a run of points (its first, how many, and the box its
//!   points take), in order of the block of 8 × 8 × 8 cells each is in;
//! - the blocks (the cells each holds, and their box), so a question rules
//!   out most of a scan by a few hundred boxes;
//! - the points, 24 bytes each, cell by cell: x, y, z as the file's whole
//!   numbers (i32), intensity (u16), class (u8), flags (u8: synthetic 1,
//!   key-point 2, withheld 4, overlap 8), returns (u8: the return number in
//!   the low 4 bits, the pulse's number of returns in the high 4), a spare
//!   byte, then red, green and blue (u16 each).
//!
//! Made in two passes: the file read once (LAZ on every core), each point
//! counted to its cell and written in turn to a scratch file; then the
//! scratch read back in order and each point put in its cell's place.

use crate::fx::FxMap;
use memmap2::{Mmap, MmapMut};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

pub const MAGIC: &[u8; 8] = b"CADPENG1";
pub const VERSION: u32 = 1;
pub const HEAD: usize = 512;
pub const RECORD: usize = 24;
pub const CELL: usize = 40;
pub const BLOCK: usize = 48;

/// About this many points to a cell, where a scan's points lie on surfaces.
const TARGET: f64 = 4096.0;
/// Points read at a time.
const CHUNK: u64 = 1 << 20;
/// A cell's place across, up and along is kept in 21 bits each, from this.
const BIAS: i64 = 1 << 20;
const MOST: i64 = (1 << 21) - 1;
/// A block is 8 cells each way.
const BLOCK_BITS: u32 = 3;

/// How far a build has come, 0 to 1 (millionths), read from another thread.
#[derive(Default, Debug)]
pub struct Progress(AtomicU64);

impl Progress {
    pub fn set(&self, f: f64) {
        let v = (f.clamp(0.0, 1.0) * 1e6) as u64;
        self.0.store(v, Ordering::Relaxed);
    }
    pub fn get(&self) -> f64 {
        self.0.load(Ordering::Relaxed) as f64 / 1e6
    }
}

#[derive(Debug)]
pub enum IndexError {
    Io(std::io::Error),
    Las(String),
    Bad(String),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "{e}"),
            IndexError::Las(e) => write!(f, "{e}"),
            IndexError::Bad(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e)
    }
}

impl From<las::Error> for IndexError {
    fn from(e: las::Error) -> Self {
        IndexError::Las(e.to_string())
    }
}

/// A cell: its points' first place, how many, and the box they take (the file's whole numbers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellEntry {
    pub start: u64,
    pub count: u32,
    pub min: [i32; 3],
    pub max: [i32; 3],
}

/// A block: its first cell, how many cells, and the box their points take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockEntry {
    pub first: u64,
    pub cells: u32,
    pub points: u64,
    pub min: [i32; 3],
    pub max: [i32; 3],
}

/// A point as kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointRec {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub intensity: u16,
    pub class: u8,
    pub flags: u8,
    pub returns: u8,
    pub rgb: [u16; 3],
}

/// A scan's index, open (read-only, mapped).
pub struct Index {
    map: Mmap,
    pub path: PathBuf,
    pub count: u64,
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    pub min: [i32; 3],
    pub max: [i32; 3],
    pub has_rgb: bool,
    pub source_size: u64,
    pub source_mtime: u64,
    pub cells: usize,
    pub blocks: usize,
    cells_at: usize,
    blocks_at: usize,
    points_at: usize,
    /// Made now (else opened as it was made before).
    pub built: bool,
}

#[inline]
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
#[inline]
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
#[inline]
fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
#[inline]
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
#[inline]
fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

impl Index {
    /// An index file made before, if it's whole and of this version.
    pub fn open(path: &Path) -> Result<Index, IndexError> {
        let file = File::open(path)?;
        // Safety: the file is ours, written whole before it was renamed in
        // place, and never written again.
        let map = unsafe { Mmap::map(&file)? };
        if map.len() < HEAD || &map[0..8] != MAGIC {
            return Err(IndexError::Bad("not an index".into()));
        }
        if u32_at(&map, 8) != VERSION {
            return Err(IndexError::Bad("an index of another version".into()));
        }
        if map[200] != 1 {
            return Err(IndexError::Bad("an index not finished".into()));
        }
        let count = u64_at(&map, 32);
        let cells = u64_at(&map, 120) as usize;
        let blocks = u64_at(&map, 128) as usize;
        let cells_at = u64_at(&map, 136) as usize;
        let blocks_at = u64_at(&map, 144) as usize;
        let points_at = u64_at(&map, 152) as usize;
        let want = points_at as u128 + count as u128 * RECORD as u128;
        if cells_at != HEAD
            || blocks_at != cells_at + cells * CELL
            || points_at != blocks_at + blocks * BLOCK
            || (map.len() as u128) < want
        {
            return Err(IndexError::Bad("an index cut short".into()));
        }
        let v3 = |at: usize| [f64_at(&map, at), f64_at(&map, at + 8), f64_at(&map, at + 16)];
        let i3 = |at: usize| [i32_at(&map, at), i32_at(&map, at + 4), i32_at(&map, at + 8)];
        Ok(Index {
            path: path.to_path_buf(),
            count,
            scale: v3(40),
            offset: v3(64),
            min: i3(160),
            max: i3(172),
            has_rgb: u32_at(&map, 12) & 1 == 1,
            source_size: u64_at(&map, 16),
            source_mtime: u64_at(&map, 24),
            cells,
            blocks,
            cells_at,
            blocks_at,
            points_at,
            built: false,
            map,
        })
    }

    pub fn cell(&self, k: usize) -> CellEntry {
        let at = self.cells_at + k * CELL;
        let b = &self.map;
        CellEntry {
            start: u64_at(b, at),
            count: u32_at(b, at + 8),
            min: [i32_at(b, at + 16), i32_at(b, at + 20), i32_at(b, at + 24)],
            max: [i32_at(b, at + 28), i32_at(b, at + 32), i32_at(b, at + 36)],
        }
    }

    pub fn block(&self, k: usize) -> BlockEntry {
        let at = self.blocks_at + k * BLOCK;
        let b = &self.map;
        BlockEntry {
            first: u64_at(b, at),
            cells: u32_at(b, at + 8),
            points: u64_at(b, at + 16),
            min: [i32_at(b, at + 24), i32_at(b, at + 28), i32_at(b, at + 32)],
            max: [i32_at(b, at + 36), i32_at(b, at + 40), i32_at(b, at + 44)],
        }
    }

    #[inline]
    pub fn point(&self, i: u64) -> PointRec {
        let at = self.points_at + i as usize * RECORD;
        let b = &self.map[at..at + RECORD];
        PointRec {
            x: i32_at(b, 0),
            y: i32_at(b, 4),
            z: i32_at(b, 8),
            intensity: u16_at(b, 12),
            class: b[14],
            flags: b[15],
            returns: b[16],
            rgb: [u16_at(b, 18), u16_at(b, 20), u16_at(b, 22)],
        }
    }

    /// A point's place in the file's own coordinates.
    #[inline]
    pub fn file_xyz(&self, x: i32, y: i32, z: i32) -> [f64; 3] {
        [
            x as f64 * self.scale[0] + self.offset[0],
            y as f64 * self.scale[1] + self.offset[1],
            z as f64 * self.scale[2] + self.offset[2],
        ]
    }
}

/// The index file's name for a scan file: by its path, size and date.
pub fn cache_name(src: &Path, size: u64, mtime: u64) -> String {
    // FNV-1a, 64 bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(src.to_string_lossy().to_lowercase().as_bytes());
    eat(&size.to_le_bytes());
    eat(&mtime.to_le_bytes());
    format!("{h:016x}.cpe")
}

fn stamp_of(src: &Path) -> Result<(u64, u64), IndexError> {
    let meta = fs::metadata(src)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok((meta.len(), mtime))
}

/// A scan's index: the one made before for this file as it is now, else made now.
pub fn build_or_open(src: &Path, cache_dir: &Path, progress: &Progress) -> Result<Index, IndexError> {
    let (size, mtime) = stamp_of(src)?;
    let path = cache_dir.join(cache_name(src, size, mtime));
    if let Ok(ix) = Index::open(&path) {
        if ix.source_size == size && ix.source_mtime == mtime {
            progress.set(1.0);
            return Ok(ix);
        }
    }
    fs::create_dir_all(cache_dir)?;
    build(src, &path, size, mtime, progress)?;
    let mut ix = Index::open(&path)?;
    ix.built = true;
    progress.set(1.0);
    Ok(ix)
}

#[derive(Clone, Copy)]
struct Acc {
    count: u64,
    min: [i32; 3],
    max: [i32; 3],
}

#[inline]
fn cell_of(p: [i32; 3], origin: [i64; 3], size: [i64; 3]) -> [i64; 3] {
    let mut c = [0i64; 3];
    for a in 0..3 {
        c[a] = ((p[a] as i64 - origin[a]).div_euclid(size[a]) + BIAS).clamp(0, MOST);
    }
    c
}

#[inline]
fn key_of(c: [i64; 3]) -> u64 {
    (c[0] as u64) | (c[1] as u64) << 21 | (c[2] as u64) << 42
}

/// Cells in order of their block, then within it.
#[inline]
fn order_of(key: u64) -> u64 {
    let m = (1u64 << 21) - 1;
    let (x, y, z) = (key & m, (key >> 21) & m, (key >> 42) & m);
    let block = (x >> BLOCK_BITS) | (y >> BLOCK_BITS) << 18 | (z >> BLOCK_BITS) << 36;
    let inner = (x & 7) | (y & 7) << 3 | (z & 7) << 6;
    block << 9 | inner
}

fn encode(out: &mut [u8], p: &PointRec) {
    out[0..4].copy_from_slice(&p.x.to_le_bytes());
    out[4..8].copy_from_slice(&p.y.to_le_bytes());
    out[8..12].copy_from_slice(&p.z.to_le_bytes());
    out[12..14].copy_from_slice(&p.intensity.to_le_bytes());
    out[14] = p.class;
    out[15] = p.flags;
    out[16] = p.returns;
    out[17] = 0;
    out[18..20].copy_from_slice(&p.rgb[0].to_le_bytes());
    out[20..22].copy_from_slice(&p.rgb[1].to_le_bytes());
    out[22..24].copy_from_slice(&p.rgb[2].to_le_bytes());
}

fn decode(b: &[u8]) -> PointRec {
    PointRec {
        x: i32_at(b, 0),
        y: i32_at(b, 4),
        z: i32_at(b, 8),
        intensity: u16_at(b, 12),
        class: b[14],
        flags: b[15],
        returns: b[16],
        rgb: [u16_at(b, 18), u16_at(b, 20), u16_at(b, 22)],
    }
}

fn whole(v: f64, offset: f64, scale: f64) -> i32 {
    let w = ((v - offset) / scale).round();
    w.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// The scan file's points into a new index file at `path`.
pub fn build(src: &Path, path: &Path, size: u64, mtime: u64, progress: &Progress) -> Result<(), IndexError> {
    let mut reader = las::Reader::from_path(src)?;
    let header = reader.header().clone();
    let t = header.transforms();
    let scale = [t.x.scale, t.y.scale, t.z.scale];
    let offset = [t.x.offset, t.y.offset, t.z.offset];
    if scale.iter().any(|s| !(*s > 0.0) || !s.is_finite()) {
        return Err(IndexError::Bad("the file's coordinate scale isn't usable".into()));
    }
    let bounds = header.bounds();
    let expect = header.number_of_points().max(1);
    let has_rgb = header.point_format().has_color;
    let lo = [bounds.min.x, bounds.min.y, bounds.min.z];
    let hi = [bounds.max.x, bounds.max.y, bounds.max.z];
    let mut origin = [0i64; 3];
    let mut extent = [1i64; 3];
    for a in 0..3 {
        let l = if lo[a].is_finite() { lo[a] } else { offset[a] };
        let h = if hi[a].is_finite() && hi[a] >= l { hi[a] } else { l };
        origin[a] = whole(l, offset[a], scale[a]) as i64;
        extent[a] = (whole(h, offset[a], scale[a]) as i64 - origin[a]).max(1);
    }
    // Cells of about TARGET points, the points taken as lying on surfaces.
    let area = ((hi[0] - lo[0]) * (hi[1] - lo[1])).abs();
    let area = if area.is_finite() && area > 0.0 { area } else { 1.0 };
    let side = (area * TARGET / expect as f64).sqrt();
    let mut cell = [1i64; 3];
    for a in 0..3 {
        let c = (side / scale[a]).ceil();
        let c = if c.is_finite() && c >= 1.0 { c as i64 } else { 1 };
        // At most a million cells each way.
        cell[a] = c.max(extent[a] / 1_000_000 + 1);
    }

    // Pass 1: each point counted to its cell and written to the scratch file.
    let scratch = path.with_extension("scratch");
    let mut cells: FxMap<u64, Acc> = FxMap::default();
    let mut total: u64 = 0;
    {
        let mut out = BufWriter::with_capacity(8 << 20, File::create(&scratch)?);
        let mut pts: Vec<las::Point> = Vec::with_capacity(CHUNK as usize);
        let mut rec = [0u8; RECORD];
        loop {
            pts.clear();
            let got = reader.read_points_into(CHUNK, &mut pts)?;
            if got == 0 {
                break;
            }
            for p in &pts {
                let r = PointRec {
                    x: whole(p.x, offset[0], scale[0]),
                    y: whole(p.y, offset[1], scale[1]),
                    z: whole(p.z, offset[2], scale[2]),
                    intensity: p.intensity,
                    class: u8::from(p.classification),
                    flags: (p.is_synthetic as u8)
                        | (p.is_key_point as u8) << 1
                        | (p.is_withheld as u8) << 2
                        | (p.is_overlap as u8) << 3,
                    returns: (p.return_number & 15) | (p.number_of_returns & 15) << 4,
                    rgb: p.color.map(|c| [c.red, c.green, c.blue]).unwrap_or([0, 0, 0]),
                };
                let key = key_of(cell_of([r.x, r.y, r.z], origin, cell));
                let acc = cells.entry(key).or_insert(Acc {
                    count: 0,
                    min: [i32::MAX; 3],
                    max: [i32::MIN; 3],
                });
                acc.count += 1;
                for (a, v) in [r.x, r.y, r.z].into_iter().enumerate() {
                    acc.min[a] = acc.min[a].min(v);
                    acc.max[a] = acc.max[a].max(v);
                }
                encode(&mut rec, &r);
                out.write_all(&rec)?;
            }
            total += got;
            progress.set(0.6 * (total as f64 / expect as f64).min(1.0));
        }
        out.flush()?;
    }

    // The cells in order, each its place; the blocks over them.
    let mut order: Vec<(u64, u64, Acc)> = cells.iter().map(|(k, a)| (order_of(*k), *k, *a)).collect();
    order.sort_unstable_by_key(|o| o.0);
    let mut slot_of: FxMap<u64, u32> = FxMap::default();
    slot_of.reserve(order.len());
    let mut starts: Vec<u64> = Vec::with_capacity(order.len());
    let mut run = 0u64;
    let mut blocks: Vec<BlockEntry> = Vec::new();
    let (mut lo_all, mut hi_all) = ([i32::MAX; 3], [i32::MIN; 3]);
    for (k, (o, key, acc)) in order.iter().enumerate() {
        slot_of.insert(*key, k as u32);
        starts.push(run);
        run += acc.count;
        let b = o >> 9;
        let new_block = match blocks.last() {
            None => true,
            Some(_) => order[k - 1].0 >> 9 != b,
        };
        if new_block {
            blocks.push(BlockEntry {
                first: k as u64,
                cells: 0,
                points: 0,
                min: [i32::MAX; 3],
                max: [i32::MIN; 3],
            });
        }
        let last = blocks.last_mut().unwrap();
        last.cells += 1;
        last.points += acc.count;
        for a in 0..3 {
            last.min[a] = last.min[a].min(acc.min[a]);
            last.max[a] = last.max[a].max(acc.max[a]);
            lo_all[a] = lo_all[a].min(acc.min[a]);
            hi_all[a] = hi_all[a].max(acc.max[a]);
        }
    }
    if total == 0 {
        lo_all = [0; 3];
        hi_all = [0; 3];
    }

    // The index file, its head and tables written now, its "whole" flag last.
    let cells_at = HEAD;
    let blocks_at = cells_at + order.len() * CELL;
    let points_at = blocks_at + blocks.len() * BLOCK;
    let length = points_at as u64 + total * RECORD as u64;
    let building = path.with_extension("building");
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&building)?;
    file.set_len(length.max(HEAD as u64))?;
    // Safety: a file of our own, just made and sized, mapped while we write it.
    let mut map = unsafe { MmapMut::map_mut(&file)? };
    {
        let h = &mut map[0..HEAD];
        h.fill(0);
        h[0..8].copy_from_slice(MAGIC);
        h[8..12].copy_from_slice(&VERSION.to_le_bytes());
        h[12..16].copy_from_slice(&(has_rgb as u32).to_le_bytes());
        h[16..24].copy_from_slice(&size.to_le_bytes());
        h[24..32].copy_from_slice(&mtime.to_le_bytes());
        h[32..40].copy_from_slice(&total.to_le_bytes());
        for a in 0..3 {
            h[40 + a * 8..48 + a * 8].copy_from_slice(&scale[a].to_le_bytes());
            h[64 + a * 8..72 + a * 8].copy_from_slice(&offset[a].to_le_bytes());
            h[88 + a * 8..96 + a * 8].copy_from_slice(&cell[a].to_le_bytes());
            h[96 + a * 8..104 + a * 8].copy_from_slice(&origin[a].to_le_bytes());
            h[160 + a * 4..164 + a * 4].copy_from_slice(&lo_all[a].to_le_bytes());
            h[172 + a * 4..176 + a * 4].copy_from_slice(&hi_all[a].to_le_bytes());
        }
        h[120..128].copy_from_slice(&(order.len() as u64).to_le_bytes());
        h[128..136].copy_from_slice(&(blocks.len() as u64).to_le_bytes());
        h[136..144].copy_from_slice(&(cells_at as u64).to_le_bytes());
        h[144..152].copy_from_slice(&(blocks_at as u64).to_le_bytes());
        h[152..160].copy_from_slice(&(points_at as u64).to_le_bytes());
        h[184] = header.point_format().to_u8().unwrap_or(0);
    }
    for (k, (_, _, acc)) in order.iter().enumerate() {
        let at = cells_at + k * CELL;
        let c = &mut map[at..at + CELL];
        c[0..8].copy_from_slice(&starts[k].to_le_bytes());
        c[8..12].copy_from_slice(&(acc.count as u32).to_le_bytes());
        c[12..16].fill(0);
        for a in 0..3 {
            c[16 + a * 4..20 + a * 4].copy_from_slice(&acc.min[a].to_le_bytes());
            c[28 + a * 4..32 + a * 4].copy_from_slice(&acc.max[a].to_le_bytes());
        }
    }
    for (k, b) in blocks.iter().enumerate() {
        let at = blocks_at + k * BLOCK;
        let c = &mut map[at..at + BLOCK];
        c[0..8].copy_from_slice(&b.first.to_le_bytes());
        c[8..12].copy_from_slice(&b.cells.to_le_bytes());
        c[12..16].fill(0);
        c[16..24].copy_from_slice(&b.points.to_le_bytes());
        for a in 0..3 {
            c[24 + a * 4..28 + a * 4].copy_from_slice(&b.min[a].to_le_bytes());
            c[36 + a * 4..40 + a * 4].copy_from_slice(&b.max[a].to_le_bytes());
        }
    }

    // Pass 2: the scratch read back in order, each point to its cell's place.
    {
        let mut fill: Vec<u64> = vec![0; order.len()];
        let mut input = BufReader::with_capacity(8 << 20, File::open(&scratch)?);
        let mut buf = vec![0u8; CHUNK as usize * RECORD];
        let mut done: u64 = 0;
        while done < total {
            let n = ((total - done).min(CHUNK)) as usize;
            let bytes = &mut buf[..n * RECORD];
            input.read_exact(bytes)?;
            for r in bytes.chunks_exact(RECORD) {
                let p = decode(r);
                let key = key_of(cell_of([p.x, p.y, p.z], origin, cell));
                let k = slot_of[&key] as usize;
                let at = points_at + (starts[k] + fill[k]) as usize * RECORD;
                fill[k] += 1;
                map[at..at + RECORD].copy_from_slice(r);
            }
            done += n as u64;
            progress.set(0.6 + 0.4 * (done as f64 / total.max(1) as f64));
        }
    }
    map.flush()?;
    map[200] = 1;
    map.flush_range(0, HEAD)?;
    drop(map);
    drop(file);
    let _ = fs::remove_file(&scratch);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&building, path)?;
    Ok(())
}
