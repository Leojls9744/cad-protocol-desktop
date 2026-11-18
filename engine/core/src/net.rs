//! Geo W., the trained scan classifier (desktop 1.23), on every core: the
//! page's own work (web lib/editor/scan-net.ts and scan-normals.ts) ported
//! as it is, so the same points and model give the same classes:
//!
//! - the scan's points (feet) in cubes of 0.25 ft, then 1 ft and 4 ft made
//!   from those (their counts weighing them), grouped in the order numpy's
//!   unique sorts their keys (`group_by_keys`);
//! - each cube's height above the low point of its 10 ft and 40 ft plan
//!   cells (the cube 2% up), over 10;
//! - each finest cube's 24, 16 and 16 nearest cubes at the three scales,
//!   within 1, 4 and 16 ft (the nearest repeated where fewer), as the
//!   model's inputs; a version 2 model's extra inputs (returns, and each
//!   cube's surface: normal, curvature, the break to its neighbours') after;
//! - the model: each scale's small network over each neighbour, the
//!   largest of each output over the neighbours, then the head; each cube's
//!   best class and its softmax share, every point its cube's.
//!
//! The page works in 64-bit numbers and keeps the inputs and each layer's
//! outputs in 32: so does this.

use crate::fx::FxMap;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Each scale: its cubes' size, the radius and how many neighbours (feet).
pub const SCALES: [(f64, f64, usize); 3] = [(0.25, 1.0, 24), (1.0, 4.0, 16), (4.0, 16.0, 16)];
pub const CELLS: [f64; 2] = [10.0, 40.0];
pub const LOW_SHARE: f64 = 0.02;
/// Version 1's numbers a neighbour, each scale.
pub const CHANNELS: [usize; 3] = [8, 4, 4];

/// One layer: `out` × `inp` weights row by row at `w` in the blob, `out` biases at `b`.
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub inp: usize,
    pub out: usize,
    pub w: usize,
    pub b: usize,
}

/// The model: its three branches, its head, its classes and what a version 2 model reads.
#[derive(Clone, Debug)]
pub struct Model {
    pub branches: [Vec<Layer>; 3],
    pub head: Vec<Layer>,
    pub classes: usize,
    pub returns: bool,
    pub normal: bool,
    pub curvature: bool,
    pub brk: bool,
    pub blob: Vec<f32>,
}

impl Model {
    fn more(&self) -> usize {
        (if self.returns { 2 } else { 0 }) + (if self.normal { 3 } else { 0 }) + (self.curvature as usize) + (self.brk as usize)
    }

    /// How many numbers a neighbour at each scale.
    pub fn channels(&self) -> [usize; 3] {
        let m = self.more();
        [CHANNELS[0] + m, CHANNELS[1] + m, CHANNELS[2] + m]
    }

    fn shaped(&self) -> bool {
        self.normal || self.curvature || self.brk
    }

    /// Whether the layers fit together and the blob holds them (as the page's parseNet checks).
    pub fn check(&self) -> Result<(), String> {
        let floats = self.blob.len();
        let fit = |layers: &[Layer], cin: usize| -> Result<usize, String> {
            let mut c = cin;
            for l in layers {
                if l.inp != c || l.w + l.inp * l.out > floats || l.b + l.out > floats {
                    return Err("the model's layers don't fit together".into());
                }
                c = l.out;
            }
            Ok(c)
        };
        let ch = self.channels();
        let mut pooled = 0;
        for (k, br) in self.branches.iter().enumerate() {
            if br.is_empty() {
                return Err("a branch has no layers".into());
            }
            pooled += fit(br, ch[k])?;
        }
        if fit(&self.head, pooled + CELLS.len())? != self.classes || self.classes == 0 || self.classes > 255 {
            return Err("the model's last layer doesn't give each class a score".into());
        }
        Ok(())
    }
}

/// Cubes: each one's mean place (feet), its points' count, mean intensity (0–1) and colour (0–255).
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub n: usize,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub count: Vec<f64>,
    pub intensity: Vec<f64>,
    pub rgb: Option<Vec<f64>>,
}

