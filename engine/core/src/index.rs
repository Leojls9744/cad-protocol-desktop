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
//!   byte (1.24: an E57 point's setup, its low 8 bits; the high 4 in the
//!   flags' high nibble), then red, green and blue (u16 each). Within a cell the points
//!   are shuffled (1.20), so any first part of a cell is an even sample of
//!   it: a view that wants fewer takes the same first share of each cell.
//!
//! Made in two passes: the file read once (LAZ on every core), each point
//! counted to its cell and written in turn to a scratch file; then the
//! scratch read back in order and each point put in its cell's place. An
//! E57 file (1.24, e57.rs) has no bounds ahead, so its scratch is read once
//! more between, to count its cells. The head's flags: colour 1, 16-bit
//! colour 2, setups 4 (an E57 of several), E57 8.

use crate::fx::FxMap;
use memmap2::{Mmap, MmapMut};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MAGIC: &[u8; 8] = b"CADPENG1";
/// 2 (desktop 1.20): each cell's points in a shuffled order, so the first
/// part of any cell is an even sample of it; whether colours are 16-bit.
pub const VERSION: u32 = 2;
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
    /// Synthetic 1, key-point 2, withheld 4, overlap 8.
    pub flags: u8,
    pub returns: u8,
    pub rgb: [u16; 3],
    /// An E57 point's setup (its scan's place in the file, 1.24); 0 for LAS.
    pub setup: u16,
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
    /// Colours of 16 bits (any channel past 255): read as bytes by their high byte.
    pub wide: bool,
    /// Each point's setup is kept (an E57 of several scans, 1.24).
    pub setups: bool,
    /// Made from an E57 file (1.24).
    pub e57: bool,
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
            wide: u32_at(&map, 12) & 2 == 2,
            setups: u32_at(&map, 12) & 4 == 4,
            e57: u32_at(&map, 12) & 8 == 8,
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
        decode(&self.map[at..at + RECORD])
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

/// FNV-1a, 64 bits, of each run of bytes in turn.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for bytes in parts {
        for b in *bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// A scan file's path as its indexes' names start (1.25).
fn path_key(src: &Path) -> String {
    format!("{:016x}-", fnv(&[src.to_string_lossy().to_lowercase().as_bytes()]))
}

/// The index file's name for a scan file (1.25): by its path, then its size
/// and date, so a file's older indexes are known by the first part.
pub fn cache_name(src: &Path, size: u64, mtime: u64) -> String {
    format!("{}{:016x}.cpe", path_key(src), fnv(&[&size.to_le_bytes(), &mtime.to_le_bytes()]))
}

/// The name 1.19–1.24 gave it: path, size and date in one.
fn old_cache_name(src: &Path, size: u64, mtime: u64) -> String {
    let h = fnv(&[src.to_string_lossy().to_lowercase().as_bytes(), &size.to_le_bytes(), &mtime.to_le_bytes()]);
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

/// Half-made files (a build stopped short) are let go after this long.
const STALE: Duration = Duration::from_secs(24 * 60 * 60);

/// The folder's half-made files of a day or more ago removed, once a run (1.25).
fn tidy_folder_once(cache_dir: &Path) {
    static DONE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    {
        let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
        if done.iter().any(|d| d == cache_dir) {
            return;
        }
        done.push(cache_dir.to_path_buf());
    }
    let Ok(list) = fs::read_dir(cache_dir) else { return };
    let now = SystemTime::now();
    for entry in list.flatten() {
        let p = entry.path();
        let half = matches!(p.extension().and_then(|e| e.to_str()), Some("scratch" | "building"));
        let old = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= STALE);
        if half && old {
            let _ = fs::remove_file(&p);
        }
    }
}

/// The scan file's other indexes (made of it before it changed) and half-made files removed, all but `keep` (1.25).
fn tidy_others(cache_dir: &Path, src: &Path, keep: &Path) {
    let Ok(list) = fs::read_dir(cache_dir) else { return };
    let mine = path_key(src);
    for entry in list.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = matches!(p.extension().and_then(|e| e.to_str()), Some("cpe" | "scratch" | "building"));
        if ours && name.starts_with(&mine) && p != keep {
            let _ = fs::remove_file(&p);
        }
    }
}

/// The index made before for this file as it is now, if there is one
/// (1.25: one 1.19–1.24 made taken on under its new name).
pub fn open_made(src: &Path, cache_dir: &Path) -> Option<Index> {
    tidy_folder_once(cache_dir);
    let (size, mtime) = stamp_of(src).ok()?;
    let fits = |ix: &Index| ix.source_size == size && ix.source_mtime == mtime;
    let path = cache_dir.join(cache_name(src, size, mtime));
    if let Some(ix) = Index::open(&path).ok().filter(fits) {
        return Some(ix);
    }
    let old = cache_dir.join(old_cache_name(src, size, mtime));
    let ix = Index::open(&old).ok().filter(fits)?;
    drop(ix);
    if fs::rename(&old, &path).is_ok() {
        tidy_others(cache_dir, src, &path);
        return Index::open(&path).ok().filter(fits);
    }
    Index::open(&old).ok().filter(fits)
}

