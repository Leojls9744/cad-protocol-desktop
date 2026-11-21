//! Each point's shading from the shape of the surface it's on: the page's
//! own detail shading (cloud-surface.ts surfaceDetail and skyView), worked
//! out here for the points the engine draws, so they're lit as the page
//! lights its own:
//!
//! - Lighting: by its surface's direction, from a light in the north-west
//!   above (ambient 0.45 and the rest diffuse; both faces of a wall alike).
//! - Relief: how far it stands above or below the broad surface round it,
//!   over that surface's own roughness.
//! - Edge: where its own small patch of surface turns from the broad one.
//! - Sky: how much sky it sees (255: all of it), less in a gutter, a joint
//!   or a crack.
//!
//! Space is cut into cubes CELLS times the points' average spacing across
//! (a patch: its own surface); the 3 × 3 × 3 round each is the broad
//! surface. The hollows are on a grid of the lowest surface. Four bytes a
//! point come of it (`PointVertex::shade`); the card makes them a
//! brightness with the strengths the view asks for (gpu.rs), so a change
//! of strength sends no points again.
//!
//! The numbers are the page's: the same patches, the same sums in the same
//! order, the same roundings. The work is spread over the cores where that
//! changes none of them.

use crate::scene::{pack, CellMap, PointVertex};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// A patch's side, in the points' average spacing over their largest face.
pub const CELLS: f64 = 4.0;
/// The points a patch needs for a direction of its own.
const NEED: f64 = 4.0;
/// Ambient light: the share every surface gets whichever way it faces.
pub const AMBIENT: f64 = 0.45;
/// A patch for at most every this many points (on average).
const MOST_SHARE: usize = 8;