/// Items grouped by their cube keys, in the keys' order (as numpy's unique sorts them): how many, each item's group.
fn group_by_keys(kx: &[f64], ky: &[f64], kz: &[f64]) -> (usize, Vec<i32>) {
    let m = kx.len();
    let (mut x0, mut y0, mut z0) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let (mut y1, mut z1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for i in 0..m {
        x0 = x0.min(kx[i]);
        y0 = y0.min(ky[i]);
        z0 = z0.min(kz[i]);
        y1 = y1.max(ky[i]);
        z1 = z1.max(kz[i]);
    }
    let ny = y1 - y0 + 1.0;
    let nz = z1 - z0 + 1.0;
    let keys: Vec<f64> = (0..m).map(|i| ((kx[i] - x0) * ny + (ky[i] - y0)) * nz + (kz[i] - z0)).collect();
    let mut sorted = keys.clone();
    sorted.par_sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    sorted.dedup();
    let of = keys
        .par_iter()
        .map(|k| sorted.partition_point(|v| v < k) as i32)
        .collect();
    (sorted.len(), of)
}

/// Points in cubes `size` on a side: the cubes, and each point's cube (as the page's netCubes).
pub fn cubes(px: &[f64], py: &[f64], pz: &[f64], intensity: &[f64], rgb: Option<&[f64]>, size: f64) -> (Level, Vec<i32>) {
    let m = px.len();
    let kx: Vec<f64> = px.iter().map(|v| (v / size).floor()).collect();
    let ky: Vec<f64> = py.iter().map(|v| (v / size).floor()).collect();
    let kz: Vec<f64> = pz.iter().map(|v| (v / size).floor()).collect();
    let (n, of) = group_by_keys(&kx, &ky, &kz);
    let mut l = Level {
        n,
        x: vec![0.0; n],
        y: vec![0.0; n],
        z: vec![0.0; n],
        count: vec![0.0; n],
        intensity: vec![0.0; n],
        rgb: rgb.map(|_| vec![0.0; n * 3]),
    };
    for i in 0..m {
        let c = of[i] as usize;
        l.x[c] += px[i];
        l.y[c] += py[i];
        l.z[c] += pz[i];
        l.count[c] += 1.0;
        l.intensity[c] += intensity[i];
        if let (Some(src), Some(dst)) = (rgb, l.rgb.as_mut()) {
            for a in 0..3 {
                dst[c * 3 + a] += src[i * 3 + a];
            }
        }
    }
    for c in 0..n {
        let w = l.count[c];
        l.x[c] /= w;
        l.y[c] /= w;
        l.z[c] /= w;
        l.intensity[c] /= w;
        if let Some(rgb) = l.rgb.as_mut() {
            for a in 0..3 {
                rgb[c * 3 + a] /= w;
            }
        }
    }
    (l, of)
}

/// Cubes `size` on a side made from finer ones, their counts weighing them; and each finer cube's cube.
pub fn coarser(level: &Level, size: f64) -> (Level, Vec<i32>) {
    let m = level.n;
    let kx: Vec<f64> = level.x.iter().map(|v| (v / size).floor()).collect();
    let ky: Vec<f64> = level.y.iter().map(|v| (v / size).floor()).collect();
    let kz: Vec<f64> = level.z.iter().map(|v| (v / size).floor()).collect();
    let (n, of) = group_by_keys(&kx, &ky, &kz);
    let mut out = Level { n, x: vec![0.0; n], y: vec![0.0; n], z: vec![0.0; n], count: vec![0.0; n], intensity: vec![0.0; n], rgb: None };
    for i in 0..m {
        let c = of[i] as usize;
        let w = level.count[i];
        out.x[c] += level.x[i] * w;
        out.y[c] += level.y[i] * w;
        out.z[c] += level.z[i] * w;
        out.intensity[c] += level.intensity[i] * w;
        out.count[c] += w;
    }
    for c in 0..n {
        let w = out.count[c];
        out.x[c] /= w;
        out.y[c] /= w;
        out.z[c] /= w;
        out.intensity[c] /= w;
    }
    (out, of)
}

/// Each cube's height above the low point of its 10 ft and 40 ft plan cells, over 10: [n × 2].
pub fn lows(level: &Level) -> Vec<f32> {
    let n = level.n;
    let mut out = vec![0f32; n * CELLS.len()];
    let zero = vec![0.0; n];
    for (j, cell) in CELLS.iter().enumerate() {
        let kx: Vec<f64> = level.x.iter().map(|v| (v / cell).floor()).collect();
        let ky: Vec<f64> = level.y.iter().map(|v| (v / cell).floor()).collect();
        let (cells, of) = group_by_keys(&kx, &ky, &zero);
        let mut start = vec![0usize; cells + 1];
        for &c in &of {
            start[c as usize + 1] += 1;
        }
        for c in 0..cells {
            start[c + 1] += start[c];
        }
        let mut at = start[..cells].to_vec();
        let mut zs = vec![0.0; n];
        for i in 0..n {
            let c = of[i] as usize;
            zs[at[c]] = level.z[i];
            at[c] += 1;
        }
        let low: Vec<f64> = (0..cells)
            .map(|c| {
                let part = &mut zs[start[c]..start[c + 1]].to_vec();
                part.sort_by(|a, b| a.partial_cmp(b).unwrap());
                part[(LOW_SHARE * part.len() as f64).floor() as usize]
            })
            .collect();
        for i in 0..n {
            out[i * CELLS.len() + j] = ((level.z[i] - low[of[i] as usize]) / 10.0) as f32;
        }
    }
    out
}

/// A level's cubes in a grid of cells `cell` on a side, for finding neighbours (the page's scan-net Grid).
struct Grid {
    cells: FxMap<i64, (u32, u32)>,
    order: Vec<u32>,
    x0: f64,
    y0: f64,
    z0: f64,
    ny: f64,
    nz: f64,
    cell: f64,
}

impl Grid {
    fn new(l: &Level, cell: f64) -> Grid {
        let (mut x0, mut y0, mut z0) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let (mut y1, mut z1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for i in 0..l.n {
            x0 = x0.min(l.x[i]);
            y0 = y0.min(l.y[i]);
            z0 = z0.min(l.z[i]);
            y1 = y1.max(l.y[i]);
            z1 = z1.max(l.z[i]);
        }
        let x0 = x0 - cell;
        let y0 = y0 - cell;
        let z0 = z0 - cell;
        let ny = ((y1 - y0) / cell).floor() + 3.0;
        let nz = ((z1 - z0) / cell).floor() + 3.0;
        let mut g = Grid { cells: FxMap::default(), order: Vec::new(), x0, y0, z0, ny, nz, cell };
        let key: Vec<f64> = (0..l.n).map(|i| g.key_at(l.x[i], l.y[i], l.z[i])).collect();
        let mut order: Vec<u32> = (0..l.n as u32).collect();
        // Stable, as the page's sort: equal keys keep their order.
        order.par_sort_by(|a, b| key[*a as usize].partial_cmp(&key[*b as usize]).unwrap());
        let mut s = 0usize;
        for i in 1..=l.n {
            if i == l.n || key[order[i] as usize] != key[order[s] as usize] {
                g.cells.insert(key[order[s] as usize] as i64, (s as u32, i as u32));
                s = i;
            }
        }
        g.order = order;
        g
    }

    #[inline]
    fn key_at(&self, x: f64, y: f64, z: f64) -> f64 {
        let c = self.cell;
        (((x - self.x0) / c).floor() * self.ny + ((y - self.y0) / c).floor()) * self.nz + ((z - self.z0) / c).floor()
    }

    /// The `k` nearest cubes within `radius` of (x, y, z), nearest first, into `idx` (the nearest repeated where fewer).
    #[allow(clippy::too_many_arguments)]
    fn nearest(&self, l: &Level, x: f64, y: f64, z: f64, radius: f64, k: usize, idx: &mut [u32], dist: &mut [f64]) {
        let c = self.cell;
        let ix = ((x - self.x0) / c).floor();
        let iy = ((y - self.y0) / c).floor();
        let iz = ((z - self.z0) / c).floor();
        let r2 = radius * radius;
        let mut found = 0usize;
        for a in [ix - 1.0, ix, ix + 1.0] {
            for b in [iy - 1.0, iy, iy + 1.0] {
                for e in [iz - 1.0, iz, iz + 1.0] {
                    let Some(&(s0, s1)) = self.cells.get(&(((a * self.ny + b) * self.nz + e) as i64)) else { continue };
                    for s in s0..s1 {
                        let j = self.order[s as usize];
                        let ju = j as usize;
                        let dx = l.x[ju] - x;
                        let dy = l.y[ju] - y;
                        let dz = l.z[ju] - z;
                        let d = dx * dx + dy * dy + dz * dz;
                        if d > r2 {
                            continue;
                        }
                        if found == k && d >= dist[k - 1] {
                            continue;
                        }
                        let mut p = if found < k { found } else { k - 1 };
                        while p > 0 && dist[p - 1] > d {
                            dist[p] = dist[p - 1];
                            idx[p] = idx[p - 1];
                            p -= 1;
                        }
                        dist[p] = d;
                        idx[p] = j;
                        if found < k {
                            found += 1;
                        }
                    }
                }
            }
        }
        for p in found..k {
            idx[p] = idx[0];
        }
    }
}

/// Each group's normal, curvature and normal-angle break (the page's groupShapes).
#[derive(Clone, Debug, Default)]
pub struct Shapes {
    pub nx: Vec<f32>,
    pub ny: Vec<f32>,
    pub nz: Vec<f32>,
    pub curvature: Vec<f32>,
    pub break_deg: Vec<f32>,
}

const SWEEPS: usize = 32;
const LINE: f64 = 1e-8;

/// One Jacobi turn of j zeroing its (p, q) entry, the eigenvectors (j[9..18], by column) turned with it.
fn rotate(j: &mut [f64; 18], p: usize, q: usize) {
    let apq = j[p * 3 + q];
    if apq == 0.0 {
        return;
    }
    let app = j[p * 4];
    let aqq = j[q * 4];
    let theta = (aqq - app) / (2.0 * apq);
    let t = (if theta >= 0.0 { 1.0 } else { -1.0 }) / (theta.abs() + (theta * theta + 1.0).sqrt());
    let c = 1.0 / (t * t + 1.0).sqrt();
    let s = t * c;
    let tau = s / (1.0 + c);
    j[p * 4] = app - t * apq;
    j[q * 4] = aqq + t * apq;
    j[p * 3 + q] = 0.0;
    j[q * 3 + p] = 0.0;
    let r = 3 - p - q;
    let arp = j[r * 3 + p];
    let arq = j[r * 3 + q];
    let rp = arp - s * (arq + tau * arp);
    let rq = arq + s * (arp - tau * arq);
    j[r * 3 + p] = rp;
    j[p * 3 + r] = rp;
    j[r * 3 + q] = rq;
    j[q * 3 + r] = rq;
    let mut i = 9;
    while i < 18 {
        let vp = j[i + p];
        let vq = j[i + q];
        j[i + p] = vp - s * (vq + tau * vp);
        j[i + q] = vq + s * (vp - tau * vq);
        i += 3;
    }
}

/// Each group's surface from its points (`of`: each point's group; `n` groups; their whole-step keys).
#[allow(clippy::too_many_arguments)]
pub fn group_shapes(x: &[f64], y: &[f64], z: &[f64], of: &[i32], n: usize, kx: &[f64], ky: &[f64], kz: &[f64]) -> Shapes {
    const W: usize = 10;
    let mut m = vec![0.0f64; n * W];
    for i in 0..of.len() {
        let g = of[i];
        if g < 0 || g as usize >= n {
            continue;
        }
        let s = g as usize * W;
        let c = m[s] + 1.0;
        m[s] = c;
        let (px, py, pz) = (x[i], y[i], z[i]);
        let dx = px - m[s + 1];
        let dy = py - m[s + 2];
        let dz = pz - m[s + 3];
        let mx = m[s + 1] + dx / c;
        let my = m[s + 2] + dy / c;
        let mz = m[s + 3] + dz / c;
        m[s + 1] = mx;
        m[s + 2] = my;
        m[s + 3] = mz;
        let (ex, ey, ez) = (px - mx, py - my, pz - mz);
        m[s + 4] += dx * ex;
        m[s + 5] += dx * ey;
        m[s + 6] += dx * ez;
        m[s + 7] += dy * ey;
        m[s + 8] += dy * ez;
        m[s + 9] += dz * ez;
    }
    let per: Vec<(f32, f32, f32, f32, bool)> = (0..n)
        .into_par_iter()
        .map(|g| {
            let s = g * W;
            let (mut ax, mut ay, mut az) = (0.0, 0.0, 1.0);
            let mut curved = 0.0;
            let mut flat = false;
            if m[s] >= 3.0 {
                let mut j = [0.0f64; 18];
                j[0] = m[s + 4];
                j[1] = m[s + 5];
                j[2] = m[s + 6];
                j[3] = m[s + 5];
                j[4] = m[s + 7];
                j[5] = m[s + 8];
                j[6] = m[s + 6];
                j[7] = m[s + 8];
                j[8] = m[s + 9];
                j[9] = 1.0;
                j[13] = 1.0;
                j[17] = 1.0;
                for _ in 0..SWEEPS {
                    let off = j[1] * j[1] + j[2] * j[2] + j[5] * j[5];
                    let on = j[0] * j[0] + j[4] * j[4] + j[8] * j[8];
                    if off <= 1e-30 * on {
                        break;
                    }
                    rotate(&mut j, 0, 1);
                    rotate(&mut j, 0, 2);
                    rotate(&mut j, 1, 2);
                }
                let l0 = j[0].max(0.0);
                let l1 = j[4].max(0.0);
                let l2 = j[8].max(0.0);
                let sum = l0 + l1 + l2;
                let (mut k, mut least) = (0usize, l0);
                if l1 < least {
                    k = 1;
                    least = l1;
                }
                if l2 < least {
                    k = 2;
                    least = l2;
                }
                let most = l0.max(l1).max(l2);
                let middle = sum - most - least;
                if sum > 0.0 && middle > LINE * most {
                    let vx = j[9 + k];
                    let vy = j[12 + k];
                    let vz = j[15 + k];
                    let len = (vx * vx + vy * vy + vz * vz).sqrt();
                    let sign = if vz < 0.0 { -1.0 / len } else { 1.0 / len };
                    ax = vx * sign;
                    ay = vy * sign;
                    az = vz * sign;
                    curved = least / sum;
                    flat = true;
                }
            }
            (ax as f32, ay as f32, az as f32, curved as f32, flat)
        })
        .collect();
    let mut out = Shapes {
        nx: per.iter().map(|p| p.0).collect(),
        ny: per.iter().map(|p| p.1).collect(),
        nz: per.iter().map(|p| p.2).collect(),
        curvature: per.iter().map(|p| p.3).collect(),
        break_deg: vec![0.0; n],
    };
    // The groups with a plane by their keys.
    let mut by_key: FxMap<(i64, i64, i64), u32> = FxMap::default();
    for g in 0..n {
        if per[g].4 {
            by_key.insert((kx[g] as i64, ky[g] as i64, kz[g] as i64), g as u32);
        }
    }
    let breaks: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|g| {
            if !per[g].4 {
                return 0.0;
            }
            let (ax, ay, az) = (out.nx[g] as f64, out.ny[g] as f64, out.nz[g] as f64);
            let (gx, gy, gz) = (kx[g] as i64, ky[g] as i64, kz[g] as i64);
            let (mut sine, mut cosine) = (0.0f64, 1.0f64);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(&o) = by_key.get(&(gx + dx, gy + dy, gz + dz)) else { continue };
                        let o = o as usize;
                        if o == g {
                            continue;
                        }
                        let (bx, by, bz) = (out.nx[o] as f64, out.ny[o] as f64, out.nz[o] as f64);
                        let cx = ay * bz - az * by;
                        let cy = az * bx - ax * bz;
                        let cz = ax * by - ay * bx;
                        let sin = (cx * cx + cy * cy + cz * cz).sqrt();
                        let cos = (ax * bx + ay * by + az * bz).abs();
                        if sin * cosine > sine * cos {
                            sine = sin;
                            cosine = cos;
                        }
                    }
                }
            }
            (sine.atan2(cosine) * 180.0 / std::f64::consts::PI) as f32
        })
        .collect();
    out.break_deg = breaks;
    out
}

