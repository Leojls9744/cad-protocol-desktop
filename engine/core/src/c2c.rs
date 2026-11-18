//! Cloud to cloud (desktop 1.22): lining one scan up on another and
//! measuring how far apart they are, on every core. The same work as the
//! page's (web lib/editor/scan-align.ts: alignScans after the picked
//! points' fit, checkScans' gaps), given the same points, done the same way
//! so the answers agree:
//!
//! - `Grid`: points in cubes counted from the first point (PointGrid), each
//!   cube's points visited newest first, as the page's lists are.
//! - `Surfaces`: each point's surface normal from those within its radius
//!   (flat: the thinnest spread under a tenth of the next), each worked out
//!   the first time it's needed, kept as 32-bit floats as the page keeps
//!   them.
//! - `gaps_to`, `gap_stats`, `refine` (point-to-plane ICP: Huber then Tukey
//!   weights, the reach narrowing, a touch of damping, the direction the
//!   overlap barely holds), `align` and `check_gaps`.

use crate::fx::{FxMap, FxSet};
use rayon::prelude::*;
use std::sync::OnceLock;

/// A move, x' = R x + t, row by row with the shift after each (the page's ScanAlign).
pub type Align = [f64; 12];

pub const IDENTITY: Align = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// A gap where the other scan has points round a point but no surface to measure to.
pub const UNMEASURED: f32 = -1.0;

#[inline]
fn apply(a: &Align, x: f64, y: f64, z: f64) -> [f64; 3] {
    [
        a[0] * x + a[1] * y + a[2] * z + a[3],
        a[4] * x + a[5] * y + a[6] * z + a[7],
        a[8] * x + a[9] * y + a[10] * z + a[11],
    ]
}

/// `a` after `b`.
pub fn compose(a: &Align, b: &Align) -> Align {
    let mut out = [0.0; 12];
    for r in 0..3 {
        for c in 0..3 {
            let mut s = 0.0;
            for k in 0..3 {
                s += a[r * 4 + k] * b[k * 4 + c];
            }
            out[r * 4 + c] = s;
        }
        let mut t = a[r * 4 + 3];
        for k in 0..3 {
            t += a[r * 4 + k] * b[k * 4 + 3];
        }
        out[r * 4 + 3] = t;
    }
    out
}

fn key_of(ix: i64, iy: i64, iz: i64) -> i64 {
    (ix * 1_048_576 + iy) * 4096 + iz.clamp(0, 4095)
}

/// Points (x, y, z in a row) in cubes of `cell`, to find those near a point.
pub struct Grid {
    pub cell: f64,
    pub xyz: Vec<f64>,
    pub count: usize,
    x0: f64,
    y0: f64,
    z0: f64,
    /// Each cube's first place in `order`, and how many.
    cells: FxMap<i64, (u32, u32)>,
    /// Point indexes by cube, newest first within each.
    order: Vec<u32>,
}

impl Grid {
    pub fn new(xyz: Vec<f64>, cell: f64) -> Grid {
        let count = xyz.len() / 3;
        let (x0, y0, z0) = if count > 0 { (xyz[0], xyz[1], xyz[2]) } else { (0.0, 0.0, 0.0) };
        let mut g = Grid { cell, xyz, count, x0, y0, z0, cells: FxMap::default(), order: Vec::new() };
        let mut keyed: Vec<(i64, u32)> = (0..count)
            .map(|i| (g.key(g.xyz[i * 3], g.xyz[i * 3 + 1], g.xyz[i * 3 + 2]), i as u32))
            .collect();
        // By cube, the newest first (the page's lists).
        keyed.par_sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        g.order = keyed.iter().map(|k| k.1).collect();
        let mut i = 0;
        while i < keyed.len() {
            let k = keyed[i].0;
            let mut j = i;
            while j < keyed.len() && keyed[j].0 == k {
                j += 1;
            }
            g.cells.insert(k, (i as u32, (j - i) as u32));
            i = j;
        }
        g
    }

    #[inline]
    fn index(&self, x: f64, y: f64, z: f64) -> (i64, i64, i64) {
        (
            ((x - self.x0) / self.cell).floor() as i64 + 524_288,
            ((y - self.y0) / self.cell).floor() as i64 + 524_288,
            ((z - self.z0) / self.cell).floor() as i64 + 2048,
        )
    }

