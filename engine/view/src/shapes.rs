//! The scan points isolated (the page's ISOLATEPOINTS, HIDEPOINTS and the
//! brush's Isolate; scan-isolate.ts, scan-edits.ts shapesKeeper): shapes
//! drawn in views, each all the way through its view, kept in turn. One
//! with no op adds the points inside it; "and" keeps only those inside it
//! of what's kept so far; "out" takes those inside it away. A list that
//! starts with "and" or "out" starts from every point. A point of a class
//! that was off as a shape was drawn is never inside it.
//!
//! A shape is its view's across and up (three numbers each) and its
//! corners across and up it; one painted with the brush has its dabs too
//! (circles across and up the view, each with a cut toward the eye where
//! it was painted on a 3D view's front surface only).
//!
//! The engine tests every point it reads by them, as the page tests its
//! own, so what's isolated is the scan's every point there, not the few
//! the page keeps.

/// What a shape does to the points kept by the shapes before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    And,
    Out,
}

/// A polygon across and up a view (u, v, u, v, …), tested even-odd, its edges in bands by v (the page's bandTest).
struct Polygon {
    poly: Vec<f64>,
    n: usize,
    lo: [f64; 2],
    hi: [f64; 2],
    step: f64,
    bands: Vec<Vec<u32>>,
}

impl Polygon {
    fn new(poly: Vec<f64>) -> Polygon {
        let n = poly.len() / 2;
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for k in 0..n {
            for a in 0..2 {
                lo[a] = lo[a].min(poly[2 * k + a]);
                hi[a] = hi[a].max(poly[2 * k + a]);
            }
        }
        let count = n.clamp(1, 1024);
        let mut step = (hi[1] - lo[1]) / count as f64;
        if !(step > 0.0) || !step.is_finite() {
            step = 1.0;
        }
        let mut bands = vec![Vec::new(); count];
        if n >= 3 {
            let band = |v: f64| (((v - lo[1]) / step).floor().max(0.0) as usize).min(count - 1);
            for k in 0..n {
                let a = poly[2 * k + 1];
                let b = poly[2 * ((k + 1) % n) + 1];
                for edges in &mut bands[band(a.min(b))..=band(a.max(b))] {
                    edges.push(k as u32);
                }
            }
        }
        Polygon { poly, n, lo, hi, step, bands }
    }

    #[inline]
    fn holds(&self, u: f64, v: f64) -> bool {
        if self.n < 3 || !(u >= self.lo[0] && u <= self.hi[0] && v >= self.lo[1] && v <= self.hi[1]) {
            return false;
        }
        let band = (((v - self.lo[1]) / self.step).floor().max(0.0) as usize).min(self.bands.len() - 1);
        let mut inside = false;
        for &k in &self.bands[band] {
            let k = k as usize;
            let (au, av) = (self.poly[2 * k], self.poly[2 * k + 1]);
            let j = (k + 1) % self.n;
            let (bu, bv) = (self.poly[2 * j], self.poly[2 * j + 1]);
            // Each edge counted where it crosses the line across to the right of the point (its lower end in, its upper out).
            if (av > v) != (bv > v) && u < au + ((v - av) * (bu - au)) / (bv - av) {
                inside = !inside;
            }
        }
        inside
    }
}

/// A brushed shape's dabs, filed by cell (each cell at least a dab across, at most 256 a side), with each one's cut toward the eye.
struct Dabs {
    dabs: Vec<f64>,
    cuts: Option<Vec<f64>>,
    eye: [f64; 3],
    lo: [f64; 2],
    hi: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    start: Vec<u32>,
    order: Vec<u32>,
}