/// The points' returns as LAS keeps them (1-based; 0 where a point has none).
pub struct Returns<'a> {
    pub number: &'a [u8],
    pub of: &'a [u8],
}

/// A scan made ready for the model.
pub struct Scene {
    pub levels: Vec<Level>,
    pub lows: Vec<f32>,
    pub of: Vec<i32>,
    grids: Vec<Grid>,
    /// Each level's returns (two a cube) and shapes, only as the model reads them.
    returns: Vec<Option<Vec<f32>>>,
    shapes: Vec<Option<Shapes>>,
}

impl Scene {
    #[allow(clippy::too_many_arguments)]
    pub fn new(model: &Model, px: &[f64], py: &[f64], pz: &[f64], intensity: &[f64], rgb: Option<&[f64]>, returns: Option<Returns>) -> Scene {
        let (level, of) = cubes(px, py, pz, intensity, rgb, SCALES[0].0);
        let mut levels = vec![level];
        let mut ups: Vec<Vec<i32>> = Vec::new();
        for s in 1..SCALES.len() {
            let (up, up_of) = coarser(&levels[0], SCALES[s].0);
            levels.push(up);
            ups.push(up_of);
        }
        let m = px.len();
        let counted = model.returns && returns.is_some();
        let shaped = model.shaped();
        let mut rets = Vec::new();
        let mut shapes = Vec::new();
        for (s, lv) in levels.iter().enumerate() {
            let n = lv.n;
            let size = SCALES[s].0;
            // Each point's cube at this scale.
            let at: Vec<i32> = if s == 0 { of.clone() } else { of.iter().map(|c| ups[s - 1][*c as usize]).collect() };
            let mean = match (&returns, counted) {
                (Some(r), true) => {
                    let mut sum = vec![0.0f64; n * 2];
                    for i in 0..m {
                        let c = at[i] as usize * 2;
                        let (rn, k) = (r.number[i], r.of[i]);
                        if rn > 0 && k > 0 {
                            sum[c] += if rn >= k { 1.0 } else { rn as f64 / k as f64 };
                            if rn >= k {
                                sum[c + 1] += 1.0;
                            }
                        } else {
                            sum[c] += 0.5;
                        }
                    }
                    let mut mean = vec![0f32; n * 2];
                    for c in 0..n {
                        mean[c * 2] = (sum[c * 2] / lv.count[c]) as f32;
                        mean[c * 2 + 1] = (sum[c * 2 + 1] / lv.count[c]) as f32;
                    }
                    Some(mean)
                }
                _ => None,
            };
            rets.push(mean);
            shapes.push(if shaped {
                let (mut kx, mut ky, mut kz) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
                if s == 0 {
                    for i in 0..m {
                        let c = of[i] as usize;
                        kx[c] = (px[i] / size).floor();
                        ky[c] = (py[i] / size).floor();
                        kz[c] = (pz[i] / size).floor();
                    }
                } else {
                    let fine = &levels[0];
                    let up = &ups[s - 1];
                    for f in 0..fine.n {
                        let c = up[f] as usize;
                        kx[c] = (fine.x[f] / size).floor();
                        ky[c] = (fine.y[f] / size).floor();
                        kz[c] = (fine.z[f] / size).floor();
                    }
                }
                Some(group_shapes(px, py, pz, &at, n, &kx, &ky, &kz))
            } else {
                None
            });
        }
        let grids = levels.iter().enumerate().map(|(s, lv)| Grid::new(lv, SCALES[s].1)).collect();
        let lows = lows(&levels[0]);
        Scene { levels, lows, of, grids, returns: rets, shapes }
    }