    #[inline]
    fn key(&self, x: f64, y: f64, z: f64) -> i64 {
        let (ix, iy, iz) = self.index(x, y, z);
        key_of(ix, iy, iz)
    }

    #[inline]
    fn cube(&self, k: i64) -> &[u32] {
        match self.cells.get(&k) {
            Some(&(s, n)) => &self.order[s as usize..(s + n) as usize],
            None => &[],
        }
    }

    #[inline]
    pub fn at(&self, i: usize) -> [f64; 3] {
        [self.xyz[i * 3], self.xyz[i * 3 + 1], self.xyz[i * 3 + 2]]
    }

    /// The nearest point within `reach`, or None.
    pub fn nearest(&self, x: f64, y: f64, z: f64, reach: f64) -> Option<usize> {
        let (ix, iy, iz) = self.index(x, y, z);
        let m = ((reach / self.cell).ceil() as i64).max(1);
        let mut best = None;
        let mut best_d = reach * reach;
        for dx in -m..=m {
            for dy in -m..=m {
                for dz in -m..=m {
                    for &i in self.cube(key_of(ix + dx, iy + dy, iz + dz)) {
                        let i = i as usize;
                        let ex = self.xyz[i * 3] - x;
                        let ey = self.xyz[i * 3 + 1] - y;
                        let ez = self.xyz[i * 3 + 2] - z;
                        let d = ex * ex + ey * ey + ez * ez;
                        if d <= best_d {
                            best_d = d;
                            best = Some(i);
                        }
                    }
                }
            }
        }
        best
    }

    /// Every point within `reach`, into `out`.
    pub fn near(&self, x: f64, y: f64, z: f64, reach: f64, out: &mut Vec<u32>) {
        out.clear();
        let (ix, iy, iz) = self.index(x, y, z);
        let m = ((reach / self.cell).ceil() as i64).max(1);
        let r2 = reach * reach;
        for dx in -m..=m {
            for dy in -m..=m {
                for dz in -m..=m {
                    for &i in self.cube(key_of(ix + dx, iy + dy, iz + dz)) {
                        let k = i as usize;
                        let ex = self.xyz[k * 3] - x;
                        let ey = self.xyz[k * 3 + 1] - y;
                        let ez = self.xyz[k * 3 + 2] - z;
                        if ex * ex + ey * ey + ez * ez <= r2 {
                            out.push(i);
                        }
                    }
                }
            }
        }
    }
}

/// Eigenvalues and vectors of a symmetric 3 × 3 (Jacobi, as the page's eigen3): vectors by column.
pub fn eigen3(m: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        if a[0][1].abs() + a[0][2].abs() + a[1][2].abs() < 1e-15 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-20 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = (if theta >= 0.0 { 1.0 } else { -1.0 }) / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let kp = a[k][p];
                let kq = a[k][q];
                a[k][p] = c * kp - s * kq;
                a[k][q] = s * kp + c * kq;
            }
            for k in 0..3 {
                let pk = a[p][k];
                let qk = a[q][k];
                a[p][k] = c * pk - s * qk;
                a[q][k] = s * pk + c * qk;
            }
            for row in v.iter_mut() {
                let kp = row[p];
                let kq = row[q];
                row[p] = c * kp - s * kq;
                row[q] = s * kp + c * kq;
            }
        }
    }
    let vectors = [[v[0][0], v[1][0], v[2][0]], [v[0][1], v[1][1], v[2][1]], [v[0][2], v[1][2], v[2][2]]];
    ([a[0][0], a[1][1], a[2][2]], vectors)
}

/// Eigenvalues and vectors (each a row here) of a symmetric n × n (Jacobi, as the page's eigenSym).
pub fn eigen_sym(m: &[Vec<f64>]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = m.len();
    let mut a: Vec<Vec<f64>> = m.to_vec();
    let mut v: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for _ in 0..100 {
        let mut off = 0.0;
        for p in 0..n {
            for q in p + 1..n {
                off += a[p][q].abs();
            }
        }
        if off < 1e-14 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = (if theta >= 0.0 { 1.0 } else { -1.0 }) / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let kp = row[p];
                    let kq = row[q];
                    row[p] = c * kp - s * kq;
                    row[q] = s * kp + c * kq;
                }
                for k in 0..n {
                    let pk = a[p][k];
                    let qk = a[q][k];
                    a[p][k] = c * pk - s * qk;
                    a[q][k] = s * pk + c * qk;
                }
                for row in v.iter_mut() {
                    let kp = row[p];
                    let kq = row[q];
                    row[p] = c * kp - s * kq;
                    row[q] = s * kp + c * kq;
                }
            }
        }
    }
    let values = (0..n).map(|i| a[i][i]).collect();
    let vectors = (0..n).map(|j| v.iter().map(|row| row[j]).collect()).collect();
    (values, vectors)
}

