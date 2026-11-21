//! What the engine draws: points, each a place and what its colour is made
//! from, sampled from a scan's index for the view, or made up for the check.

use crate::camera::{Affine, Camera};
use crate::shapes::Shapes;
use crate::surface;
use bytemuck::{Pod, Zeroable};
use cadp_engine_core::crop::{keeps as boxes_keep, CropBox};
use cadp_engine_core::Index;
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/// A point on the card: its place from its scan's middle (the file's own
/// units, as f32 near 0 so nothing's lost), its own colour, its class (as
/// the drawing has it), its intensity, its setup and its shading from the
/// shape of the surface it's on (surface.rs: light, relief, edge, sky).
/// The card colours it by whichever the view asks for (gpu.rs), so a
/// change of colouring, of a class's layer or of a shading's strength
/// sends no points again.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PointVertex {
    pub pos: [f32; 3],
    pub rgb: [u8; 3],
    pub class: u8,
    pub intensity: u16,
    pub setup: u16,
    /// Its surface's light (0–255), its relief (−127 to 127, as a byte), its edge (0–255) and the sky it sees (255: all).
    pub shade: [u8; 4],
    /// Its spacing where it is, for Fill gaps (surface.rs gap_byte: 0 none known), and three bytes to spare.
    pub more: [u8; 4],
}

/// Points to draw, from `centre` (the file's coordinates).
#[derive(Clone, Debug, Default)]
pub struct Points {
    pub centre: [f64; 3],
    /// How far any of them is from the middle, at most.
    pub radius: f64,
    /// They have colours of their own (else by height where their own are asked for).
    pub colours: bool,
    /// Their shading's worked out (else each is `surface::plain()`, and drawn unshaded).
    pub shaded: bool,
    /// How long that took, ms.
    pub shade_ms: f64,
    /// Their average spacing in the drawing, which each one's own is told by (`more[0]`); 0: not measured.
    pub gap: f64,
    pub data: Vec<PointVertex>,
}

/// A hash of a cell's number as it is (already well spread by a multiply).
#[derive(Default)]
pub struct CellHasher(u64);

impl Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ *b as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    }
}

pub(crate) type CellMap = HashMap<u64, u32, BuildHasherDefault<CellHasher>>;

/// Cells either way of the middle a place can be filed in (2^20).
const HALF: i64 = 1 << 20;

/// The page's own points of a scan (the ones it keeps and has classified,
/// painted or taken out), filed by where they are in the drawing: the
/// class a point of the scan's file takes is its nearest's, and it's left
/// out where that was taken out, as the page gives its own close-ups
/// their classes (cloud-detail.ts borrowNearestSteps). Filed once for the
/// points; their classes and what's taken out can be given again.
pub struct BorrowPoints {
    pub key: String,
    cell: f64,
    origin: [f64; 3],
    xyz: Vec<f32>,
    grid: CellMap,
}

impl BorrowPoints {
    /// `xyz`: each point's place from `origin`, in the drawing; `cell`: the cells' size (twice the points' spacing, as the page files them).
    pub fn new(key: String, cell: f64, origin: [f64; 3], xyz: Vec<f32>) -> BorrowPoints {
        let cell = if cell > 0.0 && cell.is_finite() { cell } else { 1.0 };
        let n = xyz.len() / 3;
        let mut grid = CellMap::with_capacity_and_hasher(n / 3 + 16, Default::default());
        for i in 0..n {
            let at = [xyz[i * 3] as f64, xyz[i * 3 + 1] as f64, xyz[i * 3 + 2] as f64];
            if let Some(k) = cell_of(at, cell) {
                // The last in a cell is its, as the page has it.
                grid.insert(k, i as u32);
            }
        }
        BorrowPoints { key, cell, origin, xyz, grid }
    }

    pub fn count(&self) -> usize {
        self.xyz.len() / 3
    }

    /// The page's point nearest a place in the drawing: its cell's, else the nearest of those in the 26 cells round it; none with none there.
    pub fn nearest(&self, p: [f64; 3]) -> Option<u32> {
        let at = [p[0] - self.origin[0], p[1] - self.origin[1], p[2] - self.origin[2]];
        let (ix, iy, iz) = cell_at(at, self.cell)?;
        if let Some(i) = self.grid.get(&pack(ix, iy, iz)) {
            return Some(*i);
        }
        let mut best = None;
        let mut best_d = f64::INFINITY;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let (x, y, z) = (ix + dx, iy + dy, iz + dz);
                    if x < 0 || y < 0 || z < 0 || x >= 2 * HALF || y >= 2 * HALF || z >= 2 * HALF {
                        continue;
                    }
                    if let Some(i) = self.grid.get(&pack(x, y, z)) {
                        let k = *i as usize * 3;
                        let d = (self.xyz[k] as f64 - at[0]).powi(2) + (self.xyz[k + 1] as f64 - at[1]).powi(2) + (self.xyz[k + 2] as f64 - at[2]).powi(2);
                        if d < best_d {
                            best_d = d;
                            best = Some(*i);
                        }
                    }
                }
            }
        }
        best
    }
}