    pub fn cubes(&self) -> usize {
        self.levels[0].n
    }

    /// The model's inputs for cubes `from` to `to` − 1: each scale's neighbours, then the heights.
    pub fn inputs(&self, model: &Model, from: usize, to: usize) -> Vec<Vec<f32>> {
        let n = to - from;
        let w = &self.levels[0];
        let ch = model.channels();
        let mut out: Vec<Vec<f32>> = (0..SCALES.len()).map(|s| vec![0f32; n * SCALES[s].2 * ch[s]]).collect();
        let mut idx = [0u32; 32];
        let mut dist = [0f64; 32];
        let more = model.more() > 0;
        for s in 0..SCALES.len() {
            let (_, radius, k) = SCALES[s];
            let lv = &self.levels[s];
            let c = ch[s];
            let x = &mut out[s];
            let mean = self.returns[s].as_ref();
            let sh = self.shapes[s].as_ref();
            for i in 0..n {
                let cube = from + i;
                let (cx, cy, cz) = (w.x[cube], w.y[cube], w.z[cube]);
                self.grids[s].nearest(lv, cx, cy, cz, radius, k, &mut idx, &mut dist);
                for q in 0..k {
                    let j = idx[q] as usize;
                    let at = (i * k + q) * c;
                    x[at] = ((lv.x[j] - cx) / radius) as f32;
                    x[at + 1] = ((lv.y[j] - cy) / radius) as f32;
                    x[at + 2] = ((lv.z[j] - cz) / radius) as f32;
                    x[at + 3] = lv.intensity[j] as f32;
                    if s == 0 {
                        if let Some(rgb) = lv.rgb.as_ref() {
                            x[at + 4] = (rgb[j * 3] / 255.0) as f32;
                            x[at + 5] = (rgb[j * 3 + 1] / 255.0) as f32;
                            x[at + 6] = (rgb[j * 3 + 2] / 255.0) as f32;
                            x[at + 7] = 1.0;
                        }
                    }
                    if more {
                        let mut e = at + CHANNELS[s];
                        if model.returns {
                            x[e] = mean.map(|m| m[j * 2]).unwrap_or(0.5);
                            x[e + 1] = mean.map(|m| m[j * 2 + 1]).unwrap_or(0.0);
                            e += 2;
                        }
                        if let Some(sh) = sh {
                            if model.normal {
                                x[e] = sh.nx[j];
                                x[e + 1] = sh.ny[j];
                                x[e + 2] = sh.nz[j];
                                e += 3;
                            }
                            if model.curvature {
                                x[e] = sh.curvature[j];
                                e += 1;
                            }
                            if model.brk {
                                x[e] = ((sh.break_deg[j] as f64) / 90.0) as f32;
                            }
                        }
                    }
                }
            }
        }
        out.push(self.lows[from * CELLS.len()..to * CELLS.len()].to_vec());
        out
    }
}

