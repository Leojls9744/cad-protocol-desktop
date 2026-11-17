//! A scan's points in a box of the drawing (desktop 1.20): what a view
//! close up draws, and what a command asks for (every point in a region).
//!
//! The cells the box reaches (their boxes moved into the drawing) are
//! found by the blocks first. If they hold more than `most`, the same
//! first share of each cell is taken (cells are shuffled, so that's an even
//! sample), else every point. Each point taken is moved into the drawing
//! and kept when it's in the box (and in the ring, if one's given: a
//! region's outline in plan).

use crate::index::Index;
use rayon::prelude::*;

/// What's asked.
#[derive(Clone, Debug)]
pub struct RegionQuery {
    /// The box, in the drawing.
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// A region's outline in plan (x, y each), inside the box; none: the box.
    pub ring: Option<Vec<[f64; 2]>>,
    /// About the most points to give.
    pub most: usize,
}

/// The points found, placed about the box's middle.
#[derive(Clone, Debug, Default)]
pub struct RegionPoints {
    pub origin: [f64; 3],
    pub xyz: Vec<f32>,
    pub intensity: Vec<u16>,
    pub class: Vec<u8>,
    pub flags: Vec<u8>,
    pub returns: Vec<u8>,
    /// Bytes (16-bit colours by their high byte); none when the scan has no colour.
    pub rgb: Option<Vec<u8>>,
    /// Every point in the box came (none left out for `most`).
    pub complete: bool,
    /// Points in the cells the box reaches, and cells.
    pub tried: u64,
    pub cells: u64,
}

const WITHHELD: u8 = 4;
const PARALLEL_FROM: u64 = 200_000;

#[inline]
fn moved(a: &[f64; 12], p: [f64; 3]) -> [f64; 3] {
    [
        a[0] * p[0] + a[1] * p[1] + a[2] * p[2] + a[3],
        a[4] * p[0] + a[5] * p[1] + a[6] * p[2] + a[7],
        a[8] * p[0] + a[9] * p[1] + a[10] * p[2] + a[11],
    ]
}

/// A box (the file's whole numbers) moved into the drawing: the box round its corners.
fn moved_box(ix: &Index, a: &[f64; 12], min: [i32; 3], max: [i32; 3]) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for k in 0..8 {
        let c = [
            if k & 1 == 0 { min[0] } else { max[0] },
            if k & 2 == 0 { min[1] } else { max[1] },
            if k & 4 == 0 { min[2] } else { max[2] },
        ];
        let p = moved(a, ix.file_xyz(c[0], c[1], c[2]));
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

#[inline]
fn overlaps(lo: [f64; 3], hi: [f64; 3], q: &RegionQuery) -> bool {
    (0..3).all(|i| lo[i] <= q.max[i] && hi[i] >= q.min[i])
}

/// Whether a point in plan is inside a ring (even-odd).
pub fn in_ring(ring: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

struct Part {
    xyz: Vec<f32>,
    intensity: Vec<u16>,
    class: Vec<u8>,
    flags: Vec<u8>,
    returns: Vec<u8>,
    rgb: Vec<u8>,
}

/// The scan's points in a box of the drawing.
pub fn region(ix: &Index, affine: &[f64; 12], q: &RegionQuery) -> RegionPoints {
    let origin = [
        (q.min[0] + q.max[0]) / 2.0,
        (q.min[1] + q.max[1]) / 2.0,
        (q.min[2] + q.max[2]) / 2.0,
    ];
    let mut hit: Vec<usize> = Vec::new();
    let mut total: u64 = 0;
    for b in 0..ix.blocks {
        let blk = ix.block(b);
        let (lo, hi) = moved_box(ix, affine, blk.min, blk.max);
        if !overlaps(lo, hi, q) {
            continue;
        }
        for c in blk.first as usize..(blk.first as usize + blk.cells as usize) {
            let cell = ix.cell(c);
            let (lo, hi) = moved_box(ix, affine, cell.min, cell.max);
            if overlaps(lo, hi, q) {
                hit.push(c);
                total += cell.count as u64;
            }
        }
    }
    let share = if total as usize > q.most && total > 0 {
        q.most as f64 / total as f64
    } else {
        1.0
    };
    let wide = ix.wide;
    let with_rgb = ix.has_rgb;
    let take = |c: &usize| -> Part {
        let cell = ix.cell(*c);
        let n = if share >= 1.0 {
            cell.count as u64
        } else {
            ((cell.count as f64 * share).ceil() as u64).min(cell.count as u64)
        };
        let mut part = Part {
            xyz: Vec::with_capacity(n as usize * 3),
            intensity: Vec::with_capacity(n as usize),
            class: Vec::with_capacity(n as usize),
            flags: Vec::with_capacity(n as usize),
            returns: Vec::with_capacity(n as usize),
            rgb: Vec::with_capacity(if with_rgb { n as usize * 3 } else { 0 }),
        };
        for i in cell.start..cell.start + n {
            let r = ix.point(i);
            if r.flags & WITHHELD != 0 {
                continue;
            }
            let p = moved(affine, ix.file_xyz(r.x, r.y, r.z));
            if (0..3).any(|k| p[k] < q.min[k] || p[k] > q.max[k]) {
                continue;
            }
            if let Some(ring) = &q.ring {
                if !in_ring(ring, p[0], p[1]) {
                    continue;
                }
            }
            part.xyz.extend_from_slice(&[
                (p[0] - origin[0]) as f32,
                (p[1] - origin[1]) as f32,
                (p[2] - origin[2]) as f32,
            ]);
            part.intensity.push(r.intensity);
            part.class.push(r.class);
            part.flags.push(r.flags);
            part.returns.push(r.returns);
            if with_rgb {
                let s = if wide { 8 } else { 0 };
                for v in r.rgb {
                    part.rgb.push(((v >> s) as u32).min(255) as u8);
                }
            }
        }
        part
    };
    let parts: Vec<Part> = if total >= PARALLEL_FROM {
        hit.par_iter().map(take).collect()
    } else {
        hit.iter().map(take).collect()
    };
    let n: usize = parts.iter().map(|p| p.class.len()).sum();
    let mut out = RegionPoints {
        origin,
        xyz: Vec::with_capacity(n * 3),
        intensity: Vec::with_capacity(n),
        class: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        returns: Vec::with_capacity(n),
        rgb: if with_rgb { Some(Vec::with_capacity(n * 3)) } else { None },
        complete: share >= 1.0,
        tried: total,
        cells: hit.len() as u64,
    };
    for p in parts {
        out.xyz.extend(p.xyz);
        out.intensity.extend(p.intensity);
        out.class.extend(p.class);
        out.flags.extend(p.flags);
        out.returns.extend(p.returns);
        if let Some(rgb) = out.rgb.as_mut() {
            rgb.extend(p.rgb);
        }
    }
    out
}