fn cell_at(at: [f64; 3], cell: f64) -> Option<(i64, i64, i64)> {
    let f = |v: f64| {
        let k = (v / cell).floor();
        (k.is_finite() && k >= -(HALF as f64) && k < HALF as f64).then(|| k as i64 + HALF)
    };
    Some((f(at[0])?, f(at[1])?, f(at[2])?))
}

pub(crate) fn pack(x: i64, y: i64, z: i64) -> u64 {
    (x as u64) | (y as u64) << 21 | (z as u64) << 42
}

fn cell_of(at: [f64; 3], cell: f64) -> Option<u64> {
    cell_at(at, cell).map(|(x, y, z)| pack(x, y, z))
}

/// The page's points with their classes and what's taken out, as they are now.
pub struct Borrow {
    pub key: String,
    pub points: Arc<BorrowPoints>,
    class: Vec<u8>,
    removed: Option<Vec<u8>>,
}

impl Borrow {
    /// None when the lists aren't the points' length.
    pub fn new(key: String, points: Arc<BorrowPoints>, class: Vec<u8>, removed: Option<Vec<u8>>) -> Option<Borrow> {
        let n = points.count();
        (class.len() == n && removed.as_ref().is_none_or(|r| r.len() == n)).then_some(Borrow { key, points, class, removed })
    }

    /// The class a place in the drawing takes and whether it's taken out; none with no point of the page's near.
    #[inline]
    pub fn at(&self, p: [f64; 3]) -> Option<(u8, bool)> {
        let i = self.points.nearest(p)? as usize;
        Some((self.class[i], self.removed.as_ref().is_some_and(|r| r[i] != 0)))
    }
}

/// Which of a scan's points are drawn, and with what class: an E57's one
/// setup alone, or all but those turned off; the file's class codes read
/// as the scan's class map has them; and, for a scan the page has
/// classified or edited, the page's own classes and removals (`borrow`).
/// Withheld points never are. `isolate`: only the points the shapes
/// isolated keep (shapes.rs), by where they are in the drawing and their
/// class. `shade`: their shading from the shape of their surface is worked
/// out with them (surface.rs). `key` tells one from another.
#[derive(Clone, Default)]
pub struct Keep {
    pub setup: Option<u16>,
    pub off: Vec<u16>,
    pub map: Option<Arc<[u8; 256]>>,
    pub borrow: Option<Arc<Borrow>>,
    pub isolate: Option<Arc<Shapes>>,
    pub shade: bool,
    /// Each point's spacing is measured with them (the page's Fill gaps).
    pub gaps: bool,
    pub key: String,
}

impl Keep {
    /// Every point (but the withheld), the file's own classes.
    pub fn all() -> Keep {
        Keep::default()
    }

    /// With its key made from what it holds.
    pub fn keyed(mut self) -> Keep {
        let map = self.map.as_ref().map(|m| m.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01B3)));
        self.key = format!(
            "{:?}|{:?}|{:?}|{}|{}{}|{}",
            self.setup,
            self.off,
            map,
            self.borrow.as_ref().map(|b| b.key.as_str()).unwrap_or(""),
            if self.shade { "shaded" } else { "" },
            if self.gaps { "gaps" } else { "" },
            self.isolate.as_ref().map(|s| s.key.as_str()).unwrap_or("")
        );
        self
    }

    #[inline]
    pub fn keeps(&self, flags: u8, setup: u16) -> bool {
        flags & 4 == 0 && self.setup.is_none_or(|only| only == setup) && !self.off.contains(&setup)
    }
}

/// What of the scans a view's limit boxes and its slice leave showing, in
/// the drawing, as the page tests them (cloud-limits.ts cropsKeeper: out of
/// every box that shows what's outside it, and in one of those that show
/// what's inside; and in the slice's slab). A scan's detail for a view is
/// read by it (only what shows takes up the scan's budget of points, so a
/// thin slice comes as dense as its file has it); the card tests each
/// point by it too (gpu.rs), so a box dragged shows at once on the points
/// held. `key` tells one from another.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub boxes: Vec<CropBox>,
    pub slice: Option<CropBox>,
    pub key: String,
}