/// The hollows' grid: cells this many of the average spacings across.
pub const HOLLOW_CELL: f64 = 1.5;
/// The cells out each way a hollow's sides are looked for.
pub const HOLLOW_STEPS: [i64; 8] = [1, 2, 3, 4, 6, 8, 11, 15];
/// A rise under this share of a cell isn't a side: the scan's own scatter.
const HOLLOW_FLOOR: f64 = 0.15;
/// The most cells the grid takes (coarser past it).
const HOLLOW_MOST_CELLS: f64 = 16_777_216.0;
const HOLLOW_DIRS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// The light: from the north-west, above.
pub fn light() -> [f64; 3] {
    let v = [-0.5, 0.5, std::f64::consts::FRAC_1_SQRT_2];
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

/// Lighting that leaves a colour as it is: level ground's.
pub fn light_level() -> f64 {
    AMBIENT + (1.0 - AMBIENT) * light()[2]
}

/// A point nothing's known of: lit as level ground, on its surface, no edge, all the sky.
pub fn plain() -> [u8; 4] {
    [js_round(light_level() * 255.0) as u8, 0, 0, 255]
}

/// Math.round: halves go up.
#[inline]
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The smallest eigenvalue's vector and the three values (ascending) of the symmetric [[a, b, c], [b, d, e], [c, e, f]].
fn smallest(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> [f64; 6] {
    // Eigenvalues by the trigonometric method, the vector by cross products of the rows of (M − λI).
    let p1 = b * b + c * c + e * e;
    let q = (a + d + f) / 3.0;
    let (l0, l1, l2);
    if p1 < 1e-30 {
        let mut s = [a, d, f];
        s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        (l0, l1, l2) = (s[0], s[1], s[2]);
    } else {
        let p2 = (a - q) * (a - q) + (d - q) * (d - q) + (f - q) * (f - q) + 2.0 * p1;
        let p = (p2 / 6.0).sqrt();
        let (ba, bd, bf, bb, bc, be) = ((a - q) / p, (d - q) / p, (f - q) / p, b / p, c / p, e / p);
        let det = ba * (bd * bf - be * be) - bb * (bb * bf - be * bc) + bc * (bb * be - bd * bc);
        let r = (det / 2.0).clamp(-1.0, 1.0);
        let phi = r.acos() / 3.0;
        l2 = q + 2.0 * p * phi.cos();
        l0 = q + 2.0 * p * (phi + 2.0 * std::f64::consts::PI / 3.0).cos();
        l1 = 3.0 * q - l0 - l2;
    }
    let r0 = [a - l0, b, c];
    let r1 = [b, d - l0, e];
    let r2 = [c, e, f - l0];
    let cross = |u: [f64; 3], v: [f64; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let mut best = [0.0, 0.0, 1.0];
    let mut size = 0.0;
    for v in [cross(r0, r1), cross(r0, r2), cross(r1, r2)] {
        let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        if s > size {
            size = s;
            best = v;
        }
    }
    let n = size.sqrt();
    let dir = if n > 0.0 { [best[0] / n, best[1] / n, best[2] / n] } else { [0.0, 0.0, 1.0] };
    [dir[0], dir[1], dir[2], l0.max(0.0), l1.max(0.0), l2.max(0.0)]
}

/// A surface's direction turned to face up (a wall's, one way round).
fn face_up(n: &mut [f64; 6]) {
    let flip = if n[2].abs() > 0.2 { n[2] < 0.0 } else { n[0] + 0.7 * n[1] < 0.0 };
    if flip {
        n[0] = -n[0];
        n[1] = -n[1];
        n[2] = -n[2];
    }
}

/// Each point's patch (cubes `side` across from `lo`), numbered as found.
struct Grid {
    cell_of: Vec<u32>,
    /// Each patch's place in the grid, packed.
    keys: Vec<u64>,
    /// The patch at a place.
    map: CellMap,
}

/// A place's number along one axis.
#[inline]
fn along(v: f32, lo: f64, side: f64) -> i64 {
    (((v as f64 - lo) / side).floor() as i64).clamp(0, (1 << 21) - 1)
}

#[inline]
fn unpack(key: u64) -> [i64; 3] {
    [(key & 0x1f_ffff) as i64, (key >> 21 & 0x1f_ffff) as i64, (key >> 42 & 0x1f_ffff) as i64]
}

/// How many points a core takes at a time.
fn part_of(count: usize) -> usize {
    (count / (rayon::current_num_threads().max(1) * 4)).max(1 << 14)
}

/// Each point's patch, in `cell_of` (given back with nothing in it worth
/// keeping past `most` patches: the side's too small). Each core numbers
/// the patches of its own run of points as it finds them; the runs' are
/// then numbered together in the points' order, so the numbers are those
/// one pass over them all would give.
fn patches<P: Fn(&PointVertex) -> [f32; 3] + Sync>(data: &[PointVertex], place: &P, lo: [f64; 3], side: f64, most: usize, mut cell_of: Vec<u32>) -> Result<Grid, Vec<u32>> {
    cell_of.resize(data.len(), 0);
    let part = part_of(data.len());
    let locals: Vec<Option<Vec<u64>>> = cell_of
        .par_chunks_mut(part)
        .zip(data.par_chunks(part))
        .map(|(ids, points)| {
            let mut map = CellMap::default();
            let mut keys: Vec<u64> = Vec::new();
            let mut last = (u64::MAX, 0u32);
            for (id, v) in ids.iter_mut().zip(points) {
                let p = place(v);
                let key = pack(along(p[0], lo[0], side), along(p[1], lo[1], side), along(p[2], lo[2], side));
                // A scan's points come a place at a time: most are in the patch the one before was.
                if key != last.0 {
                    let next = keys.len() as u32;
                    let k = *map.entry(key).or_insert(next);
                    if k == next {
                        if keys.len() >= most {
                            return None;
                        }
                        keys.push(key);
                    }
                    last = (key, k);
                }
                *id = last.1;
            }
            Some(keys)
        })
        .collect();
    if locals.iter().any(Option::is_none) {
        return Err(cell_of);
    }
    let found: usize = locals.iter().flatten().map(Vec::len).sum();
    let mut map = CellMap::with_capacity_and_hasher(found.min(most) + 16, Default::default());
    let mut keys: Vec<u64> = Vec::with_capacity(found.min(most));
    let mut renumber: Vec<Vec<u32>> = Vec::with_capacity(locals.len());
    for local in locals.into_iter().flatten() {
        let mut to = Vec::with_capacity(local.len());
        for key in local {
            let next = keys.len() as u32;
            let k = *map.entry(key).or_insert(next);
            if k == next {
                if keys.len() >= most {
                    return Err(cell_of);
                }
                keys.push(key);
            }
            to.push(k);
        }
        renumber.push(to);
    }
    cell_of.par_chunks_mut(part).zip(renumber.par_iter()).for_each(|(ids, to)| {
        for id in ids {
            *id = to[*id as usize];
        }
    });
    Ok(Grid { cell_of, keys, map })
}

/// An f32 as a number that orders as it does (so the lowest of many can be kept by many cores at once).
#[inline]
fn ordered(v: f32) -> u32 {
    let b = v.to_bits();
    if b >> 31 == 0 {
        b | 0x8000_0000
    } else {
        !b
    }
}

#[inline]
fn unordered(k: u32) -> f32 {
    f32::from_bits(if k >> 31 == 1 { k & 0x7fff_ffff } else { !k })
}

/// The hollows' grid: its cells' size and how many across and up, and each cell's sky (255: all of it) and lowest point.
struct Sky {
    cell: f64,
    nx: usize,
    ny: usize,
    low: Vec<f32>,
    open: Vec<u8>,
    band: f64,
}

impl Sky {
    #[inline]
    fn at(&self, p: [f32; 3], lo: [f64; 3]) -> usize {
        let ix = (((p[0] as f64 - lo[0]) / self.cell).floor().max(0.0) as usize).min(self.nx - 1);
        let iy = (((p[1] as f64 - lo[1]) / self.cell).floor().max(0.0) as usize).min(self.ny - 1);
        iy * self.nx + ix
    }

    /// How much sky a point sees: its cell's where it's on the lowest surface, all of it above.
    #[inline]
    fn of(&self, p: [f32; 3], lo: [f64; 3]) -> u8 {
        let k = self.at(p, lo);
        if p[2] as f64 <= self.low[k] as f64 + self.band {
            self.open[k]
        } else {
            255
        }
    }
}

/// How much sky each cell of the lowest surface sees (the page's skyView):
/// each cell's lowest point; its horizon in eight directions out to 15
/// cells, the share of sky a side hides the sine of its height, one
/// further off counting less (half at the furthest). A rise under 0.15 of
/// a cell isn't a side, nor one higher than the reach.
fn sky_view<P: Fn(&PointVertex) -> [f32; 3] + Sync>(data: &[PointVertex], place: &P, lo: [f64; 3], hi: [f64; 3], average: f64) -> Option<Sky> {
    if (data.len() as f64) < NEED || !(average > 0.0) {
        return None;
    }
    let mut cell = HOLLOW_CELL * average;
    let across = |c: f64| (((hi[0] - lo[0]) / c).floor() + 1.0, ((hi[1] - lo[1]) / c).floor() + 1.0);
    let (mut nx, mut ny) = across(cell);
    while nx * ny > HOLLOW_MOST_CELLS {
        cell *= 1.5;
        (nx, ny) = across(cell);
    }
    if !(nx >= 1.0 && ny >= 1.0) {
        return None;
    }
    let mut sky = Sky { cell, nx: nx as usize, ny: ny as usize, low: Vec::new(), open: Vec::new(), band: 0.0 };
    let cells = sky.nx * sky.ny;
    // Each cell's lowest point.
    let lowest: Vec<AtomicU32> = (0..cells).into_par_iter().map(|_| AtomicU32::new(ordered(f32::INFINITY))).collect();
    data.par_iter().for_each(|v| {
        let p = place(v);
        lowest[sky.at(p, lo)].fetch_min(ordered(p[2]), Ordering::Relaxed);
    });
    sky.low = lowest.into_par_iter().map(|a| unordered(a.into_inner())).collect();
    let last = HOLLOW_STEPS[HOLLOW_STEPS.len() - 1];
    let reach = last as f64 * cell;
    let floor = HOLLOW_FLOOR * cell;
    let (nx, ny) = (sky.nx, sky.ny);
    let low = &sky.low;
    // Each cell's sky: its horizon along the eight directions. A row at a
    // time, and along the row for each direction and step (the same sums
    // in the same order for each cell as looking round it would give, but
    // the row's cells side by side, which the processor does several at
    // once).
    let mut open = vec![255u8; cells];
    open.par_chunks_mut(nx).enumerate().for_each_init(
        || (vec![0f64; nx], vec![0f64; nx]),
        |(hidden, most), (iy, row)| {
            let here = &low[iy * nx..(iy + 1) * nx];
            hidden.fill(0.0);
            for (dx, dy) in HOLLOW_DIRS {
                let run = if dx != 0 && dy != 0 { std::f64::consts::SQRT_2 * cell } else { cell };
                most.fill(0.0);
                for s in HOLLOW_STEPS {
                    // Past the grid's edge that way: no further.
                    let y = iy as i64 + dy * s;
                    if y < 0 || y >= ny as i64 {
                        break;
                    }
                    let shift = dx * s;
                    let (from, to) = if shift >= 0 { (0, nx as i64 - shift) } else { (-shift, nx as i64) };
                    if from >= to {
                        break;
                    }
                    let there = &low[y as usize * nx..(y as usize + 1) * nx];
                    let d = s as f64 * run;
                    let d2 = d * d;
                    let counts = 1.0 - (0.5 * s as f64) / last as f64;
                    // A side's share is under this much of its rise (the rise over how far off it is): a nearer side that hides more already settles most of them with no root taken.
                    let under = counts / d * (1.0 + 1e-9);
                    let mine = &here[from as usize..to as usize];
                    let theirs = &there[(from + shift) as usize..(to + shift) as usize];
                    for ((m, z0), nb) in most[from as usize..to as usize].iter_mut().zip(mine).zip(theirs) {
                        // An empty cell's lowest is infinite: past the reach, no side.
                        let rise = *nb as f64 - *z0 as f64 - floor;
                        if rise > 0.0 && rise <= reach && rise * under > *m {
                            let share = (rise / (rise * rise + d2).sqrt()) * counts;
                            if share > *m {
                                *m = share;
                            }
                        }
                    }
                }
                for (h, m) in hidden.iter_mut().zip(most.iter()) {
                    *h += *m;
                }
            }
            for ((out, h), z0) in row.iter_mut().zip(hidden.iter()).zip(here) {
                if *z0 != f32::INFINITY {
                    *out = js_round((1.0 - *h / HOLLOW_DIRS.len() as f64) * 255.0) as u8;
                }
            }
        },
    );
    sky.open = open;
    // The points on the lowest surface take their cell's.
    sky.band = cell.max(2.0 * floor);
    Some(sky)
}

/// Each point's shading worked out from where they all are, into its
/// `shade` (light, relief, edge, sky). `turn`: the scan's file into the
/// drawing's directions (the rows of its move there, without the shift),
/// where the two aren't the same way round: the light is the drawing's
/// north-west, and up is the drawing's.
pub fn shade_points(data: &mut [PointVertex], turn: Option<&[f64; 9]>) {
    let unknown = plain();
    data.par_iter_mut().for_each(|v| v.shade = unknown);
    let count = data.len();
    if (count as f64) < NEED {
        return;
    }
    // Where a point is, the drawing's way round (f32, as the page keeps its own).
    let place = |v: &PointVertex| -> [f32; 3] {
        match turn {
            None => v.pos,
            Some(t) => {
                let (x, y, z) = (v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64);
                [(t[0] * x + t[1] * y + t[2] * z) as f32, (t[3] * x + t[4] * y + t[5] * z) as f32, (t[6] * x + t[7] * y + t[8] * z) as f32]
            }
        }
    };
    // Their box, and their average spacing over its largest face.
    let (lo, hi) = data
        .par_chunks(1 << 16)
        .map(|part| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for v in part {
                let p = place(v);
                for a in 0..3 {
                    if p[a] < lo[a] {
                        lo[a] = p[a];
                    }
                    if p[a] > hi[a] {
                        hi[a] = p[a];
                    }
                }
            }
            (lo, hi)
        })
        .reduce(
            || ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |a, b| {
                let mut lo = a.0;
                let mut hi = a.1;
                for k in 0..3 {
                    lo[k] = lo[k].min(b.0[k]);
                    hi[k] = hi[k].max(b.1[k]);
                }
                (lo, hi)
            },
        );
    let lo = [lo[0] as f64, lo[1] as f64, lo[2] as f64];
    let hi = [hi[0] as f64, hi[1] as f64, hi[2] as f64];
    if !lo.iter().chain(&hi).all(|v| v.is_finite()) {
        return;
    }
    let (w, l, h) = (hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
    let area = (w * l).max(w * h).max(l * h).max(1e-12);
    let average = (area / count as f64).sqrt();
    let widest = w.max(l).max(h);
    // Patches CELLS spacings across; bigger where that would make more than
    // a patch for every MOST_SHARE points (a scanner's sparse far reaches);
    // smaller again while patches would hold more than twice that many (a
    // tripod scan, dense round each setup).
    let least = (widest / 1_048_576.0).max(1e-9);
    let most = count.div_ceil(MOST_SHARE);
    let mut side = (CELLS * average).max(least);
    let mut spare = Vec::new();
    let mut grid = loop {
        match patches(data, &place, lo, side, most, std::mem::take(&mut spare)) {
            Ok(g) => break g,
            Err(back) => {
                spare = back;
                side *= 1.5;
            }
        }
    };
    let mut tries = 0;
    while tries < 4 && grid.keys.len() * 2 * MOST_SHARE < count {
        match patches(data, &place, lo, (side / 1.5).max(least), most, std::mem::take(&mut spare)) {
            Ok(finer) => {
                side = (side / 1.5).max(least);
                spare = std::mem::replace(&mut grid, finer).cell_of;
            }
            Err(_) => break,
        }
        tries += 1;
    }
    drop(spare);
    let Grid { cell_of, keys, map } = grid;
    let cells = keys.len();
    // Each patch's moments, about its own corner (n, Σx, Σy, Σz, Σxx, Σxy,
    // Σxz, Σyy, Σyz, Σzz), each sum in its points' order (each core takes a
    // run of the patches and looks through the points for theirs).
    let mut moments = vec![0f32; cells * 10];
    let run = cells.div_ceil(rayon::current_num_threads().max(1)).max(1);
    moments.par_chunks_mut(run * 10).enumerate().for_each(|(t, mine)| {
        let from = t * run;
        let to = from + mine.len() / 10;
        for (i, &k) in cell_of.iter().enumerate() {
            let k = k as usize;
            if k < from || k >= to {
                continue;
            }
            let c = unpack(keys[k]);
            let p = place(&data[i]);
            let x = p[0] as f64 - lo[0] - c[0] as f64 * side;
            let y = p[1] as f64 - lo[1] - c[1] as f64 * side;
            let z = p[2] as f64 - lo[2] - c[2] as f64 * side;
            let m = &mut mine[(k - from) * 10..(k - from) * 10 + 10];
            for (at, add) in [1.0, x, y, z, x * x, x * y, x * z, y * y, y * z, z * z].into_iter().enumerate() {
                m[at] = (m[at] as f64 + add) as f32;
            }
        }
    });
    // Each patch's own surface (its direction and how bent), and the broad
    // one round it (the 3 × 3 × 3: its middle, direction, roughness and how
    // much of a surface it is).
    let floor = 0.5 * average;
    let mut own = vec![0f32; cells * 4];
    let mut has_own = vec![0u8; cells];
    let mut broad = vec![0f32; cells * 8];
    own.par_chunks_mut(4).zip(has_own.par_iter_mut()).zip(broad.par_chunks_mut(8)).enumerate().for_each(|(k, ((own, has), broad))| {
        let m = &moments[k * 10..k * 10 + 10];
        let n = m[0] as f64;
        if n >= NEED {
            let (mx, my, mz) = (m[1] as f64 / n, m[2] as f64 / n, m[3] as f64 / n);
            let mut e = smallest(
                m[4] as f64 / n - mx * mx,
                m[5] as f64 / n - mx * my,
                m[6] as f64 / n - mx * mz,
                m[7] as f64 / n - my * my,
                m[8] as f64 / n - my * mz,
                m[9] as f64 / n - mz * mz,
            );
            face_up(&mut e);
            own[0] = e[0] as f32;
            own[1] = e[1] as f32;
            own[2] = e[2] as f32;
            // How bent its points are (0 flat, 1 a ball): a curb's corner, a wall's foot.
            let sum = e[3] + e[4] + e[5];
            own[3] = if sum > 0.0 { (3.0 * e[3] / sum).min(1.0) as f32 } else { 0.0 };
            *has = 1;
        }
        // The broad surface: the moments of the 27 round it, about this patch's corner.
        let c = unpack(keys[k]);
        let mut s = [0f64; 10];
        for dz in -1i64..=1 {
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let j = if dx == 0 && dy == 0 && dz == 0 {
                        k
                    } else {
                        let (x, y, z) = (c[0] + dx, c[1] + dy, c[2] + dz);
                        if x < 0 || y < 0 || z < 0 {
                            continue;
                        }
                        match map.get(&pack(x, y, z)) {
                            Some(j) => *j as usize,
                            None => continue,
                        }
                    };
                    let q = &moments[j * 10..j * 10 + 10];
                    let cnt = q[0] as f64;
                    if cnt == 0.0 {
                        continue;
                    }
                    // Its moments moved from its corner to this one.
                    let (ax, ay, az) = (dx as f64 * side, dy as f64 * side, dz as f64 * side);
                    let (sx, sy, sz) = (q[1] as f64, q[2] as f64, q[3] as f64);
                    s[0] += cnt;
                    s[1] += sx + cnt * ax;
                    s[2] += sy + cnt * ay;
                    s[3] += sz + cnt * az;
                    s[4] += q[4] as f64 + 2.0 * ax * sx + cnt * ax * ax;
                    s[5] += q[5] as f64 + ay * sx + ax * sy + cnt * ax * ay;
                    s[6] += q[6] as f64 + az * sx + ax * sz + cnt * ax * az;
                    s[7] += q[7] as f64 + 2.0 * ay * sy + cnt * ay * ay;
                    s[8] += q[8] as f64 + az * sy + ay * sz + cnt * ay * az;
                    s[9] += q[9] as f64 + 2.0 * az * sz + cnt * az * az;
                }
            }
        }
        let nb = s[0];
        if nb < 3.0 * NEED {
            return;
        }
        let (bx, by, bz) = (s[1] / nb, s[2] / nb, s[3] / nb);
        let mut e = smallest(s[4] / nb - bx * bx, s[5] / nb - bx * by, s[6] / nb - bx * bz, s[7] / nb - by * by, s[8] / nb - by * bz, s[9] / nb - bz * bz);
        face_up(&mut e);
        broad[0] = bx as f32;
        broad[1] = by as f32;
        broad[2] = bz as f32;
        broad[3] = e[0] as f32;
        broad[4] = e[1] as f32;
        broad[5] = e[2] as f32;
        // How rough the broad surface is (twice its spread off its plane).
        broad[6] = (2.0 * e[3].sqrt()).max(floor) as f32;
        // How much of a surface it is (leaves: hardly).
        broad[7] = if e[4] > 0.0 { ((e[4] - e[3]) / e[4]).max(0.0) as f32 } else { 0.0 };
    });
    drop(moments);
    drop(map);
    // The hollows, on their own grid of the lowest surface.
    let sky = sky_view(data, &place, lo, hi, average);
    let sun = light();
    let part = part_of(count);
    data.par_chunks_mut(part).zip(cell_of.par_chunks(part)).for_each(|(points, cells_of)| {
        for (v, &k) in points.iter_mut().zip(cells_of) {
            let k = k as usize;
            let o = &own[k * 4..k * 4 + 4];
            let b = &broad[k * 8..k * 8 + 8];
            let has_own = has_own[k] == 1;
            let has_broad = b[6] > 0.0;
            let p = place(v);
            let mut out = unknown;
            if let Some(s) = &sky {
                out[3] = s.of(p, lo);
            }
            if has_own || has_broad {
                let n = if has_own { [o[0] as f64, o[1] as f64, o[2] as f64] } else { [b[3] as f64, b[4] as f64, b[5] as f64] };
                let lit = AMBIENT + (1.0 - AMBIENT) * (n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]).abs();
                out[0] = js_round(lit.min(1.0) * 255.0) as u8;
            }
            if has_broad {
                let flat = b[7] as f64;
                let c = unpack(keys[k]);
                let x = p[0] as f64 - lo[0] - c[0] as f64 * side;
                let y = p[1] as f64 - lo[1] - c[1] as f64 * side;
                let z = p[2] as f64 - lo[2] - c[2] as f64 * side;
                let r = (x - b[0] as f64) * b[3] as f64 + (y - b[1] as f64) * b[4] as f64 + (z - b[2] as f64) * b[5] as f64;
                out[1] = js_round((r / b[6] as f64).tanh() * flat * 127.0) as i8 as u8;
                // An edge: its patch bent, or turned from the surface round it, where that's a surface.
                if has_own {
                    let turn = 1.0 - (o[0] as f64 * b[3] as f64 + o[1] as f64 * b[4] as f64 + o[2] as f64 * b[5] as f64).abs();
                    let bend = (o[3] as f64 * 2.5).min(1.0);
                    out[2] = js_round(turn.max(bend).min(1.0) * (flat * 1.25).min(1.0) * 255.0) as u8;
                }
            }
            v.shade = out;
        }
    });
}

