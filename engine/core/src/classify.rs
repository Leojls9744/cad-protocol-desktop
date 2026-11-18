//! Auto-classify in the engine (1.29; Leo, 2026-11-18 06:04: "lets build
//! all 8 of you recommendations to make a true desktop app"): the heavy
//! middle of the page's rules classifier (scan-classify.ts classifyScans),
//! ported as it is so its classes are the page's, point for point, on every
//! core.
//!
//! The page works out the ground first (each cell's lowest point, filtered
//! and filled in) and the ground's colour; the engine then does what took
//! the page most of its time: every point's height above the ground, the
//! points on it (ground, or noise under it), the voxels of what stands
//! above, each voxel's surface (net.rs group_shapes), its neighbourhood's
//! shape and the forest's votes (rows on every core), a roof's edge taken
//! back from the trees, leafy green, leaves that aren't a building, a
//! fence's voxels under a building's wall, and the whole objects (each
//! class's touching voxels, poles and wires linked farther). The page goes
//! on from there with the objects' rules, as before.
//!
//! Everything is worked out in the page's order and arithmetic (its cells
//! looked through in the same order, its sums in the same order, its
//! Math.hypot as V8 has it, its single-precision stores), so the page's
//! rules see the same voxels.

use crate::c2c::eigen3;
use crate::fx::FxMap;
use crate::net::{group_shapes, Shapes};
use rayon::prelude::*;

/// The forest's classes, by index (scan-forest.ts FOREST_CLASSES).
pub const GROUND: i8 = 0;
pub const VEG: i8 = 1;
pub const BUILDING: i8 = 2;
pub const POLE: i8 = 3;
pub const WIRE: i8 = 4;
pub const FENCE: i8 = 5;
pub const OTHER: i8 = 6;
const CLASSES: usize = 7;

/// The ASPRS codes the points on the ground take.
const CODE_GROUND: u8 = 2;
const CODE_LOW_NOISE: u8 = 7;

/// scan-classify.ts's constants.
const GREEN_OVER_GROUND: f64 = 0.06;
const VEG_FLOOR_FT: f64 = 1.0;
const SURFACE: f64 = 0.08;
const LEAVES_SHARE: f64 = 0.65;
const LEAVES_MIDDLE: f64 = 0.1;
const FENCE_PLANE: f64 = 0.75;
const FENCE_ABOVE: f64 = 10.0;

/// Math.hypot as V8 works it out: scaled by the largest, a compensated sum
/// (checked against V8 on millions of values), so a shape's numbers are the
/// page's to the last bit.
pub fn js_hypot(v: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &x in v {
        if x.is_nan() {
            nan = true;
        } else {
            let a = x.abs();
            if a > max {
                max = a;
            }
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut comp = 0.0f64;
    for &x in v {
        let n = x.abs() / max;
        let summand = n * n - comp;
        let pre = sum + summand;
        comp = (pre - sum) - summand;
        sum = pre;
    }
    sum.sqrt() * max
}

/// JS's Math.max of two numbers (no NaN here).
#[inline]
fn jmax(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}
#[inline]
fn jmin(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

/// The page's Cells (scan-classify-rules.ts): indices filed by cell, each
/// cell's in the order added; `near` gives the 3 × 3 (× 3) cells round a
/// place in the same order the page does.
pub struct Cells {
    size: f64,
    flat: bool,
    map: FxMap<i64, Vec<u32>>,
}

impl Cells {
    pub fn new(size: f64, flat: bool) -> Cells {
        Cells { size, flat, map: FxMap::default() }
    }
    #[inline]
    fn key(&self, cx: i64, cy: i64, cz: i64) -> i64 {
        ((cx + 32768) * 65536 + (cy + 32768)) * (if self.flat { 1 } else { 65536 }) + (if self.flat { 0 } else { cz + 32768 })
    }
    pub fn add(&mut self, i: u32, x: f64, y: f64, z: f64) {
        let k = self.key((x / self.size).floor() as i64, (y / self.size).floor() as i64, (z / self.size).floor() as i64);
        self.map.entry(k).or_default().push(i);
    }
    /// Every index in the cells round (x, y, z), its own included, into `out`.
    pub fn near(&self, x: f64, y: f64, z: f64, out: &mut Vec<u32>) {
        out.clear();
        let cx = (x / self.size).floor() as i64;
        let cy = (y / self.size).floor() as i64;
        let cz = (z / self.size).floor() as i64;
        let (z0, z1) = if self.flat { (0, 0) } else { (-1, 1) };
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in z0..=z1 {
                    if let Some(list) = self.map.get(&self.key(cx + dx, cy + dy, cz + dz)) {
                        out.extend_from_slice(list);
                    }
                }
            }
        }
    }
}

/// A scan's points, as the page keeps them.
pub struct CloudIn<'a> {
    pub origin: [f64; 3],
    pub xyz: &'a [f32],
    pub rgb: Option<&'a [u8]>,
}