/// The most limit boxes the card tests (the page's CARD_BOXES).
pub const CLIP_BOXES: usize = 8;

impl Clip {
    /// Nothing clipped.
    pub fn is_none(&self) -> bool {
        self.boxes.is_empty() && self.slice.is_none()
    }

    /// With its key made from what it holds.
    pub fn keyed(mut self) -> Clip {
        let word = |b: &CropBox| format!("{:?}{:?}{:?}{}{}{}{}{:?}", b.min, b.max, b.centre, b.cos, b.sin, b.turned, b.outside, b.slack);
        self.key = self.boxes.iter().map(word).chain(self.slice.iter().map(|s| format!("/{}", word(s)))).collect::<Vec<_>>().join(";");
        self
    }

    /// Whether a place in the drawing shows.
    #[inline]
    pub fn keeps(&self, p: [f64; 3]) -> bool {
        boxes_keep(&self.boxes, p) && self.slice.as_ref().is_none_or(|s| s.holds(p))
    }

    /// Whether a box of the drawing may hold a place that shows.
    pub fn reaches(&self, lo: [f64; 3], hi: [f64; 3]) -> bool {
        if self.slice.as_ref().is_some_and(|s| !s.reaches(lo, hi)) {
            return false;
        }
        let mut inside = self.boxes.iter().filter(|b| !b.outside).peekable();
        inside.peek().is_none() || inside.any(|b| b.reaches(lo, hi))
    }
}

/// A box of a scan's file (`lo` to `hi`) as the box of the drawing that holds it.
pub fn drawn_box(affine: &Affine, lo: [f64; 3], hi: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let half = [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0, (hi[2] - lo[2]) / 2.0];
    let mut out = ([0.0; 3], [0.0; 3]);
    for r in 0..3 {
        let c = affine[r * 4] * mid[0] + affine[r * 4 + 1] * mid[1] + affine[r * 4 + 2] * mid[2] + affine[r * 4 + 3];
        let e = affine[r * 4].abs() * half[0] + affine[r * 4 + 1].abs() * half[1] + affine[r * 4 + 2].abs() * half[2];
        out.0[r] = c - e;
        out.1[r] = c + e;
    }
    out
}

/// What a view sees of a scan: the cells whose boxes reach into the
/// drawing area (a little past its edges, so a small pan needs nothing
/// new), found from each cell's box through the scan's place in the
/// drawing and the view. The page's views are parallel, so nothing is
/// nearer or further: only across and up matter.
pub struct Sight {
    /// Each of the scan's axes across and up the screen, pixels a unit of the file's, and where its origin (0, 0, 0) lands from the drawing area's middle.
    across: [f64; 4],
    up: [f64; 4],
    half: [f64; 2],
}

/// How far past the drawing area's edges cells are taken, as a share of its size.
pub const SIGHT_MARGIN: f64 = 0.15;

impl Sight {
    pub fn new(camera: &Camera, affine: &Affine) -> Sight {
        let upp = if camera.units_per_pixel > 0.0 { camera.units_per_pixel } else { 1.0 };
        let row = |axis: [f64; 3]| {
            let mut out = [0.0; 4];
            for c in 0..3 {
                out[c] = (affine[c] * axis[0] + affine[4 + c] * axis[1] + affine[8 + c] * axis[2]) / upp;
            }
            out[3] = ((affine[3] - camera.target[0]) * axis[0] + (affine[7] - camera.target[1]) * axis[1] + (affine[11] - camera.target[2]) * axis[2]) / upp;
            out
        };
        Sight {
            across: row(camera.right),
            up: row(camera.up),
            half: [camera.rect[2] / 2.0 * (1.0 + 2.0 * SIGHT_MARGIN), camera.rect[3] / 2.0 * (1.0 + 2.0 * SIGHT_MARGIN)],
        }
    }

    /// Whether a box (the file's coordinates) reaches into what's seen.
    #[inline]
    pub fn sees(&self, lo: [f64; 3], hi: [f64; 3]) -> bool {
        let span = |r: &[f64; 4]| {
            let mut a = r[3];
            let mut b = r[3];
            for c in 0..3 {
                let (x, y) = (r[c] * lo[c], r[c] * hi[c]);
                a += x.min(y);
                b += x.max(y);
            }
            (a, b)
        };
        let (x0, x1) = span(&self.across);
        let (y0, y1) = span(&self.up);
        x1 >= -self.half[0] && x0 <= self.half[0] && y1 >= -self.half[1] && y0 <= self.half[1]
    }
}

/// What a sample holds of its scan: which cells (a bit each) and what share of each one's points, and the limit boxes and slice it was read by (their key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Taken {
    pub cells: Vec<u64>,
    pub share: f64,
    /// The points of the cells taken, all told (before the share).
    pub of: u64,
    pub clip: String,
}

