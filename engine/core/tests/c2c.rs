//! Cloud to cloud (1.22): lining scans up and measuring how far apart they are.

use cadp_engine_core::c2c::{align, check, compose, gap_stats, AlignOpts, IDENTITY, UNMEASURED};

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 as f64 / 4_294_967_296.0
    }
}

/// A floor, two walls and a box, points at random on them (feet), a little noise.
fn scene(n: usize, rng: &mut Rng) -> Vec<f64> {
    let mut out = Vec::with_capacity(n * 3);
    for _ in 0..n {
        let k = rng.next();
        let (x, y, z) = if k < 0.5 {
            (rng.next() * 40.0, rng.next() * 40.0, 100.0)
        } else if k < 0.7 {
            (0.0, rng.next() * 40.0, 100.0 + rng.next() * 10.0)
        } else if k < 0.9 {
            (rng.next() * 40.0, 0.0, 100.0 + rng.next() * 10.0)
        } else {
            (20.0, 15.0 + rng.next() * 5.0, 100.0 + rng.next() * 4.0)
        };
        out.extend_from_slice(&[x + (rng.next() - 0.5) * 0.01, y + (rng.next() - 0.5) * 0.01, z + (rng.next() - 0.5) * 0.01]);
    }
    out
}

fn moved(a: &[f64; 12], p: &[f64]) -> Vec<f64> {
    p.chunks_exact(3)
        .flat_map(|q| {
            [
                a[0] * q[0] + a[1] * q[1] + a[2] * q[2] + a[3],
                a[4] * q[0] + a[5] * q[1] + a[6] * q[2] + a[7],
                a[8] * q[0] + a[9] * q[1] + a[10] * q[2] + a[11],
            ]
        })
        .collect()
}

/// A turn about the vertical and a shift.
fn turn(deg: f64, t: [f64; 3]) -> [f64; 12] {
    let (s, c) = deg.to_radians().sin_cos();
    [c, -s, 0.0, t[0], s, c, 0.0, t[1], 0.0, 0.0, 1.0, t[2]]
}

/// The inverse of a rigid move.
fn inverse(a: &[f64; 12]) -> [f64; 12] {
    let mut out = [0.0; 12];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 4 + c] = a[c * 4 + r];
        }
    }
    for r in 0..3 {
        out[r * 4 + 3] = -(out[r * 4] * a[3] + out[r * 4 + 1] * a[7] + out[r * 4 + 2] * a[11]);
    }
    out
}

/// How far a found move leaves the scene's corners from where they belong.
fn miss(found: &[f64; 12], off: &[f64; 12]) -> f64 {
    let back = compose(found, off);
    [[0.0, 0.0, 100.0], [40.0, 40.0, 110.0], [20.0, 20.0, 104.0]]
        .iter()
        .map(|p| {
            let q = moved(&back, p);
            ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt()
        })
        .fold(0.0, f64::max)
}

#[test]
fn lines_a_scan_up_cloud_to_cloud_from_a_rough_fit() {
    let mut rng = Rng(11);
    let fixed = scene(60_000, &mut rng);
    let truth = turn(0.3, [0.3, -0.2, 0.06]);
    let off = inverse(&truth);
    let moving = moved(&off, &scene(60_000, &mut rng));
    // The picked points' fit, a few hundredths out.
    let fit = compose(&turn(0.02, [0.02, -0.015, 0.01]), &truth);
    for level in [false, true] {
        let a = align(&fixed, &moving, &AlignOpts { fit, fit_rms: 0.03, level, refine: true, foot: 1.0, tolerance: 0.05 });
        assert!(a.refined);
        assert!(miss(&fit, &off) > 0.02);
        assert!(miss(&a.align, &off) < 0.003, "level {level}: {}", miss(&a.align, &off));
        assert!(a.after.rms < a.before.rms && a.after.within > 0.95, "{:?}", a.after);
        // Floor and two walls hold every direction.
        assert!(a.degenerate.is_none(), "{:?}", a.degenerate);
    }
    // Without cloud to cloud: the fit as it is, measured.
    let a = align(&fixed, &moving, &AlignOpts { fit, fit_rms: 0.03, level: false, refine: false, foot: 1.0, tolerance: 0.05 });
    assert!(!a.refined);
    assert_eq!(a.align, fit);
}

#[test]
fn says_when_a_lone_floor_cant_hold_the_scans_in_plan() {
    let mut rng = Rng(5);
    let floor = |n: usize, rng: &mut Rng| -> Vec<f64> {
        (0..n).flat_map(|_| [rng.next() * 40.0, rng.next() * 40.0, 100.0 + (rng.next() - 0.5) * 0.01]).collect()
    };
    let fixed = floor(30_000, &mut rng);
    let moving = floor(30_000, &mut rng);
    let a = align(&fixed, &moving, &AlignOpts { fit: IDENTITY, fit_rms: 0.03, level: false, refine: true, foot: 1.0, tolerance: 0.05 });
    let (ratio, _along) = a.degenerate.expect("a floor alone says nothing about where along it");
    assert!(ratio < 1e-3);
}

#[test]
fn measures_scans_against_each_other_both_ways() {
    let mut rng = Rng(3);
    let a = scene(40_000, &mut rng);
    let b = scene(40_000, &mut rng);
    // A third scan far off meets neither.
    let far = moved(&turn(0.0, [500.0, 0.0, 0.0]), &scene(5_000, &mut rng));
    let c = check(&[a.clone(), b.clone(), far], 1.0, 0.05);
    assert_eq!(c.pairs.len(), 1);
    let p = &c.pairs[0];
    assert_eq!((p.i, p.j), (0, 1));
    assert_eq!(p.ab.len(), c.samples[0].len() / 3);
    assert_eq!(p.fab.len(), p.ab.len());
    let mut both = p.ab.clone();
    both.extend_from_slice(&p.ba);
    let s = gap_stats(&both, 0.05);
    assert!(s.overlap > 0.95 && s.within > 0.9 && s.rms < 0.02, "{s:?}");
    // On the floor and on the walls, each side.
    assert!(p.fab.iter().any(|f| *f == 2) && p.fab.iter().any(|f| *f == 1));
    // Every point's gap to the other; the far scan's not seen by any.
    assert_eq!(c.gaps[0].len(), a.len() / 3);
    let close = c.gaps[0].iter().filter(|g| g.is_finite() && **g >= 0.0 && **g < 0.05).count();
    assert!(close as f64 > 0.7 * c.gaps[0].len() as f64, "{close}");
    assert!(c.gaps[2].iter().all(|g| g.is_nan()));
}

#[test]
fn counts_gaps_as_the_page_does() {
    let s = gap_stats(&[0.01, 0.02, 0.2, f32::INFINITY, UNMEASURED, f32::NAN], 0.05);
    assert_eq!(s.checked, 6.0);
    assert!((s.overlap - 5.0 / 6.0).abs() < 1e-12);
    assert!((s.within - 2.0 / 4.0).abs() < 1e-12);
    assert!((s.alone - 1.0 / 4.0).abs() < 1e-12);
    assert!((s.p95 - 0.2f32 as f64).abs() < 1e-9);
    let none = gap_stats(&[], 0.05);
    assert_eq!(none.overlap, 0.0);
}