/// One tree of the forest (scan-forest.ts): node i's feature (-2 a leaf), threshold, left, right and class.
pub struct Tree {
    pub f: Vec<i32>,
    pub t: Vec<f64>,
    pub l: Vec<i32>,
    pub r: Vec<i32>,
    pub c: Vec<i32>,
}

/// What the page worked out first, and what the stage needs.
pub struct StageAsk<'a> {
    pub clouds: Vec<CloudIn<'a>>,
    pub x0: f64,
    pub y0: f64,
    pub y1: f64,
    pub z_min: f64,
    pub z_max: f64,
    /// Feet in one unit, the neighbourhood's reach, the voxel's side, the ground's rise.
    pub k: f64,
    pub r: f64,
    pub v: f64,
    pub rise: f64,
    /// The ground grid (NaN: none), its columns, rows and cell.
    pub gz: &'a [f64],
    pub cols: usize,
    pub rows: usize,
    pub g: f64,
    pub ground_green: f64,
    pub coloured: bool,
    pub trees: &'a [Tree],
}

/// What the stage found, for the page's rules.
pub struct Stage {
    /// Per scan: each point's code when it's on (or under) the ground (0: not), its voxel (-1: none) and height above the ground.
    pub ground: Vec<Vec<u8>>,
    pub vox: Vec<Vec<i32>>,
    pub hags: Vec<Vec<f32>>,
    pub n: usize,
    pub cx: Vec<f64>,
    pub cy: Vec<f64>,
    pub cz: Vec<f64>,
    pub ch: Vec<f64>,
    pub sn: Vec<f64>,
    pub sg: Vec<f64>,
    pub shapes: Shapes,
    pub label: Vec<i8>,
    /// Each voxel's choices, most votes first, CLASSES each (-1 past its last).
    pub choices: Vec<i8>,
    pub share: Vec<f32>,
    pub forest_top: Vec<i8>,
    pub scatter: Vec<f32>,
    pub flat: Vec<f32>,
    pub face_x: Vec<f32>,
    pub face_y: Vec<f32>,
    /// The whole objects: their members one after another, and where each starts.
    pub members: Vec<u32>,
    pub starts: Vec<u32>,
}

/// The ground's height under (x, y), from the grid, as the page's groundAt.
fn ground_at(a: &StageAsk, x: f64, y: f64) -> f64 {
    let cols = a.cols as f64;
    let rows = a.rows as f64;
    let fx = jmin(cols - 1.0, jmax(0.0, (x - a.x0) / a.g - 0.5));
    let fy = jmin(rows - 1.0, jmax(0.0, (y - a.y0) / a.g - 0.5));
    let c0 = fx.floor();
    let r0 = fy.floor();
    let c1 = jmin(cols - 1.0, c0 + 1.0);
    let r1 = jmin(rows - 1.0, r0 + 1.0);
    let tx = fx - c0;
    let ty = fy - r0;
    let z = |c: f64, r: f64| {
        let val = a.gz[r as usize * a.cols + c as usize];
        if val.is_nan() {
            0.0
        } else {
            val
        }
    };
    (z(c0, r0) * (1.0 - tx) + z(c1, r0) * tx) * (1.0 - ty) + (z(c0, r1) * (1.0 - tx) + z(c1, r1) * tx) * ty
}

