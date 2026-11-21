//! The brush on the card (the page's POINTCLOUDBRUSH): its preview under
//! the cursor, and its strokes that the points on the card haven't been
//! read with yet.
//!
//! The preview is the page's own (cloud-gl.ts, scan-brush-preview.ts): the
//! points under the brush's circle tinted in its colour while it hovers,
//! and those under it and its latest few dabs shown painted (or gone) while
//! it paints.
//!
//! A stroke is a press's dabs (circles across and up the view it was
//! painted in, each all the way through that view, or from a cut toward
//! the eye where it painted the front surface only) and what it does to
//! the points inside them: gives them a class, hides them, or keeps only
//! them. The card tests every point it draws by the strokes each frame, so
//! a press shows as it's painted, on every point, with nothing read again;
//! once the press is done the page gives the engine the scan's classes and
//! isolated shapes as they are now, the scan is read with them, and the
//! stroke is dropped. A point of a class a stroke leaves alone (one whose
//! layer was off or locked as it was painted) is never inside it.
//!
//! The strokes that give a class are put on first, in turn, then those
//! that hide or keep, in turn, by the class each point has then: as the
//! page tests its own points (scan-edits.ts: the shapes isolated are
//! tested by a point's class as it is now).

/// The most strokes the card tests at once (more: the page draws the scans till the engine has read them).
pub const STROKES: usize = 4;
/// The most of its latest dabs a preview shows painted (the page's PREVIEW_DABS).
pub const PREVIEW_DABS: usize = 16;
/// The most dabs a stroke holds for the card.
pub const STROKE_DABS: usize = 65_536;
/// How much of the brush's colour a point under it takes while it hovers.
pub const HOVER_TINT: f32 = 0.45;

/// The brush's preview: its middle in the window and its radius (pixels),
/// its colour, whether it's painting (the points under it and its latest
/// dabs shown painted: in its colour, or gone with `hide`), and its cut
/// toward the eye in the drawing (only the points at it or in front of it:
/// the front surface it would take; none: all the way through).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub at: [f64; 2],
    pub r: f64,
    pub painting: bool,
    pub color: [f32; 3],
    pub hide: bool,
    pub cut: Option<f64>,
    /// Its latest dabs' middles in the window, pixels.
    pub dabs: Vec<[f64; 2]>,
}

/// What a stroke does to the points inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeOp {
    /// Gives them this class.
    Class(u8),
    /// Hides them.
    Out,
    /// Hides every other point.
    Keep,
}

/// A stroke as the page gives it: the view it was painted across (its
/// across and up in the drawing), its dabs across and up it (u, v, radius,
/// …), each one's cut toward the eye (`near`: one a dab, or none), and the
/// classes it leaves alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub op: StrokeOp,
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub dabs: Vec<f64>,
    pub near: Option<Vec<f64>>,
    pub off: Vec<u8>,
}

/// The strokes on the card for a view, under the key the page names them by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strokes {
    pub key: String,
    pub list: Vec<Stroke>,
}

impl Strokes {
    /// At most STROKES of them, each with at most STROKE_DABS dabs (the page gives no more).
    pub fn new(key: String, mut list: Vec<Stroke>) -> Strokes {
        list.truncate(STROKES);
        for s in &mut list {
            let n = (s.dabs.len() / 3).min(STROKE_DABS);
            s.dabs.truncate(n * 3);
            if s.near.as_ref().is_some_and(|c| c.len() < n) {
                s.near = None;
            }
            if let Some(c) = &mut s.near {
                c.truncate(n);
            }
        }
        Strokes { key, list }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// A stroke as the card tests it: its dabs filed by cell across and up its
/// view (each cell at least a dab across, at most 256 a side), their places
/// from an origin of the stroke's own (so a drawing far from 0 keeps them).
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// 0 gives a class, 1 hides, 2 keeps only.
    pub op: u32,
    pub class: u32,
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub eye: [f64; 3],
    /// Its origin: across, up and toward the eye.
    pub origin: [f64; 3],
    /// Its cells' low corner from the origin, and one over a cell's size.
    pub lo: [f32; 2],
    pub inv_cell: f32,
    pub nx: u32,
    pub ny: u32,
    /// Where its cells start in `Packed::cells`.
    pub first: u32,
    /// The classes it leaves alone, a bit each.
    pub off: [u32; 8],
}

/// The strokes as the card's given them: each one's head; `cells`, for each
/// stroke its cells in rows, each the place in `dabs` its dabs start at (one
/// more ends the last); `dabs`, each cell's dabs one after another (across
/// and up from the stroke's origin, radius, and its cut toward the eye from
/// the origin: far behind for none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Packed {
    pub heads: Vec<Head>,
    pub cells: Vec<u32>,
    pub dabs: Vec<[f32; 4]>,
}