/// The scan's index made now; its older ones removed once it's whole (1.25).
pub fn build_fresh(src: &Path, cache_dir: &Path, progress: &Progress) -> Result<Index, IndexError> {
    let (size, mtime) = stamp_of(src)?;
    fs::create_dir_all(cache_dir)?;
    let path = cache_dir.join(cache_name(src, size, mtime));
    if let Err(e) = build(src, &path, size, mtime, progress) {
        let _ = fs::remove_file(path.with_extension("scratch"));
        let _ = fs::remove_file(path.with_extension("building"));
        return Err(e);
    }
    let mut ix = Index::open(&path)?;
    ix.built = true;
    tidy_others(cache_dir, src, &path);
    progress.set(1.0);
    Ok(ix)
}

/// A scan's index: the one made before for this file as it is now, else made now.
pub fn build_or_open(src: &Path, cache_dir: &Path, progress: &Progress) -> Result<Index, IndexError> {
    if let Some(ix) = open_made(src, cache_dir) {
        progress.set(1.0);
        return Ok(ix);
    }
    build_fresh(src, cache_dir, progress)
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
    out[15] = (p.flags & 15) | (((p.setup >> 8) & 15) as u8) << 4;
    out[16] = p.returns;
    out[17] = (p.setup & 255) as u8;
    out[18..20].copy_from_slice(&p.rgb[0].to_le_bytes());
    out[20..22].copy_from_slice(&p.rgb[1].to_le_bytes());
    out[22..24].copy_from_slice(&p.rgb[2].to_le_bytes());
}

#[inline]
fn decode(b: &[u8]) -> PointRec {
    PointRec {
        x: i32_at(b, 0),
        y: i32_at(b, 4),
        z: i32_at(b, 8),
        intensity: u16_at(b, 12),
        class: b[14],
        flags: b[15] & 15,
        returns: b[16],
        rgb: [u16_at(b, 18), u16_at(b, 20), u16_at(b, 22)],
        setup: b[17] as u16 | ((b[15] >> 4) as u16) << 8,
    }
}

fn whole(v: f64, offset: f64, scale: f64) -> i32 {
    let w = ((v - offset) / scale).round();
    w.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// The scan file's points into a new index file at `path` (an E57 file by its name, else LAS or LAZ).
pub fn build(src: &Path, path: &Path, size: u64, mtime: u64, progress: &Progress) -> Result<(), IndexError> {
    let e57 = src.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case("e57")).unwrap_or(false);
    if e57 {
        return build_e57(src, path, size, mtime, progress);
    }
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
    // Cells of about TARGET points, the points taken as lying on surfaces.
    let (origin, cell) = cells_for(lo, hi, expect, offset, scale);

    // Pass 1: each point counted to its cell and written to the scratch file.
    let scratch = path.with_extension("scratch");
    let mut cells: FxMap<u64, Acc> = FxMap::default();
    let mut total: u64 = 0;
    let mut wide = false;
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
                    setup: 0,
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
                if r.rgb[0] > 255 || r.rgb[1] > 255 || r.rgb[2] > 255 {
                    wide = true;
                }
                encode(&mut rec, &r);
                out.write_all(&rec)?;
            }
            total += got;
            progress.set(0.6 * (total as f64 / expect as f64).min(1.0));
        }
        out.flush()?;
    }
    let head = Head {
        scale,
        offset,
        origin,
        cell,
        flags: (has_rgb as u32) | (wide as u32) << 1,
        size,
        mtime,
    };
    lay_out(path, &scratch, total, &cells, &head, progress)
}

/// What the index's head says of the scan, and how its cells are laid.
struct Head {
    scale: [f64; 3],
    offset: [f64; 3],
    origin: [i64; 3],
    cell: [i64; 3],
    flags: u32,
    size: u64,
    mtime: u64,
}

/// The cells' sizes for points from `lo` to `hi` (the file's whole numbers), about `expect` of them.
fn cells_for(lo: [f64; 3], hi: [f64; 3], expect: u64, offset: [f64; 3], scale: [f64; 3]) -> ([i64; 3], [i64; 3]) {
    let mut origin = [0i64; 3];
    let mut extent = [1i64; 3];
    for a in 0..3 {
        let l = if lo[a].is_finite() { lo[a] } else { offset[a] };
        let h = if hi[a].is_finite() && hi[a] >= l { hi[a] } else { l };
        origin[a] = whole(l, offset[a], scale[a]) as i64;
        extent[a] = (whole(h, offset[a], scale[a]) as i64 - origin[a]).max(1);
    }
    let area = ((hi[0] - lo[0]) * (hi[1] - lo[1])).abs();
    let area = if area.is_finite() && area > 0.0 { area } else { 1.0 };
    let side = (area * TARGET / expect.max(1) as f64).sqrt();
    let mut cell = [1i64; 3];
    for a in 0..3 {
        let c = (side / scale[a]).ceil();
        let c = if c.is_finite() && c >= 1.0 { c as i64 } else { 1 };
        cell[a] = c.max(extent[a] / 1_000_000 + 1);
    }
    (origin, cell)
}