/// `rows` rows of `l.inp` through a layer (ReLU after unless `last`), summed in 64 bits as the page does.
fn dense(blob: &[f32], l: &Layer, x: &[f32], rows: usize, last: bool) -> Vec<f32> {
    let (nin, nout) = (l.inp, l.out);
    let mut y = vec![0f32; rows * nout];
    for r in 0..rows {
        let xi = r * nin;
        let yi = r * nout;
        for o in 0..nout {
            let mut s = blob[l.b + o] as f64;
            let wi = l.w + o * nin;
            for c in 0..nin {
                s += blob[wi + c] as f64 * x[xi + c] as f64;
            }
            y[yi + o] = if last || s > 0.0 { s as f32 } else { 0.0 };
        }
    }
    y
}

/// Each class's score for `n` cubes' inputs.
pub fn forward(model: &Model, inputs: &[Vec<f32>], n: usize) -> Vec<f32> {
    let pooled: usize = model.branches.iter().map(|b| b[b.len() - 1].out).sum();
    let width = pooled + CELLS.len();
    let mut joined = vec![0f32; n * width];
    let mut off = 0;
    for (s, br) in model.branches.iter().enumerate() {
        let k = SCALES[s].2;
        let mut y = inputs[s].clone();
        for l in br {
            y = dense(&model.blob, l, &y, n * k, false);
        }
        let w = br[br.len() - 1].out;
        for i in 0..n {
            for o in 0..w {
                let mut m = f32::NEG_INFINITY;
                for q in 0..k {
                    let v = y[(i * k + q) * w + o];
                    if v > m {
                        m = v;
                    }
                }
                joined[i * width + off + o] = m;
            }
        }
        off += w;
    }
    let lows = &inputs[3];
    for i in 0..n {
        for j in 0..CELLS.len() {
            joined[i * width + off + j] = lows[i * CELLS.len() + j];
        }
    }
    let mut y = joined;
    for (i, l) in model.head.iter().enumerate() {
        y = dense(&model.blob, l, &y, n, i == model.head.len() - 1);
    }
    y
}

