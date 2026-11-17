//! The scan's points under the cursor (desktop 1.19).
//!
//! The drawing's views look along a line (they're parallel views: a point's
//! place on screen is its offset from the view's target along the view's
//! right and up, over the drawing units a pixel). So the points within
//! `reach` pixels of the cursor are those within reach × units-a-pixel of
//! the cursor's line of sight. A block or cell whose box can't come that
//! near is passed over; the points of the rest are each tried.
//!
//! Answered: the points nearest the eye within `front_px` of the cursor
//! (the surface the cursor is on, as drawn: `front`), and the points within
//! `reach` nearest the cursor (`near`), those behind the front surface by
//! more than `window` left out (none when `window` is 0: a view looking
//! straight down, every height under the cursor), or (`band`) those at the
//! heights the page asks. The page then keeps those
//! its view shows (classes off, limit boxes, the slice) and snaps.

use crate::index::Index;
use rayon::prelude::*;

/// A scan asked of: its index, the file's coordinates into the drawing
/// (an affine move, row by row with the shift after each row:
/// x' = a0 x + a1 y + a2 z + a3 …), and the classes a snap may take
/// (bit c of `allow` for class c).
pub struct ScanIn<'a> {
    pub index: &'a Index,
    pub affine: [f64; 12],
    pub allow: [u64; 4],
}

/// The view, the cursor and what's asked.
#[derive(Clone, Debug)]
pub struct SnapQuery {
    pub target: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub to_eye: [f64; 3],
    pub units_per_pixel: f64,
    pub half_w: f64,
    pub half_h: f64,
    pub cursor: [f64; 2],
    /// Pixels round the cursor.
    pub reach: f64,
    pub front_px: f64,
    /// Drawing units behind the front surface kept; 0: every height.
    pub window: f64,
    /// Points nearer the eye than this left out (the page's front was further back).
    pub below: Option<f64>,
    /// Only points this near the eye, low and high, in `near` (the page's
    /// own front ± its window); in place of `window`.
    pub band: Option<[f64; 2]>,
    pub most: usize,
    pub front_most: usize,
}

/// A point found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Its scan's place in the question.
    pub scan: u32,
    /// Its place in its index.
    pub index: u64,
    /// Where it is in the drawing.
    pub p: [f64; 3],
    /// Pixels from the cursor.
    pub d: f64,
    /// How near the eye (along the view's line toward it).
    pub h: f64,
    pub class: u8,
    pub flags: u8,
    pub intensity: u16,
    pub returns: u8,
    pub rgb: [u16; 3],
}

#[derive(Clone, Debug, Default)]
pub struct SnapAnswer {
    pub front: Vec<Candidate>,
    pub near: Vec<Candidate>,
    /// Points tried and cells looked in (for Scan Stats).
    pub tried: u64,
    pub cells: u64,
}

/// More points than this to try, and they're tried on every core.
const PARALLEL_FROM: u64 = 65_536;
/// Withheld points (the file marks them not to be used) are never snapped to.
const WITHHELD: u8 = 4;

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn moved(a: &[f64; 12], p: [f64; 3]) -> [f64; 3] {
    [
        a[0] * p[0] + a[1] * p[1] + a[2] * p[2] + a[3],
        a[4] * p[0] + a[5] * p[1] + a[6] * p[2] + a[7],
        a[8] * p[0] + a[9] * p[1] + a[10] * p[2] + a[11],
    ]
}

struct Line {
    at: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    to_eye: [f64; 3],
    r2: f64,
    upp: f64,
}

impl Line {
    /// Whether a box (the file's whole numbers) comes within reach of the line, and how near the eye it reaches.
    fn reaches(&self, ix: &Index, a: &[f64; 12], min: [i32; 3], max: [i32; 3], below: Option<f64>) -> bool {
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        let mut h0 = f64::MAX;
        for k in 0..8 {
            let c = [
                if k & 1 == 0 { min[0] } else { max[0] },
                if k & 2 == 0 { min[1] } else { max[1] },
                if k & 4 == 0 { min[2] } else { max[2] },
            ];
            let p = moved(a, ix.file_xyz(c[0], c[1], c[2]));
            let o = [p[0] - self.at[0], p[1] - self.at[1], p[2] - self.at[2]];
            let (u, v) = (dot(o, self.right), dot(o, self.up));
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
            h0 = h0.min(dot(p, self.to_eye));
        }
        if let Some(b) = below {
            if h0 > b {
                return false;
            }
        }
        // The box's shadow on the screen's plane against the circle round the cursor.
        let du = if u0 > 0.0 { u0 } else if u1 < 0.0 { -u1 } else { 0.0 };
        let dv = if v0 > 0.0 { v0 } else if v1 < 0.0 { -v1 } else { 0.0 };
        du * du + dv * dv <= self.r2
    }
}