/// The E57 grid: a tenth of a millimetre.
const E57_SCALE: f64 = 0.0001;

/// An E57 file's points into a new index file at `path` (1.24): read once
/// to the scratch (their bounds found as they come), the scratch read once
/// to count each cell, then laid out as a LAS file's.
fn build_e57(src: &Path, path: &Path, size: u64, mtime: u64, progress: &Progress) -> Result<(), IndexError> {
    let scratch = path.with_extension("scratch");
    let scale = [E57_SCALE; 3];
    let mut offset: Option<[f64; 3]> = None;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut total: u64 = 0;
    let facts;
    {
        let mut out = BufWriter::with_capacity(8 << 20, File::create(&scratch)?);
        let mut rec = [0u8; RECORD];
        let mut failed: Option<std::io::Error> = None;
        facts = crate::e57::read(
            src,
            |p| {
                if failed.is_some() {
                    return false;
                }
                // The grid from the first point, in whole metres: ±214 km round it.
                let o = *offset.get_or_insert([p.x.round(), p.y.round(), p.z.round()]);
                let r = PointRec {
                    x: whole(p.x, o[0], scale[0]),
                    y: whole(p.y, o[1], scale[1]),
                    z: whole(p.z, o[2], scale[2]),
                    intensity: p.intensity,
                    class: 1,
                    flags: 0,
                    returns: 0,
                    rgb: [p.rgb[0] as u16, p.rgb[1] as u16, p.rgb[2] as u16],
                    setup: p.setup,
                };
                for (a, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                    lo[a] = lo[a].min(v);
                    hi[a] = hi[a].max(v);
                }
                encode(&mut rec, &r);
                if let Err(e) = out.write_all(&rec) {
                    // No room: the rest of the file isn't read (1.25).
                    failed = Some(e);
                    return false;
                }
                total += 1;
                true
            },
            |done, all| progress.set(0.5 * (done as f64 / all.max(1) as f64).min(1.0)),
        );
        if let Some(e) = failed {
            let _ = fs::remove_file(&scratch);
            return Err(e.into());
        }
        out.flush()?;
    }
    let facts = match facts {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_file(&scratch);
            return Err(IndexError::Bad(e.to_string()));
        }
    };
    if total == 0 {
        let _ = fs::remove_file(&scratch);
        return Err(IndexError::Bad("The E57 scan has no valid points.".into()));
    }
    let offset = offset.unwrap_or([0.0; 3]);
    let (origin, cell) = cells_for(lo, hi, total, offset, scale);
    // Each cell counted from the scratch.
    let mut cells: FxMap<u64, Acc> = FxMap::default();
    {
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
                let acc = cells.entry(key).or_insert(Acc { count: 0, min: [i32::MAX; 3], max: [i32::MIN; 3] });
                acc.count += 1;
                for (a, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                    acc.min[a] = acc.min[a].min(v);
                    acc.max[a] = acc.max[a].max(v);
                }
            }
            done += n as u64;
            progress.set(0.5 + 0.1 * (done as f64 / total as f64));
        }
    }
    let head = Head {
        scale,
        offset,
        origin,
        cell,
        flags: (facts.colour as u32) | ((facts.scans > 1) as u32) << 2 | 1 << 3,
        size,
        mtime,
    };
    lay_out(path, &scratch, total, &cells, &head, progress)
}

/// The index file laid out from the scratch: its head and tables, each point in its cell's place, each cell shuffled.
fn lay_out(path: &Path, scratch: &Path, total: u64, cells: &FxMap<u64, Acc>, head: &Head, progress: &Progress) -> Result<(), IndexError> {
    let Head { scale, offset, origin, cell, flags, size, mtime } = *head;

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
        h[12..16].copy_from_slice(&flags.to_le_bytes());
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
        let mut input = BufReader::with_capacity(8 << 20, File::open(scratch)?);
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
    // Each cell's points shuffled (the same way every time), so its first
    // part is an even sample of it.
    for (k, (_, _, acc)) in order.iter().enumerate() {
        let n = acc.count as usize;
        if n < 2 {
            continue;
        }
        let base = points_at + starts[k] as usize * RECORD;
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15 ^ (k as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let mut tmp = [0u8; RECORD];
        for i in (1..n).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let j = (seed % (i as u64 + 1)) as usize;
            if i != j {
                let (a, b) = (base + i * RECORD, base + j * RECORD);
                tmp.copy_from_slice(&map[a..a + RECORD]);
                map.copy_within(b..b + RECORD, a);
                map[b..b + RECORD].copy_from_slice(&tmp);
            }
        }
    }
    map.flush()?;
    map[200] = 1;
    map.flush_range(0, HEAD)?;
    drop(map);
    drop(file);
    let _ = fs::remove_file(scratch);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&building, path)?;
    Ok(())
}