impl Taken {
    /// Whether this holds what `want` asks: its cells, at about its share (a little less passes: a zoom of a notch isn't sampled again).
    pub fn covers(&self, want: &Taken) -> bool {
        if self.cells.len() != want.cells.len() || self.clip != want.clip {
            return false;
        }
        if want.cells.iter().zip(&self.cells).any(|(w, h)| w & !h != 0) {
            return false;
        }
        self.share >= 1.0 || self.share * SHARE_SLACK >= want.share
    }
}

/// A sample is kept while what's wanted is under this many times its share.
pub const SHARE_SLACK: f64 = 1.3;

/// How many points are read at most to find `most` kept ones, as a multiple of `most` (a scan with next to none kept isn't read whole for them).
const READ_MOST: f64 = 50.0;
/// How many points the share kept is judged from.
const PROBE: f64 = 400_000.0;

/// The cells a view sees (all with no view) that its limit boxes and slice may show something of, and the share of each that makes about `most` points.
pub fn wanted(ix: &Index, most: usize, sight: Option<&Sight>, clip: Option<(&Clip, &Affine)>) -> Taken {
    let clip = clip.filter(|(c, _)| !c.is_none());
    let mut cells = vec![0u64; ix.cells.div_ceil(64)];
    let mut of = 0u64;
    for k in 0..ix.cells {
        let cell = ix.cell(k);
        if cell.count == 0 {
            continue;
        }
        if let Some(s) = sight {
            let lo = ix.file_xyz(cell.min[0], cell.min[1], cell.min[2]);
            let hi = ix.file_xyz(cell.max[0], cell.max[1], cell.max[2]);
            if !s.sees(lo, hi) {
                continue;
            }
        }
        if let Some((c, affine)) = clip {
            let lo = ix.file_xyz(cell.min[0], cell.min[1], cell.min[2]);
            let hi = ix.file_xyz(cell.max[0], cell.max[1], cell.max[2]);
            let (lo, hi) = drawn_box(affine, lo, hi);
            if !c.reaches(lo, hi) {
                continue;
            }
        }
        cells[k / 64] |= 1 << (k % 64);
        of += cell.count as u64;
    }
    let share = if of == 0 { 0.0 } else { (most as f64 / of as f64).min(1.0) };
    Taken { cells, share, of, clip: clip.map(|(c, _)| c.key.clone()).unwrap_or_default() }
}

/// Blue to yellow-green as `t` goes from 0 to 1: the page's elevation colours (point-cloud.ts ELEVATION_RAMP), for the check's ring.
pub fn ramp(t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let at = t * (RAMP.len() - 1) as f64;
    let i = (at.floor() as usize).min(RAMP.len() - 2);
    let f = at - i as f64;
    let mix = |k: usize| (RAMP[i][k] + (RAMP[i + 1][k] - RAMP[i][k]) * f).round() as u8;
    [mix(0), mix(1), mix(2)]
}

/// The page's elevation colours, low to high.
pub const RAMP: [[f64; 3]; 7] =
    [[59.0, 82.0, 139.0], [46.0, 110.0, 142.0], [35.0, 136.0, 142.0], [31.0, 162.0, 135.0], [59.0, 187.0, 117.0], [117.0, 208.0, 84.0], [189.0, 223.0, 38.0]];