/// How green a colour is (scan-classify.ts greenOf).
fn green_of(r: f64, g: f64, b: f64) -> f64 {
    let sum = r + g + b;
    if sum > 0.0 {
        (2.0 * g - r - b) / sum
    } else {
        0.0
    }
}

/// The forest's votes: its classes with votes, most first (ties: the lower class), and the share for the first.
fn vote(trees: &[Tree], x: &[f64; 10]) -> (Vec<i8>, f32) {
    let mut votes = [0u32; CLASSES];
    let mut count = 0u32;
    for t in trees {
        let mut i = 0usize;
        while t.f[i] >= 0 {
            i = if x[t.f[i] as usize] <= t.t[i] { t.l[i] as usize } else { t.r[i] as usize };
        }
        votes[t.c[i] as usize] += 1;
        count += 1;
    }
    let mut order: Vec<usize> = (0..CLASSES).filter(|&c| votes[c] > 0).collect();
    order.sort_by(|&a, &b| votes[b].cmp(&votes[a]).then(a.cmp(&b)));
    let share = if order.is_empty() { 0.0 } else { votes[order[0]] as f64 / (count.max(1)) as f64 };
    (order.into_iter().map(|c| c as i8).collect(), share as f32)
}

/// The class put first among a voxel's choices, the rest after in their order.
fn put_first(choices: &mut [i8], j: usize, first: i8) {
    let row = &mut choices[j * CLASSES..(j + 1) * CLASSES];
    let rest: Vec<i8> = row.iter().copied().filter(|&c| c >= 0 && c != first).collect();
    row.fill(-1);
    row[0] = first;
    for (k, c) in rest.into_iter().enumerate() {
        if k + 1 < CLASSES {
            row[k + 1] = c;
        }
    }
}