/// Solves A x = b (small, symmetric positive): None when it can't (as the page's).
fn solve(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut m: Vec<Vec<f64>> = a.iter().zip(b).map(|(row, bi)| {
        let mut r = row.clone();
        r.push(*bi);
        r
    }).collect();
    for c in 0..n {
        let mut best = c;
        for r in c + 1..n {
            if m[r][c].abs() > m[best][c].abs() {
                best = r;
            }
        }
        if m[best][c].abs() < 1e-18 {
            return None;
        }
        m.swap(c, best);
        for r in 0..n {
            if r == c {
                continue;
            }
            let f = m[r][c] / m[c][c];
            for k in c..=n {
                m[r][k] -= f * m[c][k];
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

/// A scan's surfaces to match against.
pub struct Surfaces {
    pub grid: Grid,
    pub radius: f64,
    /// Each point's normal (NaN off a surface), worked out the first time it's needed.
    normals: Vec<OnceLock<[f32; 3]>>,
    plan: FxSet<i64>,
}

impl Surfaces {
    pub fn new(xyz: Vec<f64>, radius: f64) -> Surfaces {
        let grid = Grid::new(xyz, radius);
        let count = grid.count;
        let mut s = Surfaces { grid, radius, normals: (0..count).map(|_| OnceLock::new()).collect(), plan: FxSet::default() };
        let plan: FxSet<i64> = (0..s.grid.count).map(|i| s.plan_key(s.grid.xyz[i * 3], s.grid.xyz[i * 3 + 1])).collect();
        s.plan = plan;
        s
    }

    fn plan_key(&self, x: f64, y: f64) -> i64 {
        let c = 2.0 * self.radius;
        let g = &self.grid;
        let (x0, y0) = if g.count > 0 { (g.xyz[0], g.xyz[1]) } else { (0.0, 0.0) };
        (((x - x0) / c).floor() as i64 + 1_048_576) * 2_097_152 + ((y - y0) / c).floor() as i64 + 1_048_576
    }

    /// Whether it saw the place in plan (points above or below it, and all round it).
    pub fn covers(&self, x: f64, y: f64) -> bool {
        let c = 2.0 * self.radius;
        for dx in -1..=1 {
            for dy in -1..=1 {
                if !self.plan.contains(&self.plan_key(x + dx as f64 * c, y + dy as f64 * c)) {
                    return false;
                }
            }
        }
        true
    }

    /// The surface's normal at point `i`, or None off a surface.
    #[inline]
    pub fn normal(&self, i: usize) -> Option<[f64; 3]> {
        let n = *self.normals[i].get_or_init(|| normal_at(&self.grid, self.radius, i, &mut Vec::new()));
        if n[0].is_nan() {
            None
        } else {
            Some([n[0] as f64, n[1] as f64, n[2] as f64])
        }
    }
}

fn normal_at(g: &Grid, radius: f64, i: usize, around: &mut Vec<u32>) -> [f32; 3] {
    let p = g.at(i);
    g.near(p[0], p[1], p[2], radius, around);
    let n = around.len();
    if n < 6 {
        return [f32::NAN; 3];
    }
    let nf = n as f64;
    let (mut mx, mut my, mut mz) = (0.0, 0.0, 0.0);
    for &j in around.iter() {
        let q = g.at(j as usize);
        mx += q[0] / nf;
        my += q[1] / nf;
        mz += q[2] / nf;
    }
    let mut c = [[0.0f64; 3]; 3];
    for &j in around.iter() {
        let q = g.at(j as usize);
        let d = [q[0] - mx, q[1] - my, q[2] - mz];
        for a in 0..3 {
            for b in 0..3 {
                c[a][b] += d[a] * d[b];
            }
        }
    }
    let (values, vectors) = eigen3(c);
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| values[*a].partial_cmp(&values[*b]).unwrap_or(std::cmp::Ordering::Equal));
    if values[order[0]] < 0.1 * values[order[1]] {
        let v = vectors[order[0]];
        let mut len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len == 0.0 {
            len = 1.0;
        }
        return [(v[0] / len) as f32, (v[1] / len) as f32, (v[2] / len) as f32];
    }
    [f32::NAN; 3]
}

/// At most one point a cube of `cell` (the first in each), as the page's voxelThin.
pub fn voxel_thin(xyz: &[f64], cell: f64) -> Vec<f64> {
    let n = xyz.len() / 3;
    if n == 0 {
        return Vec::new();
    }
    let (x0, y0, z0) = (xyz[0], xyz[1], xyz[2]);
    let mut seen: FxSet<i64> = FxSet::with_capacity_and_hasher(n.min(4_000_000), Default::default());
    let mut out = Vec::with_capacity(n * 3);
    for i in 0..n {
        let ix = ((xyz[i * 3] - x0) / cell).floor() as i64 + 524_288;
        let iy = ((xyz[i * 3 + 1] - y0) / cell).floor() as i64 + 524_288;
        let iz = (((xyz[i * 3 + 2] - z0) / cell).floor() as i64 + 2048).clamp(0, 4095);
        let key = (ix * 1_048_576 + iy) * 4096 + iz;
        if seen.insert(key) {
            out.extend_from_slice(&xyz[i * 3..i * 3 + 3]);
        }
    }
    out
}

/// Every `step`th point of a list, moved by `a` (as the page's thinned).
pub fn thinned(xyz: &[f64], max: f64, a: &Align) -> Vec<f64> {
    let n = xyz.len() / 3;
    let step = ((n as f64 / max.max(1.0)).ceil() as usize).max(1);
    let mut out = Vec::with_capacity((n / step + 1) * 3);
    let mut i = 0;
    while i < n {
        out.extend_from_slice(&apply(a, xyz[i * 3], xyz[i * 3 + 1], xyz[i * 3 + 2]));
        i += step;
    }
    out
}

/// How far apart points are, typically: the median distance to each one's nearest, from a sample.
pub fn spacing_of(xyz: &[f64], foot: f64) -> f64 {
    let n = xyz.len() / 3;
    if n < 2 {
        return 0.1 * foot;
    }
    let grid = Grid::new(xyz.to_vec(), foot);
    let step = (n / 1500).max(1);
    let mut around = Vec::new();
    let mut d = Vec::new();
    let mut i = 0;
    while i < n {
        let (x, y, z) = (xyz[i * 3], xyz[i * 3 + 1], xyz[i * 3 + 2]);
        grid.near(x, y, z, foot, &mut around);
        let mut best = f64::INFINITY;
        for &j in &around {
            let j = j as usize;
            if j == i {
                continue;
            }
            let e = ((xyz[j * 3] - x).powi(2) + (xyz[j * 3 + 1] - y).powi(2) + (xyz[j * 3 + 2] - z).powi(2)).sqrt();
            if e > 0.0 && e < best {
                best = e;
            }
        }
        d.push(if best.is_finite() { best } else { foot });
        i += step;
    }
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    d[d.len() / 2]
}

/// How far round a point its surface is read: three spacings, a third of a foot to two feet.
pub fn surface_radius(xyz: &[f64], foot: f64) -> f64 {
    (2.0 * foot).min((foot / 3.0).max(3.0 * spacing_of(xyz, foot)))
}

/// Each point's distance to the other scan's surface (as the page's gapsTo), and its surface's facing
/// (2 the ground or a floor, 1 a wall or pole, 0 none measured).
pub fn gaps_to(xyz: &[f64], other: &Surfaces, reach: f64) -> (Vec<f32>, Vec<u8>) {
    let n = xyz.len() / 3;
    let g = &other.grid;
    let local = reach.min(other.radius);
    let both: Vec<(f32, u8)> = (0..n)
        .into_par_iter()
        .map_init(Vec::new, |around: &mut Vec<u32>, i| {
            let (x, y, z) = (xyz[i * 3], xyz[i * 3 + 1], xyz[i * 3 + 2]);
            g.near(x, y, z, local, around);
            if around.is_empty() {
                let Some(j) = g.nearest(x, y, z, reach) else {
                    return (if other.covers(x, y) { f32::INFINITY } else { f32::NAN }, 0u8);
                };
                let q = g.at(j);
                let (ex, ey, ez) = (x - q[0], y - q[1], z - q[2]);
                return match other.normal(j) {
                    Some(nr) => ((ex * nr[0] + ey * nr[1] + ez * nr[2]).abs() as f32, if nr[2].abs() >= 0.5 { 2 } else { 1 }),
                    None => ((ex * ex + ey * ey + ez * ez).sqrt() as f32, 0),
                };
            }
            let mut best: f64 = UNMEASURED as f64;
            let mut measured = false;
            let mut flat = false;
            for &j in around.iter() {
                let j = j as usize;
                let Some(nr) = other.normal(j) else { continue };
                let q = g.at(j);
                let d = ((x - q[0]) * nr[0] + (y - q[1]) * nr[1] + (z - q[2]) * nr[2]).abs();
                if !measured || d < best {
                    best = d;
                    measured = true;
                    flat = nr[2].abs() >= 0.5;
                }
            }
            if !measured {
                (UNMEASURED, 0)
            } else {
                (best as f32, if flat { 2 } else { 1 })
            }
        })
        .collect();
    both.into_iter().unzip()
}

/// The overlap's figures from each point's gap (as the page's gapStats).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GapStats {
    pub checked: f64,
    pub overlap: f64,
    pub mean: f64,
    pub rms: f64,
    pub p95: f64,
    pub within: f64,
    pub alone: f64,
}

pub fn gap_stats(gaps: &[f32], tolerance: f64) -> GapStats {
    let mut hit: Vec<f64> = Vec::new();
    let (mut alone, mut unmeasured) = (0usize, 0usize);
    for &g in gaps {
        if g == f32::INFINITY {
            alone += 1;
        } else if g == UNMEASURED {
            unmeasured += 1;
        } else if !g.is_nan() {
            hit.push(g as f64);
        }
    }
    let n = hit.len();
    let seen = n + alone;
    let overlap = if gaps.is_empty() { 0.0 } else { (seen + unmeasured) as f64 / gaps.len() as f64 };
    if n == 0 {
        return GapStats {
            checked: gaps.len() as f64,
            overlap,
            alone: if seen > 0 { alone as f64 / seen as f64 } else { 0.0 },
            ..GapStats::default()
        };
    }
    hit.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (mut sum, mut sq, mut ok) = (0.0, 0.0, 0usize);
    for &g in &hit {
        sum += g;
        sq += g * g;
        if g <= tolerance {
            ok += 1;
        }
    }
    GapStats {
        checked: gaps.len() as f64,
        overlap,
        mean: sum / n as f64,
        rms: (sq / n as f64).sqrt(),
        p95: hit[(n - 1).min((0.95 * n as f64).floor() as usize)],
        within: ok as f64 / seen as f64,
        alone: alone as f64 / seen as f64,
    }
}

fn turn_about(theta: f64, about: [f64; 3], shift: [f64; 3]) -> Align {
    let (c, s) = (theta.cos(), theta.sin());
    [
        c,
        -s,
        0.0,
        about[0] - c * about[0] + s * about[1] + shift[0],
        s,
        c,
        0.0,
        about[1] - s * about[0] - c * about[1] + shift[1],
        0.0,
        0.0,
        1.0,
        shift[2],
    ]
}

fn small_turn(w: [f64; 3], about: [f64; 3], shift: [f64; 3]) -> Align {
    let angle = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    let mut r = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    if angle > 0.0 {
        let (kx, ky, kz) = (w[0] / angle, w[1] / angle, w[2] / angle);
        let (c, s) = (angle.cos(), angle.sin());
        let v = 1.0 - c;
        r = [
            c + kx * kx * v,
            kx * ky * v - kz * s,
            kx * kz * v + ky * s,
            ky * kx * v + kz * s,
            c + ky * ky * v,
            ky * kz * v - kx * s,
            kz * kx * v - ky * s,
            kz * ky * v + kx * s,
            c + kz * kz * v,
        ];
    }
    let mut out = [0.0; 12];
    for row in 0..3 {
        let mut ra = 0.0;
        for k in 0..3 {
            out[row * 4 + k] = r[row * 3 + k];
            ra += r[row * 3 + k] * about[k];
        }
        out[row * 4 + 3] = about[row] - ra + shift[row];
    }
    out
}

/// How the moving scan is brought onto the held one's surfaces.
#[derive(Clone, Copy, Debug)]
pub struct RefineOpts {
    pub level: bool,
    pub reach: f64,
    pub final_reach: f64,
    pub tolerance: f64,
    pub iterations: usize,
}

#[derive(Clone, Debug)]
pub struct Refined {
    pub align: Align,
    pub before: GapStats,
    pub after: GapStats,
    pub iterations: usize,
    /// The direction the overlap barely holds: its information's ratio, and which unknown.
    pub degenerate: Option<(f64, usize)>,
}

/// Point-to-plane ICP, as the page's refine.
pub fn refine(moving: &[f64], fixed: &Surfaces, o: &RefineOpts) -> Refined {
    let n = moving.len() / 3;
    let before = gap_stats(&gaps_to(moving, fixed, o.final_reach).0, o.tolerance);
    let mut align = IDENTITY;
    let unknowns = if o.level { 4 } else { 6 };
    let mut done = 0;
    let g = &fixed.grid;
    let mut last_a: Option<Vec<Vec<f64>>> = None;
    let mut last_reach = 1.0;
    let half = (o.iterations as f64 / 2.0).max(1.0);
    for it in 0..o.iterations {
        done = it + 1;
        let reach = o.final_reach.max(o.reach * (o.final_reach / o.reach).powf(it as f64 / half));
        let cur = align;
        // Each moving point's nearest held point within reach, on a surface.
        let pts: Vec<([f64; 3], [f64; 3], [f64; 3])> = (0..n)
            .into_par_iter()
            .filter_map(|i| {
                let q = apply(&cur, moving[i * 3], moving[i * 3 + 1], moving[i * 3 + 2]);
                let j = g.nearest(q[0], q[1], q[2], reach)?;
                let nr = fixed.normal(j)?;
                Some((q, g.at(j), nr))
            })
            .collect();
        if pts.len() < unknowns * 3 {
            break;
        }
        let m = pts.len() as f64;
        let mut c = [0.0; 3];
        for (q, _, _) in &pts {
            c[0] += q[0] / m;
            c[1] += q[1] / m;
            c[2] += q[2] / m;
        }
        let rs: Vec<f64> = pts
            .iter()
            .map(|(q, f, nr)| (q[0] - f[0]) * nr[0] + (q[1] - f[1]) * nr[1] + (q[2] - f[2]) * nr[2])
            .collect();
        let mut sorted = rs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = sorted[sorted.len() >> 1];
        let mut dev: Vec<f64> = rs.iter().map(|v| (v - mid).abs()).collect();
        dev.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let tukey = reach <= 2.0 * o.final_reach;
        let ct = 4.685 * (1.4826 * dev[dev.len() >> 1]).max(o.final_reach / 20.0);
        let mut a = vec![vec![0.0; unknowns]; unknowns];
        let mut b = vec![0.0; unknowns];
        for (k, (q, _f, nr)) in pts.iter().enumerate() {
            let (qx, qy, qz) = (q[0] - c[0], q[1] - c[1], q[2] - c[2]);
            let r = rs[k];
            let kk = reach / 10.0;
            let w = if tukey {
                if r.abs() < ct { (1.0 - (r / ct).powi(2)).powi(2) } else { 0.0 }
            } else if r.abs() <= kk {
                1.0
            } else {
                kk / r.abs()
            };
            let j: Vec<f64> = if o.level {
                vec![-qy * nr[0] + qx * nr[1], nr[0], nr[1], nr[2]]
            } else {
                vec![qy * nr[2] - qz * nr[1], qz * nr[0] - qx * nr[2], qx * nr[1] - qy * nr[0], nr[0], nr[1], nr[2]]
            };
            for x in 0..unknowns {
                b[x] -= w * j[x] * r;
                for y in 0..unknowns {
                    a[x][y] += w * j[x] * j[y];
                }
            }
        }
        last_a = Some(a.clone());
        let spread: f64 = pts
            .iter()
            .map(|(q, _, _)| (q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2) + (q[2] - c[2]).powi(2))
            .sum::<f64>()
            / m;
        last_reach = spread.sqrt().max(1e-9);
        let trace: f64 = (0..unknowns).map(|x| a[x][x]).sum();
        for x in 0..unknowns {
            a[x][x] += 1e-6 * (trace / unknowns as f64);
        }
        let Some(x) = solve(&a, &b) else { break };
        let step = if o.level {
            turn_about(x[0], c, [x[1], x[2], x[3]])
        } else {
            small_turn([x[0], x[1], x[2]], c, [x[3], x[4], x[5]])
        };
        align = compose(&step, &align);
        let moved = if o.level {
            (x[1] * x[1] + x[2] * x[2] + x[3] * x[3]).sqrt() + x[0].abs() * 100.0
        } else {
            (x[3] * x[3] + x[4] * x[4] + x[5] * x[5]).sqrt() + (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() * 100.0
        };
        if reach <= o.final_reach && moved < o.final_reach * 1e-4 {
            break;
        }
    }
    let after = gap_stats(&gaps_to(&thinned(moving, n as f64, &align), fixed, o.final_reach).0, o.tolerance);
    let mut degenerate = None;
    if let Some(la) = last_a {
        let turns = if o.level { 1 } else { 3 };
        let d: Vec<f64> = (0..unknowns).map(|x| if x < turns { 1.0 / last_reach } else { 1.0 }).collect();
        let s: Vec<Vec<f64>> = la.iter().enumerate().map(|(x, row)| row.iter().enumerate().map(|(y, v)| v * d[x] * d[y]).collect()).collect();
        let (values, vectors) = eigen_sym(&s);
        let hi = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let lo = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let k = values.iter().position(|v| *v == lo).unwrap_or(0);
        let ratio = if hi > 0.0 { values[k] / hi } else { 0.0 };
        if hi > 0.0 && ratio < 1e-3 {
            let v: Vec<f64> = vectors[k].iter().map(|x| x.abs()).collect();
            let top = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            degenerate = Some((ratio, v.iter().position(|x| *x == top).unwrap_or(0)));
        }
    }
    Refined { align, before, after, iterations: done, degenerate }
}

/// What's asked of `align`: the picked points' fit, and how.
#[derive(Clone, Copy, Debug)]
pub struct AlignOpts {
    pub fit: Align,
    pub fit_rms: f64,
    pub level: bool,
    pub refine: bool,
    pub foot: f64,
    pub tolerance: f64,
}

#[derive(Clone, Debug)]
pub struct Aligned {
    /// The whole move: the fit, then cloud to cloud.
    pub align: Align,
    pub before: GapStats,
    pub after: GapStats,
    pub iterations: usize,
    pub degenerate: Option<(f64, usize)>,
    pub refined: bool,
}

/// The moving scan lined up with the held one after the picked points' fit (as the page's alignScans).
pub fn align(fixed: &[f64], moving: &[f64], o: &AlignOpts) -> Aligned {
    let ft = o.foot;
    let held = thinned(&voxel_thin(fixed, 0.1 * ft), 800_000.0, &IDENTITY);
    let radius = surface_radius(&held, ft);
    let surfaces = Surfaces::new(held, radius);
    let sample = thinned(&voxel_thin(moving, 0.3 * ft), 20_000.0, &IDENTITY);
    let final_reach = 0.25 * ft;
    let before = gap_stats(&gaps_to(&sample, &surfaces, final_reach).0, o.tolerance);
    let moved = thinned(&sample, f64::INFINITY, &o.fit);
    if !o.refine {
        let after = gap_stats(&gaps_to(&moved, &surfaces, final_reach).0, o.tolerance);
        return Aligned { align: o.fit, before, after, iterations: 0, degenerate: None, refined: false };
    }
    let r = refine(
        &moved,
        &surfaces,
        &RefineOpts {
            level: o.level,
            reach: (0.5 * ft).max((3.0 * ft).min(3.0 * o.fit_rms)),
            final_reach,
            tolerance: o.tolerance,
            iterations: 30,
        },
    );
    Aligned {
        align: compose(&r.align, &o.fit),
        before,
        after: r.after,
        iterations: r.iterations,
        degenerate: r.degenerate,
        refined: true,
    }
}

/// Two scans that meet, measured both ways on their samples.
#[derive(Clone, Debug)]
pub struct PairGaps {
    pub i: usize,
    pub j: usize,
    pub ab: Vec<f32>,
    pub fab: Vec<u8>,
    pub ba: Vec<f32>,
    pub fba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Checked {
    /// Each scan's sample (evened out, a quarter foot), as the pairs were measured on.
    pub samples: Vec<Vec<f64>>,
    pub pairs: Vec<PairGaps>,
    /// Each scan's every point's gap to the nearest of the others' surfaces (NaN: not seen by any).
    pub gaps: Vec<Vec<f32>>,
}

fn rank(g: f32) -> u8 {
    if g.is_nan() {
        0
    } else if g == f32::INFINITY {
        1
    } else if g == UNMEASURED {
        2
    } else {
        3
    }
}

/// Every pair of scans that meet measured both ways, and each scan's points' gaps (checkScans' work).
pub fn check(scans: &[Vec<f64>], foot: f64, tolerance: f64) -> Checked {
    let ft = foot;
    let reach = (0.25 * ft).max(4.0 * tolerance);
    let boxes: Vec<([f64; 3], [f64; 3])> = scans
        .iter()
        .map(|p| {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for (i, v) in p.iter().enumerate() {
                let k = i % 3;
                lo[k] = lo[k].min(*v);
                hi[k] = hi[k].max(*v);
            }
            (lo, hi)
        })
        .collect();
    let meets = |i: usize, j: usize| (0..3).all(|k| boxes[i].0[k] <= boxes[j].1[k] + reach && boxes[j].0[k] <= boxes[i].1[k] + reach);
    let surfaces: Vec<Surfaces> = scans
        .iter()
        .map(|p| {
            let kept = thinned(&voxel_thin(p, 0.1 * ft), 800_000.0, &IDENTITY);
            let r = surface_radius(&kept, ft);
            Surfaces::new(kept, r)
        })
        .collect();
    let samples: Vec<Vec<f64>> = scans.iter().map(|p| thinned(&voxel_thin(p, 0.25 * ft), 100_000.0, &IDENTITY)).collect();
    let mut pairs = Vec::new();
    for i in 0..scans.len() {
        for j in i + 1..scans.len() {
            if !meets(i, j) {
                continue;
            }
            let (ab, fab) = gaps_to(&samples[i], &surfaces[j], reach);
            let (ba, fba) = gaps_to(&samples[j], &surfaces[i], reach);
            pairs.push(PairGaps { i, j, ab, fab, ba, fba });
        }
    }
    let gaps = scans
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = p.len() / 3;
            let mut out = vec![f32::NAN; n];
            for j in 0..scans.len() {
                if j == i || !meets(i, j) {
                    continue;
                }
                let (g, _) = gaps_to(p, &surfaces[j], reach);
                for k in 0..n {
                    let (v, was) = (g[k], out[k]);
                    let (rv, rw) = (rank(v), rank(was));
                    if rv > rw || (rv == 3 && v < was) {
                        out[k] = v;
                    }
                }
            }
            out
        })
        .collect();
    Checked { samples, pairs, gaps }
}

#[cfg(test)]
mod timing {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn where_the_time_goes() {
        let mut seed: u32 = 7;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed as f64 / 4_294_967_296.0
        };
        let mut pts = Vec::new();
        for _ in 0..200_000 {
            let k = rnd();
            let (x, y, z) = if k < 0.6 { (rnd() * 60.0, rnd() * 60.0, 100.0) } else { (0.0, rnd() * 60.0, 100.0 + rnd() * 12.0) };
            pts.extend_from_slice(&[x + (rnd() - 0.5) * 0.01, y + (rnd() - 0.5) * 0.01, z + (rnd() - 0.5) * 0.01]);
        }
        let t = Instant::now();
        let held = thinned(&voxel_thin(&pts, 0.1), 800_000.0, &IDENTITY);
        eprintln!("voxel {:?} ({} kept)", t.elapsed(), held.len() / 3);
        let t = Instant::now();
        let r = surface_radius(&held, 1.0);
        eprintln!("radius {:?} ({r})", t.elapsed());
        let t = Instant::now();
        let s = Surfaces::new(held.clone(), r);
        eprintln!("surfaces {:?}", t.elapsed());
        let sample = thinned(&voxel_thin(&pts, 0.3), 20_000.0, &IDENTITY);
        let t = Instant::now();
        let _ = gaps_to(&sample, &s, 0.25);
        eprintln!("gaps {:?} ({} sampled)", t.elapsed(), sample.len() / 3);
        let t = Instant::now();
        let out = refine(&sample, &s, &RefineOpts { level: false, reach: 0.5, final_reach: 0.25, tolerance: 0.05, iterations: 30 });
        eprintln!("refine {:?} ({} iterations)", t.elapsed(), out.iterations);
    }
}