/// A sample of a scan for a view: the cells it sees (all of them with no
/// view), the same first share of each (each cell's points are shuffled in
/// the index, so a first part of one is an even sample of it), about
/// `most` points of those `keep` keeps and `clip` (the view's limit boxes
/// and slice) shows. `affine`: the scan's file into the drawing, for the
/// page's classes, the boxes and the shading (the drawing's north-west and
/// up). Read on every core.
pub fn sample(ix: &Index, most: usize, keep: &Keep, affine: &Affine, sight: Option<&Sight>, clip: Option<&Clip>) -> (Points, Taken) {
    let clip = clip.filter(|c| !c.is_none());
    let drawn = |at: [f64; 3]| {
        [
            affine[0] * at[0] + affine[1] * at[1] + affine[2] * at[2] + affine[3],
            affine[4] * at[0] + affine[5] * at[1] + affine[6] * at[2] + affine[7],
            affine[8] * at[0] + affine[9] * at[1] + affine[10] * at[2] + affine[11],
        ]
    };
    let lo = ix.file_xyz(ix.min[0], ix.min[1], ix.min[2]);
    let hi = ix.file_xyz(ix.max[0], ix.max[1], ix.max[2]);
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let half = [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0, (hi[2] - lo[2]) / 2.0];
    let radius = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let mut taken = wanted(ix, most, sight, clip.map(|c| (c, affine)));
    let list: Vec<usize> = (0..ix.cells).filter(|k| taken.cells[k / 64] >> (k % 64) & 1 == 1).collect();
    let count = taken.of as f64;
    // The share of its points kept by setup and shown by the boxes and slice, judged from a small even sample, so about `most` of those are taken.
    let isolate = keep.isolate.as_ref().filter(|s| !s.is_empty());
    let by_setup = keep.setup.is_some() || !keep.off.is_empty() || clip.is_some() || isolate.is_some();
    let kept = if !by_setup || taken.of == 0 {
        1.0
    } else {
        let probe = (PROBE / count).min(1.0);
        let (seen, passed) = list
            .par_chunks(256)
            .map(|chunk| {
                let (mut seen, mut passed) = (0u64, 0u64);
                for &k in chunk {
                    let cell = ix.cell(k);
                    let take = ((cell.count as f64 * probe).ceil() as u64).clamp(1, cell.count as u64);
                    for i in cell.start..cell.start + take {
                        let p = ix.point(i);
                        seen += 1;
                        if !keep.keeps(p.flags, p.setup) {
                            continue;
                        }
                        // By where it is: the boxes and slice, and what's isolated (by the file's class as mapped: near enough to judge a share by).
                        if clip.is_some() || isolate.is_some() {
                            let d = drawn(ix.file_xyz(p.x, p.y, p.z));
                            let class = keep.map.as_ref().map_or(p.class, |m| m[p.class as usize]);
                            if clip.is_some_and(|c| !c.keeps(d)) || isolate.is_some_and(|s| !s.keeps(d, class)) {
                                continue;
                            }
                        }
                        passed += 1;
                    }
                }
                (seen, passed)
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        if seen == 0 {
            1.0
        } else {
            passed as f64 / seen as f64
        }
    };
    let share = if taken.of == 0 { 0.0 } else { (most as f64 / (count * kept).max(most as f64 / READ_MOST).max(1.0)).min(1.0).min(most as f64 * READ_MOST / count) };
    taken.share = share;
    let byte = |v: u16| if ix.wide { (v >> 8) as u8 } else { v.min(255) as u8 };
    let unshaded = surface::plain();
    let parts: Vec<Vec<PointVertex>> = list
        .par_chunks(128)
        .map(|chunk| {
            let mut out = Vec::new();
            for &k in chunk {
                let cell = ix.cell(k);
                let take = ((cell.count as f64 * share).ceil() as u64).clamp(1, cell.count as u64);
                out.reserve(take as usize);
                for i in cell.start..cell.start + take {
                    let p = ix.point(i);
                    if !keep.keeps(p.flags, p.setup) {
                        continue;
                    }
                    let at = ix.file_xyz(p.x, p.y, p.z);
                    if clip.is_some_and(|c| !c.keeps(drawn(at))) {
                        continue;
                    }
                    let mut class = match &keep.map {
                        Some(m) => m[p.class as usize],
                        None => p.class,
                    };
                    if let Some(b) = &keep.borrow {
                        if let Some((c, gone)) = b.at(drawn(at)) {
                            if gone {
                                continue;
                            }
                            class = c;
                        }
                    }
                    if isolate.is_some_and(|s| !s.keeps(drawn(at), class)) {
                        continue;
                    }
                    out.push(PointVertex {
                        pos: [(at[0] - centre[0]) as f32, (at[1] - centre[1]) as f32, (at[2] - centre[2]) as f32],
                        rgb: if ix.has_rgb { [byte(p.rgb[0]), byte(p.rgb[1]), byte(p.rgb[2])] } else { [0, 0, 0] },
                        class,
                        intensity: p.intensity,
                        setup: p.setup,
                        shade: unshaded,
                        more: [0; 4],
                    });
                }
            }
            out
        })
        .collect();
    let mut data = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for p in parts {
        data.extend_from_slice(&p);
    }
    // Each one's shading from the shape of the surface it's on, as these points make it out.
    let mut shade_ms = 0.0;
    if keep.shade {
        let t = std::time::Instant::now();
        surface::shade_points(&mut data, surface::turn_of(affine).as_ref());
        shade_ms = t.elapsed().as_secs_f64() * 1000.0;
    }
    // Each one's spacing where it is (Fill gaps), in the drawing's units.
    let mut gap = 0.0;
    if keep.gaps {
        let t = std::time::Instant::now();
        let turn = surface::turn_of(affine);
        gap = surface::gap_points(&mut data, turn.as_ref()) * if turn.is_none() { affine[0].abs() } else { 1.0 };
        shade_ms += t.elapsed().as_secs_f64() * 1000.0;
    }
    (Points { centre, radius, colours: ix.has_rgb, shaded: keep.shade, shade_ms, gap, data }, taken)
}

/// The check's own picture: a ring of `n` points 100 across, coloured round
/// it, with a dot of white ones in the middle, about `centre`.
pub fn test_ring(n: usize, centre: [f64; 3]) -> Points {
    let mut data = Vec::with_capacity(n + n / 8);
    let point = |x: f64, y: f64, z: f64, rgb: [u8; 3]| PointVertex { pos: [x as f32, y as f32, z as f32], rgb, class: 0, intensity: 0, setup: 0, shade: surface::plain(), more: [0; 4] };
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        // Three rings close together, so it's a band a few points wide.
        let r = 50.0 - (i % 3) as f64 * 0.6;
        data.push(point(r * a.cos(), r * a.sin(), 0.0, ramp(i as f64 / n as f64)));
    }
    for i in 0..n / 8 {
        let a = i as f64 * 2.399963;
        let r = 6.0 * (i as f64 / (n / 8).max(1) as f64).sqrt();
        data.push(point(r * a.cos(), r * a.sin(), 1.0, [255, 255, 255]));
    }
    Points { centre, radius: 51.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A limit box as the page gives it (point-clouds.ts engineBoxes): its turn undone, a millionth of slack.
    fn limit(min: [f64; 3], max: [f64; 3], angle: f64, outside: bool) -> CropBox {
        let a = -angle.to_radians();
        CropBox { min, max, centre: [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0], cos: a.cos(), sin: a.sin(), turned: a != 0.0, outside, slack: [1e-6; 3] }
    }

    #[test]
    fn limit_boxes_and_a_slice_show_what_the_page_s_show() {
        // None: everything.
        let none = Clip::default();
        assert!(none.is_none() && none.keeps([1e9, -1e9, 5.0]) && none.reaches([0.0; 3], [1.0; 3]));
        // One box showing what's inside it.
        let inside = Clip { boxes: vec![limit([0.0, 0.0, 0.0], [10.0, 4.0, 3.0], 0.0, false)], slice: None, key: String::new() }.keyed();
        assert!(inside.keeps([5.0, 2.0, 1.0]) && !inside.keeps([5.0, 5.0, 1.0]) && !inside.keeps([5.0, 2.0, 4.0]));
        // Two: in either; and out of one that shows what's outside it.
        let two = Clip { boxes: vec![limit([0.0, 0.0, 0.0], [10.0, 4.0, 3.0], 0.0, false), limit([20.0, 0.0, 0.0], [30.0, 4.0, 3.0], 0.0, false), limit([4.0, 1.0, 0.0], [6.0, 3.0, 3.0], 0.0, true)], slice: None, key: String::new() }.keyed();
        assert!(two.keeps([1.0, 2.0, 1.0]) && two.keeps([25.0, 2.0, 1.0]) && !two.keeps([15.0, 2.0, 1.0]) && !two.keeps([5.0, 2.0, 1.0]));
        assert_ne!(inside.key, two.key);
        // Only boxes that show what's outside them: everything but their insides.
        let hole = Clip { boxes: vec![limit([4.0, 1.0, 0.0], [6.0, 3.0, 3.0], 0.0, true)], slice: None, key: String::new() }.keyed();
        assert!(hole.keeps([100.0, 100.0, 100.0]) && !hole.keeps([5.0, 2.0, 1.0]));
        // Turned 45° about its middle (5, 2): its long way runs north-east.
        let turned = Clip { boxes: vec![limit([0.0, 0.0, 0.0], [10.0, 4.0, 3.0], 45.0, false)], slice: None, key: String::new() }.keyed();
        assert!(turned.keeps([8.0, 5.0, 1.0]) && !turned.keeps([9.0, 2.0, 1.0]) && turned.keeps([2.0, -1.0, 1.0]));
        // A slice with the boxes: in both.
        let level = limit([-1e12, -1e12, 0.9], [1e12, 1e12, 1.1], 0.0, false);
        let sliced = Clip { boxes: inside.boxes.clone(), slice: Some(level.clone()), key: String::new() }.keyed();
        assert!(sliced.keeps([5.0, 2.0, 1.0]) && !sliced.keeps([5.0, 2.0, 2.0]) && !sliced.keeps([50.0, 2.0, 1.0]));
        assert_ne!(sliced.key, inside.key);
        // The boxes of the drawing each may show something of: reached, or surely not.
        assert!(inside.reaches([9.0, 3.0, 2.0], [12.0, 6.0, 5.0]) && !inside.reaches([11.0, 0.0, 0.0], [12.0, 4.0, 3.0]) && !inside.reaches([0.0, 0.0, 3.5], [10.0, 4.0, 5.0]));
        assert!(two.reaches([22.0, 1.0, 1.0], [23.0, 2.0, 2.0]) && !two.reaches([12.0, 0.0, 0.0], [18.0, 4.0, 3.0]));
        // One that shows what's outside it turns nothing away (a box inside it isn't looked for).
        assert!(hole.reaches([4.5, 1.5, 1.0], [5.5, 2.5, 2.0]));
        assert!(sliced.reaches([4.0, 1.0, 0.5], [6.0, 3.0, 1.0]) && !sliced.reaches([4.0, 1.0, 1.5], [6.0, 3.0, 2.5]));
        // A turned box: a box off its end but inside its unturned outline isn't reached; one along its turned length is.
        assert!(!turned.reaches([8.5, 0.0, 0.0], [10.0, 0.8, 3.0]) && turned.reaches([7.0, 4.0, 0.0], [8.0, 5.0, 3.0]));
        // A box of a scan's file as the box of the drawing that holds it: moved, scaled and turned.
        let quarter = [0.0, -2.0, 0.0, 100.0, 2.0, 0.0, 0.0, 200.0, 0.0, 0.0, 2.0, 7.0];
        let (lo, hi) = drawn_box(&quarter, [1.0, 2.0, 3.0], [2.0, 4.0, 4.0]);
        assert_eq!((lo, hi), ([92.0, 202.0, 13.0], [96.0, 204.0, 15.0]));
    }

    #[test]
    fn a_vertex_is_28_bytes_as_the_card_reads_it() {
        assert_eq!(std::mem::size_of::<PointVertex>(), 28);
        let v = PointVertex { pos: [1.0, 2.0, 3.0], rgb: [4, 5, 6], class: 7, intensity: 0x0908, setup: 0x0b0a, shade: [12, 13, 14, 15], more: [16, 17, 18, 19] };
        let bytes: &[u8] = bytemuck::bytes_of(&v);
        assert_eq!(&bytes[12..], &[4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]);
    }

    #[test]
    fn points_are_kept_by_setup_and_never_when_withheld() {
        let all = Keep::all();
        assert!(all.keeps(0, 0) && all.keeps(1, 9));
        // Withheld (flag 4): never.
        assert!(!all.keeps(4, 0));
        let one = Keep { setup: Some(3), ..Keep::all() };
        assert!(one.keeps(0, 3) && !one.keeps(0, 0));
        let off = Keep { off: vec![1, 4], ..Keep::all() };
        assert!(off.keeps(0, 0) && !off.keeps(0, 1) && !off.keeps(0, 4));
        // Told apart by their keys.
        let keys: Vec<String> = [all, one, off, Keep { map: Some(Arc::new([3; 256])), ..Keep::all() }].into_iter().map(|k| k.keyed().key).collect();
        for a in 0..keys.len() {
            for b in a + 1..keys.len() {
                assert_ne!(keys[a], keys[b]);
            }
        }
    }

    #[test]
    fn the_pages_elevation_colours_run_blue_to_yellow_green() {
        assert_eq!(ramp(0.0), [59, 82, 139]);
        assert_eq!(ramp(1.0), [189, 223, 38]);
        assert_eq!(ramp(-3.0), ramp(0.0));
        assert_eq!(ramp(0.5), [31, 162, 135]);
    }

    #[test]
    fn the_checks_ring_is_round_its_middle() {
        let ring = test_ring(600, [10.0, 20.0, 30.0]);
        assert_eq!(ring.data.len(), 675);
        assert_eq!(ring.centre, [10.0, 20.0, 30.0]);
        let far = ring.data.iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1]).sqrt()).fold(0.0f32, f32::max);
        assert!(far <= 50.001 && far >= 49.9, "{far}");
    }

    #[test]
    fn a_place_takes_the_class_of_the_pages_point_nearest_it() {
        // The page's points: a row a unit apart from (1000, 2000, 10), their classes 2 and 6 by turns, the third taken out.
        let origin = [1000.0, 2000.0, 10.0];
        let xyz: Vec<f32> = (0..10).flat_map(|i| [i as f32, 0.0, 0.0]).collect();
        let class: Vec<u8> = (0..10).map(|i| if i % 2 == 0 { 2 } else { 6 }).collect();
        let mut removed = vec![0u8; 10];
        removed[2] = 1;
        let points = Arc::new(BorrowPoints::new("p".into(), 1.0, origin, xyz));
        let b = Borrow::new("k".into(), points.clone(), class.clone(), Some(removed)).unwrap();
        // In a point's own cell: its.
        assert_eq!(b.at([1000.2, 2000.3, 10.4]), Some((2, false)));
        assert_eq!(b.at([1001.2, 2000.3, 10.4]), Some((6, false)));
        assert_eq!(b.at([1002.5, 2000.1, 10.1]), Some((2, true)));
        // In a cell with none: the nearest of those round it.
        assert_eq!(b.at([1003.4, 2001.2, 10.0]), Some((6, false)));
        assert_eq!(b.at([1003.4, 1999.5, 10.0]).map(|c| c.0), Some(6));
        assert_eq!(b.at([1003.6, 1999.5, 10.0]).map(|c| c.0), Some(2));
        // Further than a cell from any: none (the file's own class stays).
        assert_eq!(b.at([1003.0, 2003.0, 10.0]), None);
        assert_eq!(b.at([f64::NAN, 0.0, 0.0]), None);
        // Lists of another length aren't taken.
        assert!(Borrow::new("k".into(), points.clone(), vec![0; 9], None).is_none());
        assert!(Borrow::new("k".into(), points, class, Some(vec![0; 3])).is_none());
    }

    #[test]
    fn a_view_sees_the_boxes_that_reach_into_it() {
        // Plan view of a drawing area 200 × 100 px at half a unit a pixel, looking at (50, 50): 100 × 50 units, and 15% more each side.
        let cam = Camera { target: [50.0, 50.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], to_eye: [0.0, 0.0, 1.0], units_per_pixel: 0.5, rect: [0.0, 0.0, 200.0, 100.0] };
        let s = Sight::new(&cam, &crate::camera::IDENTITY);
        assert!(s.sees([40.0, 40.0, -5.0], [60.0, 60.0, 900.0]));
        // Just past the right edge (x 100), inside the margin (15 units); then past it.
        assert!(s.sees([110.0, 50.0, 0.0], [112.0, 51.0, 0.0]));
        assert!(!s.sees([120.0, 50.0, 0.0], [130.0, 51.0, 0.0]));
        assert!(!s.sees([40.0, 90.0, 0.0], [60.0, 95.0, 0.0]));
        // A box across the whole view.
        assert!(s.sees([-1000.0, -1000.0, 0.0], [1000.0, 1000.0, 0.0]));
        // A scan moved 1000 east in the drawing: its box at the file's (−950, 50) is in view.
        let moved: Affine = [1.0, 0.0, 0.0, 1000.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let s = Sight::new(&cam, &moved);
        assert!(s.sees([-955.0, 45.0, 0.0], [-945.0, 55.0, 0.0]));
        assert!(!s.sees([40.0, 40.0, 0.0], [60.0, 60.0, 0.0]));
        // Looking along the ground (north up the screen is the drawing's z): tall boxes are seen by their height.
        let side = Camera { target: [0.0, 0.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], to_eye: [0.0, -1.0, 0.0], units_per_pixel: 1.0, rect: [0.0, 0.0, 100.0, 100.0] };
        let s = Sight::new(&side, &crate::camera::IDENTITY);
        assert!(s.sees([-10.0, 5000.0, -10.0], [10.0, 6000.0, 10.0]));
        assert!(!s.sees([-10.0, 0.0, 80.0], [10.0, 1.0, 90.0]));
    }

    #[test]
    fn a_sample_covers_a_view_inside_it_at_about_its_share() {
        let held = Taken { cells: vec![0b1111, 0b1], share: 0.5, of: 100, clip: String::new() };
        assert!(held.covers(&Taken { cells: vec![0b0110, 0], share: 0.5, of: 40, clip: String::new() }));
        // A little more wanted: still it.
        assert!(held.covers(&Taken { cells: vec![0b0110, 0], share: 0.6, of: 40, clip: String::new() }));
        // Much more (zoomed in), or a cell it hasn't (panned): not.
        assert!(!held.covers(&Taken { cells: vec![0b0110, 0], share: 1.0, of: 40, clip: String::new() }));
        assert!(!held.covers(&Taken { cells: vec![0b10000, 0], share: 0.1, of: 40, clip: String::new() }));
        // Every point of its cells: whatever share is wanted of them.
        let whole = Taken { cells: vec![0b11], share: 1.0, of: 10, clip: String::new() };
        assert!(whole.covers(&Taken { cells: vec![0b01], share: 1.0, of: 5, clip: String::new() }));
        assert!(!whole.covers(&Taken { cells: vec![0b01, 0], share: 1.0, of: 5, clip: String::new() }));
    }
}