/// The stage, as scan-classify.ts classifyScans does it from its voxels to its whole objects.
pub fn stage(a: &StageAsk) -> Stage {
    let k = a.k;
    let r = a.r;
    let v = a.v;
    // ── Voxels of what stands above the ground ──
    let ny = ((a.y1 - a.y0) / v).floor() as i64 + 1;
    let nz = ((a.z_max - a.z_min) / v).floor() as i64 + 1;
    let mut index: FxMap<i64, u32> = FxMap::default();
    let (mut sx, mut sy, mut sz, mut sh, mut sn, mut sg) = (vec![], vec![], vec![], vec![], vec![], vec![]);
    let (mut key_x, mut key_y, mut key_z): (Vec<f64>, Vec<f64>, Vec<f64>) = (vec![], vec![], vec![]);
    let mut ground_out = Vec::with_capacity(a.clouds.len());
    let mut vox_out = Vec::with_capacity(a.clouds.len());
    let mut hag_out = Vec::with_capacity(a.clouds.len());
    for d in &a.clouds {
        let count = d.xyz.len() / 3;
        // Each point's height above the ground, on every core.
        let lifted: Vec<(f64, f64, f64, f64)> = (0..count)
            .into_par_iter()
            .map(|i| {
                let x = d.origin[0] + d.xyz[i * 3] as f64;
                let y = d.origin[1] + d.xyz[i * 3 + 1] as f64;
                let z = d.origin[2] + d.xyz[i * 3 + 2] as f64;
                (x, y, z, z - ground_at(a, x, y))
            })
            .collect();
        let mut vox = vec![-1i32; count];
        let mut hags = vec![0f32; count];
        let mut ground = vec![0u8; count];
        for (i, &(x, y, z, h)) in lifted.iter().enumerate() {
            hags[i] = h as f32;
            if h <= a.rise {
                ground[i] = if h < -3.0 * a.rise { CODE_LOW_NOISE } else { CODE_GROUND };
                continue;
            }
            let ix = ((x - a.x0) / v).floor() as i64;
            let iy = ((y - a.y0) / v).floor() as i64;
            let iz = ((z - a.z_min) / v).floor() as i64;
            let kk = (ix * ny + iy) * nz + iz;
            let j = match index.get(&kk) {
                Some(&j) => j as usize,
                None => {
                    let j = sn.len();
                    index.insert(kk, j as u32);
                    key_x.push(ix as f64);
                    key_y.push(iy as f64);
                    key_z.push(iz as f64);
                    sx.push(0.0);
                    sy.push(0.0);
                    sz.push(0.0);
                    sh.push(0.0);
                    sn.push(0.0);
                    sg.push(0.0);
                    j
                }
            };
            if let Some(rgb) = d.rgb {
                if green_of(rgb[i * 3] as f64, rgb[i * 3 + 1] as f64, rgb[i * 3 + 2] as f64) - a.ground_green > GREEN_OVER_GROUND {
                    sg[j] += 1.0;
                }
            }
            sx[j] += x - a.x0;
            sy[j] += y - a.y0;
            sz[j] += z;
            sh[j] += h;
            sn[j] += 1.0;
            vox[i] = j as i32;
        }
        ground_out.push(ground);
        vox_out.push(vox);
        hag_out.push(hags);
    }
    let n = sn.len();
    let cx: Vec<f64> = (0..n).map(|j| sx[j] / sn[j]).collect();
    let cy: Vec<f64> = (0..n).map(|j| sy[j] / sn[j]).collect();
    let cz: Vec<f64> = (0..n).map(|j| sz[j] / sn[j]).collect();
    let ch: Vec<f64> = (0..n).map(|j| sh[j] / sn[j]).collect();
    let mut cells3 = Cells::new(r, false);
    let mut cells2 = Cells::new(r, true);
    for j in 0..n {
        cells3.add(j as u32, cx[j], cy[j], cz[j]);
        cells2.add(j as u32, cx[j], cy[j], 0.0);
    }

    // Each voxel's surface (voxelShapes): the points from the first scan's
    // origin, in single precision as the page keeps them.
    let shapes = {
        let o = a.clouds.first().map(|d| d.origin).unwrap_or([0.0; 3]);
        let (mut px, mut py, mut pz, mut of) = (vec![], vec![], vec![], vec![]);
        for (c, d) in a.clouds.iter().enumerate() {
            let vox = &vox_out[c];
            let (ox, oy, oz) = (d.origin[0] - o[0], d.origin[1] - o[1], d.origin[2] - o[2]);
            for (i, &j) in vox.iter().enumerate() {
                if j < 0 {
                    continue;
                }
                px.push((ox + d.xyz[i * 3] as f64) as f32 as f64);
                py.push((oy + d.xyz[i * 3 + 1] as f64) as f32 as f64);
                pz.push((oz + d.xyz[i * 3 + 2] as f64) as f32 as f64);
                of.push(j);
            }
        }
        group_shapes(&px, &py, &pz, &of, n, &key_x, &key_y, &key_z)
    };

    // ── Features, and the forest's votes (each voxel on its own: every core) ──
    struct Voted {
        order: Vec<i8>,
        share: f32,
        scatter: f32,
        flat: f32,
        face: (f32, f32),
    }
    let r2 = r * r;
    let voted: Vec<Voted> = (0..n)
        .into_par_iter()
        .map_init(
            || (Vec::<u32>::new(), Vec::<u32>::new()),
            |(near, around), j| {
                cells3.near(cx[j], cy[j], cz[j], near);
                around.clear();
                for &i in near.iter() {
                    let i = i as usize;
                    let dx = cx[i] - cx[j];
                    let dy = cy[i] - cy[j];
                    let dz = cz[i] - cz[j];
                    if dx * dx + dy * dy + dz * dz <= r2 {
                        around.push(i as u32);
                    }
                }
                let mut top = f64::NEG_INFINITY;
                let mut bottom = f64::INFINITY;
                let mut column = 0.0f64;
                cells2.near(cx[j], cy[j], 0.0, near);
                for &i in near.iter() {
                    let i = i as usize;
                    let dx = cx[i] - cx[j];
                    let dy = cy[i] - cy[j];
                    if dx * dx + dy * dy > r2 {
                        continue;
                    }
                    top = jmax(top, ch[i]);
                    bottom = jmin(bottom, ch[i]);
                    column += 1.0;
                }
                let mut x = [0.0f64; 10];
                x[0] = ch[j] * k;
                x[6] = around.len() as f64;
                x[7] = top * k;
                x[8] = bottom * k;
                x[9] = column;
                let mut shape = [0.0, 0.0, 1.0, 0.0, 1.0];
                let mut face = (0f32, 0f32);
                if around.len() >= 3 {
                    let len = around.len() as f64;
                    let (mut mx, mut my, mut mz) = (0.0, 0.0, 0.0);
                    for &i in around.iter() {
                        mx += cx[i as usize];
                        my += cy[i as usize];
                        mz += cz[i as usize];
                    }
                    mx /= len;
                    my /= len;
                    mz /= len;
                    let mut c = [[0.0f64; 3]; 3];
                    for &i in around.iter() {
                        let q = [cx[i as usize] - mx, cy[i as usize] - my, cz[i as usize] - mz];
                        for p in 0..3 {
                            for s in 0..3 {
                                c[p][s] += (q[p] * q[s]) / len;
                            }
                        }
                    }
                    let (values, vectors) = eigen3(c);
                    // Largest first, ties in their order (a stable sort).
                    let mut order = [0usize, 1, 2];
                    order.sort_by(|&p, &q| values[q].partial_cmp(&values[p]).unwrap_or(std::cmp::Ordering::Equal));
                    let (l1, l2, l3) = (values[order[0]], values[order[1]], values[order[2]]);
                    if l1 > 1e-12 {
                        let e1 = vectors[order[0]];
                        let e3 = vectors[order[2]];
                        shape = [
                            (l1 - l2) / l1,
                            (l2 - l3) / l1,
                            l3 / l1,
                            (e1[2] / js_hypot(&[e1[0], e1[1], e1[2]])).abs(),
                            (e3[2] / js_hypot(&[e3[0], e3[1], e3[2]])).abs(),
                        ];
                        let across = js_hypot(&[e3[0], e3[1]]);
                        if across > 1e-9 {
                            face = ((e3[0] / across) as f32, (e3[1] / across) as f32);
                        }
                    }
                }
                x[1..6].copy_from_slice(&shape);
                let (order, share) = vote(a.trees, &x);
                Voted { order, share, scatter: x[3] as f32, flat: x[5] as f32, face }
            },
        )
        .collect();
    let mut choices = vec![-1i8; n * CLASSES];
    let mut label = vec![0i8; n];
    let mut share = vec![0f32; n];
    let mut forest_top = vec![OTHER; n];
    let mut scatter = vec![0f32; n];
    let mut flat = vec![1f32; n];
    let mut face_x = vec![0f32; n];
    let mut face_y = vec![0f32; n];
    for (j, got) in voted.into_iter().enumerate() {
        for (q, &c) in got.order.iter().enumerate() {
            choices[j * CLASSES + q] = c;
        }
        // The page's Int8Array takes an empty first choice as 0.
        label[j] = got.order.first().copied().unwrap_or(0);
        forest_top[j] = got.order.first().copied().unwrap_or(OTHER);
        share[j] = got.share;
        scatter[j] = got.scatter;
        flat[j] = got.flat;
        face_x[j] = got.face.0;
        face_y[j] = got.face.1;
    }

    // A roof's edge read as a tree's crown: three times.
    for _ in 0..3 {
        let label_now = &label;
        let roof: Vec<usize> = (0..n)
            .into_par_iter()
            .map_init(Vec::<u32>::new, |near, j| {
                if label_now[j] != VEG {
                    return None;
                }
                let (mut building, mut all) = (0u32, 0u32);
                let mut top = f64::NEG_INFINITY;
                cells3.near(cx[j], cy[j], cz[j], near);
                for &i in near.iter() {
                    let i = i as usize;
                    if i == j {
                        continue;
                    }
                    let dx = cx[i] - cx[j];
                    let dy = cy[i] - cy[j];
                    let dz = cz[i] - cz[j];
                    if dx * dx + dy * dy + dz * dz > r2 {
                        continue;
                    }
                    all += 1;
                    if label_now[i] == BUILDING {
                        building += 1;
                        top = jmax(top, cz[i]);
                    }
                }
                (all > 0 && building * 3 >= all && cz[j] <= top + r / 2.0).then_some(j)
            })
            .flatten()
            .collect();
        if roof.is_empty() {
            break;
        }
        for j in roof {
            label[j] = BUILDING;
            put_first(&mut choices, j, BUILDING);
        }
    }

    // Two cues: leafy green a foot or more up is vegetation.
    if a.coloured {
        for j in 0..n {
            let l = label[j];
            let leafy = (l == GROUND || l == OTHER) && ch[j] * k >= VEG_FLOOR_FT && sg[j] / sn[j] >= 0.3 && scatter[j] as f64 >= 0.08;
            if !leafy {
                continue;
            }
            label[j] = VEG;
            put_first(&mut choices, j, VEG);
        }
    }

    // Leaves aren't a building.
    let mut near: Vec<u32> = Vec::new();
    {
        let mut seen = vec![false; n];
        let mut around: Vec<u32> = Vec::new();
        for j in 0..n {
            if seen[j] || label[j] != BUILDING {
                continue;
            }
            let mut group = vec![j];
            seen[j] = true;
            let mut q = 0;
            while q < group.len() {
                let p = group[q];
                q += 1;
                cells3.near(cx[p], cy[p], cz[p], &mut around);
                for &b in around.iter() {
                    let b = b as usize;
                    if seen[b] || label[b] != BUILDING {
                        continue;
                    }
                    let dx = cx[b] - cx[p];
                    let dy = cy[b] - cy[p];
                    let dz = cz[b] - cz[p];
                    if dx * dx + dy * dy + dz * dz > r2 {
                        continue;
                    }
                    seen[b] = true;
                    group.push(b);
                }
            }
            let mut spread: Vec<f64> = group.iter().map(|&i| scatter[i] as f64).collect();
            spread.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
            let scattered = spread.iter().filter(|&&s| s >= SURFACE).count() as f64;
            if scattered >= LEAVES_SHARE * group.len() as f64 && spread[spread.len() / 2] >= LEAVES_MIDDLE {
                for &i in &group {
                    let row = &choices[i * CLASSES..(i + 1) * CLASSES];
                    label[i] = row.iter().copied().find(|&c| c >= 0 && c != BUILDING && c != WIRE && c != POLE).unwrap_or(VEG);
                }
            }
        }
    }

    // A fence's voxel under a building's wall going on up is the building's.
    {
        let tol = jmax(FENCE_PLANE / k, v);
        let above = FENCE_ABOVE / k;
        let mut walls = Cells::new(jmax(tol, r), true);
        for j in 0..n {
            if label[j] == BUILDING {
                walls.add(j as u32, cx[j], cy[j], 0.0);
            }
        }
        let mut under = vec![];
        for j in 0..n {
            if label[j] != FENCE {
                continue;
            }
            let mut over = 0;
            walls.near(cx[j], cy[j], 0.0, &mut near);
            for &i in near.iter() {
                let i = i as usize;
                let up = ch[i] - ch[j];
                if up <= v || up > above || scatter[i] as f64 >= SURFACE {
                    continue;
                }
                if js_hypot(&[cx[i] - cx[j], cy[i] - cy[j]]) <= tol {
                    over += 1;
                }
                if over >= 2 {
                    break;
                }
            }
            if over >= 2 {
                under.push(j);
            }
        }
        for j in under {
            label[j] = BUILDING;
            put_first(&mut choices, j, BUILDING);
        }
    }

    // ── Whole objects ──
    let pole_rise = jmax(4.0 * r, 6.0 / k);
    let wire_gap = jmax(3.0 * r, 10.0 / k);
    let mut far = Cells::new(jmax(pole_rise, wire_gap), false);
    for j in 0..n {
        if label[j] == POLE || label[j] == WIRE {
            far.add(j as u32, cx[j], cy[j], cz[j]);
        }
    }
    let linked = |l: i8, p: usize, b: usize| {
        let dx = cx[b] - cx[p];
        let dy = cy[b] - cy[p];
        let dz = cz[b] - cz[p];
        if l == POLE {
            dx * dx + dy * dy <= 2.25 * r * r && dz.abs() <= pole_rise
        } else if l == WIRE {
            dx * dx + dy * dy + dz * dz <= wire_gap * wire_gap
        } else {
            dx * dx + dy * dy + dz * dz <= r * r
        }
    };
    let mut comp = vec![-1i32; n];
    let mut members: Vec<u32> = Vec::with_capacity(n);
    let mut starts: Vec<u32> = Vec::new();
    for j in 0..n {
        if comp[j] >= 0 {
            continue;
        }
        let id = starts.len() as i32;
        let from = members.len();
        starts.push(from as u32);
        members.push(j as u32);
        comp[j] = id;
        let l = label[j];
        let cells = if l == POLE || l == WIRE { &far } else { &cells3 };
        let mut q = from;
        while q < members.len() {
            let p = members[q] as usize;
            q += 1;
            cells.near(cx[p], cy[p], cz[p], &mut near);
            for &b in near.iter() {
                let b = b as usize;
                if comp[b] >= 0 || label[b] != l || !linked(l, p, b) {
                    continue;
                }
                comp[b] = id;
                members.push(b as u32);
            }
        }
    }

    Stage {
        ground: ground_out,
        vox: vox_out,
        hags: hag_out,
        n,
        cx,
        cy,
        cz,
        ch,
        sn,
        sg,
        shapes,
        label,
        choices,
        share,
        forest_top,
        scatter,
        flat,
        face_x,
        face_y,
        members,
        starts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypot_as_v8_has_it() {
        assert_eq!(js_hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(js_hypot(&[0.0, 0.0, 0.0]), 0.0);
        assert!(js_hypot(&[f64::NAN, 1.0]).is_nan());
        assert_eq!(js_hypot(&[f64::INFINITY, f64::NAN]), f64::INFINITY);
    }

    #[test]
    fn cells_near_in_the_pages_order() {
        let mut c = Cells::new(1.0, false);
        c.add(0, 0.5, 0.5, 0.5);
        c.add(1, -0.5, 0.5, 0.5);
        c.add(2, 0.5, 1.5, 0.5);
        c.add(3, 3.5, 0.5, 0.5);
        let mut out = vec![];
        c.near(0.5, 0.5, 0.5, &mut out);
        // dx -1 first (index 1), then dx 0: dy 0 (0), dy 1 (2); 3 is too far.
        assert_eq!(out, vec![1, 0, 2]);
        let mut flat = Cells::new(1.0, true);
        flat.add(5, 0.5, 0.5, 99.0);
        flat.near(0.5, 0.5, 0.0, &mut out);
        assert_eq!(out, vec![5]);
    }

    #[test]
    fn votes_most_first_then_the_lower_class() {
        // Three one-node trees: 2, 1, 2.
        let leaf = |c: i32| Tree { f: vec![-2], t: vec![0.0], l: vec![-1], r: vec![-1], c: vec![c] };
        let trees = vec![leaf(2), leaf(1), leaf(2), leaf(1), leaf(5)];
        let (order, share) = vote(&trees, &[0.0; 10]);
        assert_eq!(order, vec![1, 2, 5]);
        assert!((share - 0.4).abs() < 1e-6);
    }
}