impl Dabs {
    fn new(dabs: Vec<f64>, near: Option<Vec<f64>>, right: [f64; 3], up: [f64; 3]) -> Dabs {
        let n = dabs.len() / 3;
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        let mut most = 0.0f64;
        for k in 0..n {
            let r = dabs[3 * k + 2].abs();
            lo = [lo[0].min(dabs[3 * k] - r), lo[1].min(dabs[3 * k + 1] - r)];
            hi = [hi[0].max(dabs[3 * k] + r), hi[1].max(dabs[3 * k + 1] + r)];
            most = most.max(r);
        }
        let eye = [right[1] * up[2] - right[2] * up[1], right[2] * up[0] - right[0] * up[2], right[0] * up[1] - right[1] * up[0]];
        let mut out = Dabs { dabs, cuts: None, eye, lo, hi, cell: 1.0, nx: 0, ny: 0, start: Vec::new(), order: Vec::new() };
        if n == 0 || !(hi[0] >= lo[0] && hi[1] >= lo[1]) {
            return out;
        }
        let mut cell = (2.0 * most).max((hi[0] - lo[0]) / 256.0).max((hi[1] - lo[1]) / 256.0);
        if !(cell > 0.0) || !cell.is_finite() {
            cell = 1.0;
        }
        out.cell = cell;
        out.nx = (((hi[0] - lo[0]) / cell).ceil() as usize).max(1);
        out.ny = (((hi[1] - lo[1]) / cell).ceil() as usize).max(1);
        // Each dab in every cell its box meets: counted, then filed.
        let cells = out.nx * out.ny;
        let mut start = vec![0u32; cells + 1];
        let each = |out: &Dabs, put: &mut dyn FnMut(usize, usize)| {
            for k in 0..n {
                let (du, dv, r) = (out.dabs[3 * k], out.dabs[3 * k + 1], out.dabs[3 * k + 2].abs());
                for cy in out.row(dv - r)..=out.row(dv + r) {
                    for cx in out.col(du - r)..=out.col(du + r) {
                        put(cy * out.nx + cx, k);
                    }
                }
            }
        };
        each(&out, &mut |c, _| start[c + 1] += 1);
        for c in 1..=cells {
            start[c] += start[c - 1];
        }
        let mut fill = start[..cells].to_vec();
        let mut order = vec![0u32; start[cells] as usize];
        each(&out, &mut |c, k| {
            order[fill[c] as usize] = k as u32;
            fill[c] += 1;
        });
        out.start = start;
        out.order = order;
        out.cuts = near.filter(|c| c.len() == n);
        out
    }

    #[inline]
    fn col(&self, u: f64) -> usize {
        (((u - self.lo[0]) / self.cell).floor().max(0.0) as usize).min(self.nx - 1)
    }

    #[inline]
    fn row(&self, v: f64) -> usize {
        (((v - self.lo[1]) / self.cell).floor().max(0.0) as usize).min(self.ny - 1)
    }

    #[inline]
    fn holds(&self, u: f64, v: f64, p: [f64; 3]) -> bool {
        if self.nx == 0 || !(u >= self.lo[0] && u <= self.hi[0] && v >= self.lo[1] && v <= self.hi[1]) {
            return false;
        }
        let c = self.row(v) * self.nx + self.col(u);
        for &k in &self.order[self.start[c] as usize..self.start[c + 1] as usize] {
            let k = k as usize;
            let (du, dv, r) = (u - self.dabs[3 * k], v - self.dabs[3 * k + 1], self.dabs[3 * k + 2]);
            if du * du + dv * dv > r * r {
                continue;
            }
            if let Some(cuts) = &self.cuts {
                if self.eye[0] * p[0] + self.eye[1] * p[1] + self.eye[2] * p[2] < cuts[k] {
                    continue;
                }
            }
            return true;
        }
        false
    }
}

/// A shape as the page gives it.
pub struct ShapeIn {
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub poly: Vec<f64>,
    pub op: Op,
    pub dabs: Vec<f64>,
    pub near: Option<Vec<f64>>,
    /// The classes off as it was drawn.
    pub off: Vec<u8>,
}

struct Shape {
    right: [f64; 3],
    up: [f64; 3],
    op: Op,
    polygon: Polygon,
    dabs: Option<Dabs>,
    off: Option<Box<[bool; 256]>>,
}

/// The shapes in turn, ready to test points by; `key` tells one list from another.
pub struct Shapes {
    pub key: String,
    shapes: Vec<Shape>,
}