/// The points under the cursor of every scan asked of.
pub fn snap(scans: &[ScanIn], q: &SnapQuery) -> SnapAnswer {
    let upp = q.units_per_pixel;
    let mut answer = SnapAnswer::default();
    if !(upp > 0.0) || !(q.reach > 0.0) {
        return answer;
    }
    let sx = (q.cursor[0] - q.half_w) * upp;
    let sy = -(q.cursor[1] - q.half_h) * upp;
    let line = Line {
        at: [
            q.target[0] + q.right[0] * sx + q.up[0] * sy,
            q.target[1] + q.right[1] * sx + q.up[1] * sy,
            q.target[2] + q.right[2] * sx + q.up[2] * sy,
        ],
        right: q.right,
        up: q.up,
        to_eye: q.to_eye,
        r2: (q.reach * upp) * (q.reach * upp),
        upp,
    };
    let mut found: Vec<Candidate> = Vec::new();
    for (slot, s) in scans.iter().enumerate() {
        let ix = s.index;
        let mut hit: Vec<usize> = Vec::new();
        let mut points: u64 = 0;
        for b in 0..ix.blocks {
            let blk = ix.block(b);
            if !line.reaches(ix, &s.affine, blk.min, blk.max, q.below) {
                continue;
            }
            for c in blk.first as usize..(blk.first as usize + blk.cells as usize) {
                let cell = ix.cell(c);
                if line.reaches(ix, &s.affine, cell.min, cell.max, q.below) {
                    hit.push(c);
                    points += cell.count as u64;
                }
            }
        }
        answer.cells += hit.len() as u64;
        answer.tried += points;
        let take = |c: &usize| -> Vec<Candidate> {
            let cell = ix.cell(*c);
            let mut out = Vec::new();
            for i in cell.start..cell.start + cell.count as u64 {
                let r = ix.point(i);
                if r.flags & WITHHELD != 0 {
                    continue;
                }
                if s.allow[(r.class >> 6) as usize] >> (r.class & 63) & 1 == 0 {
                    continue;
                }
                let p = moved(&s.affine, ix.file_xyz(r.x, r.y, r.z));
                let o = [p[0] - line.at[0], p[1] - line.at[1], p[2] - line.at[2]];
                let (u, v) = (dot(o, line.right), dot(o, line.up));
                let d2 = u * u + v * v;
                if d2 > line.r2 {
                    continue;
                }
                let h = dot(p, line.to_eye);
                if let Some(b) = q.below {
                    if h > b {
                        continue;
                    }
                }
                out.push(Candidate {
                    scan: slot as u32,
                    index: i,
                    p,
                    d: d2.sqrt() / line.upp,
                    h,
                    class: r.class,
                    flags: r.flags,
                    intensity: r.intensity,
                    returns: r.returns,
                    rgb: r.rgb,
                });
            }
            out
        };
        if points >= PARALLEL_FROM {
            let got: Vec<Vec<Candidate>> = hit.par_iter().map(take).collect();
            for g in got {
                found.extend(g);
            }
        } else {
            for c in &hit {
                found.extend(take(c));
            }
        }
    }
    // The front surface: nearest the eye within front_px.
    let mut front: Vec<Candidate> = found.iter().filter(|c| c.d <= q.front_px).copied().collect();
    front.sort_unstable_by(|a, b| b.h.total_cmp(&a.h));
    front.truncate(q.front_most);
    let mut near: Vec<Candidate> = if let Some([lo, hi]) = q.band {
        found.into_iter().filter(|c| c.h >= lo && c.h <= hi).collect()
    } else {
        let floor = if q.window > 0.0 {
            front.first().map(|f| f.h - q.window)
        } else {
            None
        };
        match floor {
            Some(f) => found.into_iter().filter(|c| c.h >= f).collect(),
            None => found,
        }
    };
    near.sort_unstable_by(|a, b| a.d.total_cmp(&b.d).then(b.h.total_cmp(&a.h)));
    near.truncate(q.most);
    answer.front = front;
    answer.near = near;
    answer
}