/// What's been classified of the run in hand, 0 to 1 (a 1000th each).
pub static PROGRESS: AtomicU32 = AtomicU32::new(0);

pub fn progress() -> f64 {
    PROGRESS.load(Ordering::Relaxed) as f64 / 1000.0
}

/// Each point's class (an index into the model's classes) and how sure (the class's softmax share), and how many cubes.
#[allow(clippy::too_many_arguments)]
pub fn classify(model: &Model, px: &[f64], py: &[f64], pz: &[f64], intensity: &[f64], rgb: Option<&[f64]>, returns: Option<Returns>, batch: usize) -> (Vec<u8>, Vec<f32>, usize) {
    PROGRESS.store(0, Ordering::Relaxed);
    let scene = Scene::new(model, px, py, pz, intensity, rgb, returns);
    PROGRESS.store(100, Ordering::Relaxed);
    let n = scene.cubes();
    let c = model.classes;
    let batches: Vec<usize> = (0..n).step_by(batch.max(1)).collect();
    let done = AtomicU32::new(0);
    let total = batches.len().max(1) as u32;
    let parts: Vec<(Vec<u8>, Vec<f32>)> = batches
        .par_iter()
        .map(|&from| {
            let to = (from + batch).min(n);
            let logits = forward(model, &scene.inputs(model, from, to), to - from);
            let mut best = vec![0u8; to - from];
            let mut share = vec![0f32; to - from];
            for i in 0..to - from {
                let (mut top, mut max) = (0usize, f32::NEG_INFINITY);
                for k in 0..c {
                    if logits[i * c + k] > max {
                        max = logits[i * c + k];
                        top = k;
                    }
                }
                let mut sum = 0.0f64;
                for k in 0..c {
                    sum += (logits[i * c + k] as f64 - max as f64).exp();
                }
                best[i] = top as u8;
                share[i] = (1.0 / sum) as f32;
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            PROGRESS.store(100 + 900 * d / total, Ordering::Relaxed);
            (best, share)
        })
        .collect();
    let mut best = Vec::with_capacity(n);
    let mut share = Vec::with_capacity(n);
    for (b, s) in parts {
        best.extend(b);
        share.extend(s);
    }
    let cls = scene.of.iter().map(|o| best[*o as usize]).collect();
    let sure = scene.of.iter().map(|o| share[*o as usize]).collect();
    PROGRESS.store(1000, Ordering::Relaxed);
    (cls, sure, n)
}