impl Shapes {
    pub fn new(key: String, shapes: Vec<ShapeIn>) -> Shapes {
        let shapes = shapes
            .into_iter()
            .map(|s| {
                let off = (!s.off.is_empty()).then(|| {
                    let mut table = Box::new([false; 256]);
                    for c in &s.off {
                        table[*c as usize] = true;
                    }
                    table
                });
                let dabs = (!s.dabs.is_empty()).then(|| Dabs::new(s.dabs, s.near, s.right, s.up));
                Shape { right: s.right, up: s.up, op: s.op, polygon: Polygon::new(s.poly), dabs, off }
            })
            .collect();
        Shapes { key, shapes }
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Whether a place in the drawing, of class `class`, is kept (the page's shapesKeeper).
    #[inline]
    pub fn keeps(&self, p: [f64; 3], class: u8) -> bool {
        let Some(first) = self.shapes.first() else { return true };
        let mut kept = first.op != Op::Add;
        for s in &self.shapes {
            // Adding to a point kept, or narrowing or hiding one not: no change.
            if if s.op == Op::Add { kept } else { !kept } {
                continue;
            }
            let u = s.right[0] * p[0] + s.right[1] * p[1] + s.right[2] * p[2];
            let v = s.up[0] * p[0] + s.up[1] * p[1] + s.up[2] * p[2];
            let inside = !s.off.as_ref().is_some_and(|off| off[class as usize]) && s.polygon.holds(u, v) && s.dabs.as_ref().is_none_or(|d| d.holds(u, v, p));
            kept = if s.op == Op::Out { !inside } else { inside };
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(poly: &[f64], op: Op) -> ShapeIn {
        ShapeIn { right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], poly: poly.to_vec(), op, dabs: Vec::new(), near: None, off: Vec::new() }
    }

    const SQUARE: [f64; 8] = [0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0];

    #[test]
    fn shapes_in_turn_add_narrow_and_hide() {
        // None: every point.
        assert!(Shapes::new("".into(), vec![]).keeps([5.0, 5.0, 0.0], 2));
        // One: the points inside it, all the way through its view.
        let one = Shapes::new("a".into(), vec![plan(&SQUARE, Op::Add)]);
        assert!(one.keeps([5.0, 5.0, -100.0], 2) && one.keeps([5.0, 5.0, 900.0], 2) && !one.keeps([11.0, 5.0, 0.0], 2));
        // A second adds; "and" narrows to what's inside it; "out" takes its inside away.
        let l = [20.0, 0.0, 30.0, 0.0, 30.0, 10.0, 20.0, 10.0];
        let two = Shapes::new("b".into(), vec![plan(&SQUARE, Op::Add), plan(&l, Op::Add)]);
        assert!(two.keeps([5.0, 5.0, 0.0], 2) && two.keeps([25.0, 5.0, 0.0], 2) && !two.keeps([15.0, 5.0, 0.0], 2));
        let half = [0.0, 0.0, 5.0, 0.0, 5.0, 10.0, 0.0, 10.0];
        let narrowed = Shapes::new("c".into(), vec![plan(&SQUARE, Op::Add), plan(&l, Op::Add), plan(&half, Op::And)]);
        assert!(narrowed.keeps([2.0, 5.0, 0.0], 2) && !narrowed.keeps([7.0, 5.0, 0.0], 2) && !narrowed.keeps([25.0, 5.0, 0.0], 2));
        let holed = Shapes::new("d".into(), vec![plan(&SQUARE, Op::Add), plan(&half, Op::Out)]);
        assert!(!holed.keeps([2.0, 5.0, 0.0], 2) && holed.keeps([7.0, 5.0, 0.0], 2));
        // Starting with a hide: from every point.
        let hidden = Shapes::new("e".into(), vec![plan(&half, Op::Out)]);
        assert!(hidden.keeps([100.0, 100.0, 0.0], 2) && !hidden.keeps([2.0, 5.0, 0.0], 2));
        // Starting with a narrow: only what's inside it.
        let only = Shapes::new("f".into(), vec![plan(&half, Op::And)]);
        assert!(only.keeps([2.0, 5.0, 0.0], 2) && !only.keeps([7.0, 5.0, 0.0], 2));
    }

    #[test]
    fn a_shape_is_through_its_own_view_and_leaves_alone_the_classes_off_as_it_was_drawn() {
        // Drawn in a view from the south: across is east, up is Z; through it north and south.
        let front = ShapeIn { right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], ..plan(&SQUARE, Op::Add) };
        let s = Shapes::new("a".into(), vec![front]);
        assert!(s.keeps([5.0, -500.0, 5.0], 2) && s.keeps([5.0, 500.0, 5.0], 2) && !s.keeps([5.0, 0.0, 11.0], 2));
        // Classes 7 and 18 were off as it was drawn: never inside it (not isolated by it; never hidden by one that hides).
        let add = ShapeIn { off: vec![7, 18], ..plan(&SQUARE, Op::Add) };
        let s = Shapes::new("b".into(), vec![add]);
        assert!(s.keeps([5.0, 5.0, 0.0], 2) && !s.keeps([5.0, 5.0, 0.0], 7) && !s.keeps([5.0, 5.0, 0.0], 18));
        let out = ShapeIn { off: vec![7], ..plan(&SQUARE, Op::Out) };
        let s = Shapes::new("c".into(), vec![out]);
        assert!(!s.keeps([5.0, 5.0, 0.0], 2) && s.keeps([5.0, 5.0, 0.0], 7));
    }

    #[test]
    fn a_lasso_is_inside_by_its_edges_and_a_brushed_shape_by_its_dabs() {
        // A concave lasso (an L): the notch is outside.
        let l = [0.0, 0.0, 10.0, 0.0, 10.0, 4.0, 4.0, 4.0, 4.0, 10.0, 0.0, 10.0];
        let s = Shapes::new("a".into(), vec![plan(&l, Op::Add)]);
        assert!(s.keeps([2.0, 8.0, 0.0], 2) && s.keeps([8.0, 2.0, 0.0], 2) && !s.keeps([8.0, 8.0, 0.0], 2));
        // Many corners (a circle of 5000): in bands, the same answers.
        let round: Vec<f64> = (0..5000).flat_map(|k| { let a = k as f64 / 5000.0 * std::f64::consts::TAU; [50.0 + 10.0 * a.cos(), 50.0 + 10.0 * a.sin()] }).collect();
        let s = Shapes::new("b".into(), vec![plan(&round, Op::Add)]);
        for k in 0..200 {
            let a = k as f64 * 0.7;
            for (r, inside) in [(9.9, true), (10.1, false), (3.0, true), (0.0, true), (14.0, false)] {
                assert_eq!(s.keeps([50.0 + r * a.cos(), 50.0 + r * a.sin(), 3.0], 2), inside, "{r} at {a}");
            }
        }
        // Painted with the brush: the box round its dabs, and inside one of them.
        let brushed = ShapeIn { dabs: vec![2.0, 2.0, 1.0, 8.0, 8.0, 1.0], ..plan(&SQUARE, Op::Add) };
        let s = Shapes::new("c".into(), vec![brushed]);
        assert!(s.keeps([2.5, 2.0, 0.0], 2) && s.keeps([8.0, 7.2, 0.0], 2) && !s.keeps([5.0, 5.0, 0.0], 2) && !s.keeps([3.5, 2.0, 0.0], 2));
        // On a 3D view's front surface only: each dab's cut toward the eye (plan: the eye's up Z), points below it left alone.
        let front = ShapeIn { dabs: vec![2.0, 2.0, 1.0, 8.0, 8.0, 1.0], near: Some(vec![5.0, -1e9]), ..plan(&SQUARE, Op::Add) };
        let s = Shapes::new("d".into(), vec![front]);
        assert!(s.keeps([2.0, 2.0, 6.0], 2) && !s.keeps([2.0, 2.0, 4.0], 2) && s.keeps([8.0, 8.0, -50.0], 2));
        // A shape of under three corners holds nothing.
        let none = Shapes::new("e".into(), vec![plan(&[0.0, 0.0, 1.0, 1.0], Op::Add)]);
        assert!(!none.keeps([0.5, 0.5, 0.0], 2));
    }
}