/// No cut: every point, however far back.
const THROUGH: f32 = -3.0e38;
/// A dab painted over nothing on the front surface: no point.
const NOWHERE: f32 = 3.0e38;

pub fn pack(strokes: &[Stroke]) -> Packed {
    let mut out = Packed::default();
    for s in strokes.iter().take(STROKES) {
        let n = (s.dabs.len() / 3).min(STROKE_DABS);
        let cuts = s.near.as_ref().filter(|c| c.len() >= n);
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        let mut most = 0.0f64;
        for k in 0..n {
            let (u, v, r) = (s.dabs[3 * k], s.dabs[3 * k + 1], s.dabs[3 * k + 2].abs());
            if !(u.is_finite() && v.is_finite() && r.is_finite()) {
                continue;
            }
            lo = [lo[0].min(u - r), lo[1].min(v - r)];
            hi = [hi[0].max(u + r), hi[1].max(v + r)];
            most = most.max(r);
        }
        let eye = [s.right[1] * s.up[2] - s.right[2] * s.up[1], s.right[2] * s.up[0] - s.right[0] * s.up[2], s.right[0] * s.up[1] - s.right[1] * s.up[0]];
        let mut off = [0u32; 8];
        for c in &s.off {
            off[(*c >> 5) as usize] |= 1 << (*c & 31);
        }
        let (op, class) = match s.op {
            StrokeOp::Class(c) => (0, c as u32),
            StrokeOp::Out => (1, 0),
            StrokeOp::Keep => (2, 0),
        };
        let first = out.cells.len() as u32;
        let empty = !(hi[0] >= lo[0] && hi[1] >= lo[1]);
        if empty {
            // No dab: one cell with none in it (a stroke that keeps only then hides every point, as the page's does).
            out.heads.push(Head { op, class, right: s.right, up: s.up, eye, origin: [0.0; 3], lo: [0.0; 2], inv_cell: 1.0, nx: 1, ny: 1, first, off });
            let at = out.dabs.len() as u32;
            out.cells.extend_from_slice(&[at, at]);
            continue;
        }
        // A hair past its dabs each way: a point on the very edge of the outermost dab is in a cell.
        let pad = most * 1e-3 + 1e-9;
        lo = [lo[0] - pad, lo[1] - pad];
        hi = [hi[0] + pad, hi[1] + pad];
        let mut cell = (2.0 * most).max((hi[0] - lo[0]) / 256.0).max((hi[1] - lo[1]) / 256.0);
        if !(cell > 0.0) || !cell.is_finite() {
            cell = 1.0;
        }
        let nx = (((hi[0] - lo[0]) / cell).ceil() as usize).clamp(1, 256);
        let ny = (((hi[1] - lo[1]) / cell).ceil() as usize).clamp(1, 256);
        // The origin: the middle of its dabs, and the first cut there is along the line of sight.
        let origin = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, cuts.and_then(|c| c.iter().take(n).copied().find(|v| v.is_finite() && v.abs() < 1e29)).unwrap_or(0.0)];
        let col = |u: f64| (((u - lo[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
        let row = |v: f64| (((v - lo[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
        let mut counts = vec![0u32; nx * ny + 1];
        let each = |put: &mut dyn FnMut(usize, usize)| {
            for k in 0..n {
                let (u, v, r) = (s.dabs[3 * k], s.dabs[3 * k + 1], s.dabs[3 * k + 2].abs());
                if !(u.is_finite() && v.is_finite() && r.is_finite()) {
                    continue;
                }
                for cy in row(v - r)..=row(v + r) {
                    for cx in col(u - r)..=col(u + r) {
                        put(cy * nx + cx, k);
                    }
                }
            }
        };
        each(&mut |c, _| counts[c + 1] += 1);
        for c in 1..=nx * ny {
            counts[c] += counts[c - 1];
        }
        let base = out.dabs.len() as u32;
        let mut fill: Vec<u32> = counts[..nx * ny].to_vec();
        out.dabs.resize(out.dabs.len() + counts[nx * ny] as usize, [0.0; 4]);
        each(&mut |c, k| {
            let cut = match cuts.map(|c| c[k]) {
                None => THROUGH,
                Some(v) if !v.is_finite() || v >= 1e29 => NOWHERE,
                Some(v) if v <= -1e29 => THROUGH,
                Some(v) => (v - origin[2]) as f32,
            };
            out.dabs[(base + fill[c]) as usize] = [(s.dabs[3 * k] - origin[0]) as f32, (s.dabs[3 * k + 1] - origin[1]) as f32, s.dabs[3 * k + 2].abs() as f32, cut];
            fill[c] += 1;
        });
        out.cells.extend(counts.iter().map(|c| base + c));
        out.heads.push(Head {
            op,
            class,
            right: s.right,
            up: s.up,
            eye,
            origin,
            lo: [(lo[0] - origin[0]) as f32, (lo[1] - origin[1]) as f32],
            inv_cell: (1.0 / cell) as f32,
            nx: nx as u32,
            ny: ny as u32,
            first,
            off,
        });
    }
    out
}

impl Packed {
    /// Whether stroke `k` holds a place in the drawing (as the card tests it: the same numbers, on the processor), for a point of class `class`.
    pub fn holds(&self, k: usize, p: [f64; 3], class: u8) -> bool {
        let Some(h) = self.heads.get(k) else { return false };
        if h.off[(class >> 5) as usize] >> (class & 31) & 1 == 1 {
            return false;
        }
        let dot = |a: &[f64; 3]| a[0] * p[0] + a[1] * p[1] + a[2] * p[2];
        let (u, v, d) = ((dot(&h.right) - h.origin[0]) as f32, (dot(&h.up) - h.origin[1]) as f32, (dot(&h.eye) - h.origin[2]) as f32);
        let (fx, fy) = ((u - h.lo[0]) * h.inv_cell, (v - h.lo[1]) * h.inv_cell);
        if !(fx >= 0.0 && fy >= 0.0 && fx < h.nx as f32 && fy < h.ny as f32) {
            return false;
        }
        let cell = (h.first + fy as u32 * h.nx + fx as u32) as usize;
        self.dabs[self.cells[cell] as usize..self.cells[cell + 1] as usize].iter().any(|dab| {
            let (du, dv) = (u - dab[0], v - dab[1]);
            du * du + dv * dv <= dab[2] * dab[2] && d >= dab[3]
        })
    }

    /// What the strokes leave of a point: its class as they give it, or none where they hide it.
    pub fn leaves(&self, p: [f64; 3], class: u8) -> Option<u8> {
        let mut class = class;
        for (k, h) in self.heads.iter().enumerate() {
            if h.op == 0 && self.holds(k, p, class) {
                class = h.class as u8;
            }
        }
        for (k, h) in self.heads.iter().enumerate() {
            if h.op == 0 {
                continue;
            }
            let inside = self.holds(k, p, class);
            if (h.op == 1 && inside) || (h.op == 2 && !inside) {
                return None;
            }
        }
        Some(class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: ([f64; 3], [f64; 3]) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

    fn stroke(op: StrokeOp, dabs: Vec<f64>) -> Stroke {
        Stroke { op, right: PLAN.0, up: PLAN.1, dabs, near: None, off: Vec::new() }
    }

    #[test]
    fn a_strokes_dabs_hold_what_the_pages_do() {
        // Three dabs along a line far from 0, radius 1: inside any of them, all the way through.
        let far = [6_000_000.0, 2_000_000.0];
        let s = stroke(StrokeOp::Out, vec![far[0], far[1], 1.0, far[0] + 1.5, far[1], 1.0, far[0] + 3.0, far[1], 1.0]);
        let p = pack(std::slice::from_ref(&s));
        assert_eq!(p.heads.len(), 1);
        assert!(p.holds(0, [far[0], far[1], 5.0], 0) && p.holds(0, [far[0] + 3.9, far[1], -90.0], 0) && p.holds(0, [far[0] + 0.75, far[1] + 0.6, 0.0], 0));
        assert!(!p.holds(0, [far[0] - 1.1, far[1], 0.0], 0) && !p.holds(0, [far[0] + 0.75, far[1] + 0.9, 0.0], 0) && !p.holds(0, [far[0] + 4.1, far[1], 0.0], 0));
        // A point exactly a radius off is inside (the page's test: no farther than the radius).
        assert!(p.holds(0, [far[0], far[1] + 1.0, 0.0], 0));
        // Hidden inside, left outside.
        assert_eq!(p.leaves([far[0], far[1], 0.0], 7), None);
        assert_eq!(p.leaves([far[0] - 2.0, far[1], 0.0], 7), Some(7));
        // The classes it leaves alone are never inside it.
        let left = Stroke { off: vec![2, 200], ..s.clone() };
        let p = pack(std::slice::from_ref(&left));
        assert!(!p.holds(0, [far[0], far[1], 0.0], 2) && !p.holds(0, [far[0], far[1], 0.0], 200) && p.holds(0, [far[0], far[1], 0.0], 3));
        // Front surface only: each dab from its cut toward the eye (up, in plan); one painted over nothing holds none.
        let front = Stroke { near: Some(vec![10.0, 20.0, 1e30]), ..s.clone() };
        let p = pack(std::slice::from_ref(&front));
        assert!(p.holds(0, [far[0], far[1], 10.0], 0) && !p.holds(0, [far[0], far[1], 9.99], 0));
        assert!(p.holds(0, [far[0] + 1.5, far[1] + 0.9, 20.5], 0) && !p.holds(0, [far[0] + 2.4, far[1], 15.0], 0));
        assert!(!p.holds(0, [far[0] + 3.5, far[1], 1e6], 0));
    }

    #[test]
    fn strokes_give_classes_first_then_hide_or_keep_by_them() {
        // One gives class 6 round (0, 0); one hides round (1, 0) but leaves class 6 alone; one keeps only round (0, 0), radius 3.
        let class = stroke(StrokeOp::Class(6), vec![0.0, 0.0, 1.0]);
        let hide = Stroke { off: vec![6], ..stroke(StrokeOp::Out, vec![1.0, 0.0, 1.0]) };
        let keep = stroke(StrokeOp::Keep, vec![0.0, 0.0, 3.0]);
        // Whatever order they come in: the class is given before the others test by it.
        for list in [vec![class.clone(), hide.clone(), keep.clone()], vec![keep.clone(), hide.clone(), class.clone()]] {
            let p = pack(&list);
            // Under both the class's and the hide's: made class 6, which the hide leaves alone.
            assert_eq!(p.leaves([0.5, 0.0, 0.0], 2), Some(6));
            // Under the hide's alone: gone.
            assert_eq!(p.leaves([1.8, 0.0, 0.0], 2), None);
            // Under neither, inside the keep's: as it was. Outside the keep's: gone.
            assert_eq!(p.leaves([-2.0, 0.0, 0.0], 2), Some(2));
            assert_eq!(p.leaves([3.5, 0.0, 0.0], 2), None);
        }
        // Two classes over the same points: the later one's.
        let p = pack(&[class.clone(), stroke(StrokeOp::Class(9), vec![0.5, 0.0, 1.0])]);
        assert_eq!(p.leaves([0.4, 0.0, 0.0], 2), Some(9));
        assert_eq!(p.leaves([-0.8, 0.0, 0.0], 2), Some(6));
        // A stroke that keeps only, with no dab: no point.
        let p = pack(&[stroke(StrokeOp::Keep, Vec::new())]);
        assert_eq!(p.leaves([0.0, 0.0, 0.0], 2), None);
        // No more than the card tests, nor more dabs than a stroke holds.
        let many = Strokes::new("k".into(), vec![class.clone(); STROKES + 3]);
        assert_eq!(many.list.len(), STROKES);
        let long = Strokes::new("k".into(), vec![Stroke { near: Some(vec![0.0; 2]), ..stroke(StrokeOp::Out, vec![0.0; 9]) }]);
        assert!(long.list[0].near.is_none());
    }

    #[test]
    fn many_dabs_are_filed_by_cell_and_all_found() {
        // A long scribble: 4,000 dabs along a sine, radius 0.5; every point within a radius of a dab is inside, as tested one dab at a time.
        let mut dabs = Vec::new();
        for i in 0..4000 {
            let t = i as f64 * 0.05;
            dabs.extend_from_slice(&[1000.0 + t, 500.0 + 8.0 * (t * 0.3).sin(), 0.5]);
        }
        let s = stroke(StrokeOp::Out, dabs.clone());
        let p = pack(std::slice::from_ref(&s));
        // Cells at least a dab across.
        assert!(p.heads[0].inv_cell <= 1.0 && p.heads[0].nx <= 256 && p.heads[0].ny <= 256);
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut inside = 0;
        for _ in 0..20_000 {
            let q = [1000.0 + next() * 200.0, 490.0 + next() * 20.0, 0.0];
            // Not within a hair of a dab's edge: there the card's single precision may fall either side.
            let nearest = (0..4000).map(|k| ((q[0] - dabs[3 * k]).powi(2) + (q[1] - dabs[3 * k + 1]).powi(2)).sqrt()).fold(f64::INFINITY, f64::min);
            if (nearest - 0.5).abs() < 1e-3 {
                continue;
            }
            assert_eq!(p.holds(0, q, 0), nearest <= 0.5, "at {q:?}, {nearest} from the nearest dab");
            inside += (nearest <= 0.5) as u32;
        }
        assert!(inside > 1000);
    }
}