/// The rows of a scan's move into the drawing that turn it (none where it
/// isn't turned: the same way round and the same scale every way, which
/// the shading doesn't mind).
pub fn turn_of(affine: &[f64; 12]) -> Option<[f64; 9]> {
    let t = [affine[0], affine[1], affine[2], affine[4], affine[5], affine[6], affine[8], affine[9], affine[10]];
    let s = t[0];
    let tiny = s.abs() * 1e-9;
    let plain = s > 0.0 && (t[4] - s).abs() <= tiny && (t[8] - s).abs() <= tiny && [1, 2, 3, 5, 6, 7].iter().all(|k| t[*k].abs() <= tiny);
    (!plain && t.iter().all(|v| v.is_finite())).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64, z: f64) -> PointVertex {
        PointVertex { pos: [x as f32, y as f32, z as f32], rgb: [0; 3], class: 0, intensity: 0, setup: 0, shade: [0; 4] }
    }

    /// The page's own numbers for a made-up yard (below), from its cloud-surface.ts.
    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f64 / 16_777_216.0
        }
    }

    /// A yard 20 by 12: level ground with a gutter along it (a V 0.6 wide
    /// and 0.15 deep), a curb's face 0.15 high with its top behind it, and
    /// a wall 2 high at the far side, every point a little off its place.
    /// Made the same in the page's own code for its numbers below.
    fn yard(n: usize) -> Vec<PointVertex> {
        let mut r = Lcg(12345);
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (u, v, w) = (r.next(), r.next(), r.next());
            let noise = (w - 0.5) * 0.004;
            let p = match i % 10 {
                0 | 1 => [u * 20.0, 12.0 + noise, v * 2.0],
                2 => [u * 20.0, 8.0 + noise, v * 0.15],
                _ => {
                    let y = v * 12.0;
                    let dip = (0.3 - (y - 4.0).abs()).max(0.0) * 0.5;
                    let up = if y > 8.0 { 0.15 } else { 0.0 };
                    [u * 20.0, y, up - dip + noise]
                }
            };
            out.push(at(p[0], p[1], p[2]));
        }
        out
    }

    /// The yard's shading as the page's own cloud-surface.ts (surfaceDetail)
    /// works it out for the same points: the sums of its light, relief,
    /// edge and sky, and how many points are in a hollow.
    const PAGE_YARD: (usize, [i64; 4], usize) = (200_000, [40_348_296, 115_166, 2_838_865, 49_824_748], 24_442);

    #[test]
    fn the_yards_shading_is_the_pages_own() {
        let (n, sums, hollow) = PAGE_YARD;
        let mut data = yard(n);
        shade_points(&mut data, None);
        let mut got = [0i64; 4];
        for v in &data {
            got[0] += v.shade[0] as i64;
            got[1] += v.shade[1] as i8 as i64;
            got[2] += v.shade[2] as i64;
            got[3] += v.shade[3] as i64;
        }
        assert_eq!(got, sums, "light, relief, edge and sky, summed");
        assert_eq!(data.iter().filter(|v| v.shade[3] < 255).count(), hollow);
        // Point for point, where the page's are at hand (CADP_YARD: its four bytes a point).
        if let Ok(path) = std::env::var("CADP_YARD") {
            let page = std::fs::read(&path).expect("the page's bytes");
            let n = page.len() / 4;
            let mut data = yard(n);
            let t = std::time::Instant::now();
            shade_points(&mut data, None);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let mut off = [0usize; 4];
            let mut most = [0i32; 4];
            for (i, v) in data.iter().enumerate() {
                for k in 0..4 {
                    let (a, b) = if k == 1 { (v.shade[1] as i8 as i32, page[i * 4 + 1] as i8 as i32) } else { (v.shade[k] as i32, page[i * 4 + k] as i32) };
                    if a != b {
                        off[k] += 1;
                        most[k] = most[k].max((a - b).abs());
                    }
                }
            }
            println!("{n} points in {ms:.0} ms on {} cores: off the page's {off:?}, by at most {most:?}", rayon::current_num_threads());
            assert_eq!(off, [0; 4]);
        }
    }

    #[test]
    fn level_ground_is_left_as_it_is_and_a_wall_is_lit_by_the_way_it_faces() {
        let mut r = Lcg(7);
        // Level ground, 10 by 10, and a wall facing south at its north side.
        let mut data: Vec<PointVertex> = (0..40_000).map(|_| at(r.next() * 10.0, r.next() * 10.0, (r.next() - 0.5) * 0.002)).collect();
        let ground = data.len();
        data.extend((0..20_000).map(|_| at(r.next() * 10.0, 10.0 + (r.next() - 0.5) * 0.002, 0.3 + r.next() * 5.0)));
        shade_points(&mut data, None);
        let level = plain()[0] as f64;
        // Away from the wall's foot and the edges: level ground's own light, all the sky, on its surface.
        let inner: Vec<&PointVertex> = data[..ground].iter().filter(|v| v.pos[0] > 1.0 && v.pos[0] < 9.0 && v.pos[1] > 1.0 && v.pos[1] < 8.0).collect();
        assert!(inner.len() > 10_000);
        let mean = |f: &dyn Fn(&PointVertex) -> f64, of: &[&PointVertex]| of.iter().map(|v| f(v)).sum::<f64>() / of.len() as f64;
        assert!((mean(&|v| v.shade[0] as f64, &inner) - level).abs() < 1.5, "level ground's light");
        assert!(inner.iter().filter(|v| v.shade[3] == 255).count() * 100 > inner.len() * 97, "level ground sees the sky");
        assert!(inner.iter().filter(|v| v.shade[2] < 40).count() * 100 > inner.len() * 95, "level ground has no edges");
        // The wall faces south: the light's from the north-west, |n·L| = 0.5 there.
        let wall: Vec<&PointVertex> = data[ground..].iter().filter(|v| v.pos[2] > 1.5 && v.pos[2] < 4.5 && v.pos[0] > 1.0 && v.pos[0] < 9.0).collect();
        let lit = (AMBIENT + (1.0 - AMBIENT) * 0.5) * 255.0;
        assert!((mean(&|v| v.shade[0] as f64, &wall) - lit).abs() < 2.0, "a wall's light: {} for {lit}", mean(&|v| v.shade[0] as f64, &wall));
        // The ground at the wall's foot sees less sky.
        let foot: Vec<&PointVertex> = data[..ground].iter().filter(|v| v.pos[1] > 9.6 && v.pos[0] > 1.0 && v.pos[0] < 9.0).collect();
        assert!(mean(&|v| v.shade[3] as f64, &foot) < 235.0, "the wall's foot is in its shadow: {}", mean(&|v| v.shade[3] as f64, &foot));
    }

    #[test]
    fn a_scan_turned_in_the_drawing_is_lit_from_the_drawings_north_west() {
        let mut r = Lcg(9);
        // A wall along the file's x, facing the file's y.
        let wall = |r: &mut Lcg| at(r.next() * 10.0, (r.next() - 0.5) * 0.002, r.next() * 5.0);
        let mut plain_way: Vec<PointVertex> = (0..30_000).map(|_| wall(&mut r)).collect();
        let mut turned = plain_way.clone();
        shade_points(&mut plain_way, None);
        // Turned a quarter turn in the drawing: it faces the drawing's x there.
        let quarter = [0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        assert_eq!(turn_of(&[2.0, 0.0, 0.0, 5.0, 0.0, 2.0, 0.0, 6.0, 0.0, 0.0, 2.0, 7.0]), None);
        let turn = turn_of(&quarter).expect("a turn");
        shade_points(&mut turned, Some(&turn));
        let mean = |d: &[PointVertex]| d.iter().map(|v| v.shade[0] as f64).sum::<f64>() / d.len() as f64;
        // Facing y or x, the light's as far off either (north-west): the same.
        assert!((mean(&plain_way) - mean(&turned)).abs() < 1.0);
        // An eighth of a turn: facing the light full on, or edge on to it.
        let c = std::f64::consts::FRAC_1_SQRT_2;
        let mut toward = plain_way.clone();
        shade_points(&mut toward, Some(&turn_of(&[c, c, 0.0, 0.0, -c, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]).expect("a turn")));
        let mut along = plain_way.clone();
        shade_points(&mut along, Some(&turn_of(&[c, -c, 0.0, 0.0, c, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]).expect("a turn")));
        let (a, b) = (mean(&toward), mean(&along));
        assert!((a - b).abs() > 60.0, "a wall facing the light and one edge on to it: {a} and {b}");
    }

    #[test]
    fn a_few_points_and_none_are_left_plain() {
        let mut none: Vec<PointVertex> = Vec::new();
        shade_points(&mut none, None);
        let mut few = vec![at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(0.0, 1.0, 0.0)];
        shade_points(&mut few, None);
        assert!(few.iter().all(|v| v.shade == plain()));
        assert_eq!(plain(), [214, 0, 0, 255]);
        // All in one place: nothing to work a surface from, and nothing breaks.
        let mut one = vec![at(3.0, 3.0, 3.0); 100];
        shade_points(&mut one, None);
        assert!(one.iter().all(|v| v.shade[3] == 255));
    }

    #[test]
    fn floats_keep_their_order_as_numbers() {
        let list = [-1e30f32, -2.5, -0.0, 0.0, 1e-20, 3.0, f32::INFINITY];
        for w in list.windows(2) {
            assert!(ordered(w[0]) <= ordered(w[1]));
        }
        for v in list {
            assert_eq!(unordered(ordered(v)), v);
        }
    }
}
