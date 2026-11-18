//! Photo-real renders in the engine (1.29; Leo, 2026-11-18 06:04: "lets
//! build all 8 of you recommendations to make a true desktop app"): the
//! page's ray tracer (lib/editor/raytrace.ts) as it is, run on every core.
//!
//! The Visualize scene's triangles sit in a bounding volume hierarchy built
//! as the page builds it; each pass adds one jittered sample to every pixel.
//! Where a ray lands it's lit by the sun (a shadow ray to a slightly soft
//! sun), the drawing's lights (each with its shadow; a point light all
//! round, a spotlight in its cone, fading with distance; a distant one as
//! the sun), the sky and one bounce; glass reflects the sky by Schlick's
//! Fresnel and lets the rest through, tinted; misses see the sky's gradient.
//!
//! Rows are traced on every core. Each row of each pass draws its random
//! numbers from a source of its own (mulberry32, as the page's, seeded by
//! the render's seed, the pass and the row), so a render is the same each
//! time however many cores trace it.

use rayon::prelude::*;

type V = [f64; 3];

#[inline]
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}
#[inline]
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn norm(a: V) -> V {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let l = if l > 0.0 { l } else { 1.0 };
    [a[0] / l, a[1] / l, a[2] / l]
}

/// A colour's 0–255 channels as linear 0–1 (raytrace.ts linear).
pub fn linear_rgb(r: u8, g: u8, b: u8) -> V {
    let f = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(r), f(g), f(b)]
}

/// A mesh of the scene: its corners (x, y, z each), its triangles (three corners each), its colour, and whether it's glass.
pub struct Mesh {
    pub positions: Vec<f64>,
    pub indices: Vec<u32>,
    pub color: V,
    pub glass: bool,
}

struct Node {
    min: V,
    max: V,
    /// Its two halves (indices into the nodes), or none: a leaf of start..end.
    kids: Option<(usize, usize)>,
    start: usize,
    end: usize,
}

/// A scene ready to trace: the triangles flat, in a BVH.
pub struct Scene {
    a: Vec<V>,
    e1: Vec<V>,
    e2: Vec<V>,
    n: Vec<V>,
    color: Vec<V>,
    glass: Vec<bool>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}

impl Scene {
    /// The meshes' triangles in a BVH, built as the page's buildTraceable builds it.
    pub fn build(meshes: &[Mesh]) -> Scene {
        let mut s = Scene { a: vec![], e1: vec![], e2: vec![], n: vec![], color: vec![], glass: vec![], order: vec![], nodes: vec![] };
        let mut centers: Vec<V> = vec![];
        let mut lo_box: Vec<V> = vec![];
        let mut hi_box: Vec<V> = vec![];
        for m in meshes {
            let p = &m.positions;
            let at = |i: u32| -> Option<V> {
                let i = i as usize * 3;
                if i + 2 < p.len() {
                    Some([p[i], p[i + 1], p[i + 2]])
                } else {
                    None
                }
            };
            for t in m.indices.chunks_exact(3) {
                let (Some(pa), Some(pb), Some(pc)) = (at(t[0]), at(t[1]), at(t[2])) else { continue };
                let e1 = sub(pb, pa);
                let e2 = sub(pc, pa);
                s.a.push(pa);
                s.e1.push(e1);
                s.e2.push(e2);
                s.n.push(norm(cross(e1, e2)));
                s.color.push(m.color);
                s.glass.push(m.glass);
                let mut c = [0.0; 3];
                let mut lo = [0.0; 3];
                let mut hi = [0.0; 3];
                for k in 0..3 {
                    c[k] = (pa[k] + pb[k] + pc[k]) / 3.0;
                    lo[k] = pa[k].min(pb[k]).min(pc[k]);
                    hi[k] = pa[k].max(pb[k]).max(pc[k]);
                }
                centers.push(c);
                lo_box.push(lo);
                hi_box.push(hi);
            }
        }
        let count = s.a.len();
        s.order = (0..count).collect();
        s.build_node(0, count, 0, &centers, &lo_box, &hi_box);
        s
    }

    fn build_node(&mut self, start: usize, end: usize, depth: usize, centers: &[V], lo_box: &[V], hi_box: &[V]) -> usize {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for i in start..end {
            let k = self.order[i];
            for d in 0..3 {
                min[d] = min[d].min(lo_box[k][d]);
                max[d] = max[d].max(hi_box[k][d]);
            }
        }
        let me = self.nodes.len();
        self.nodes.push(Node { min, max, kids: None, start, end });
        if end - start <= 4 || depth > 40 {
            return me;
        }
        // Split at the middle of the widest spread of centres.
        let mut axis = 0;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for d in 0..3 {
            let mut l = f64::INFINITY;
            let mut h = f64::NEG_INFINITY;
            for i in start..end {
                let v = centers[self.order[i]][d];
                l = l.min(v);
                h = h.max(v);
            }
            if h - l > hi - lo {
                axis = d;
                lo = l;
                hi = h;
            }
        }
        if hi - lo < 1e-12 {
            return me;
        }
        let mid = (lo + hi) / 2.0;
        let mut i = start as isize;
        let mut j = end as isize - 1;
        while i <= j {
            if centers[self.order[i as usize]][axis] < mid {
                i += 1;
            } else {
                self.order.swap(i as usize, j as usize);
                j -= 1;
            }
        }
        let i = i as usize;
        if i == start || i == end {
            return me;
        }
        let left = self.build_node(start, i, depth + 1, centers, lo_box, hi_box);
        let right = self.build_node(i, end, depth + 1, centers, lo_box, hi_box);
        self.nodes[me].kids = Some((left, right));
        me
    }

    pub fn triangles(&self) -> usize {
        self.a.len()
    }

    #[inline]
    fn hit_box(n: &Node, o: V, inv: V, t_max: f64) -> bool {
        let mut t0 = 0.0;
        let mut t1 = t_max;
        for d in 0..3 {
            let mut a = (n.min[d] - o[d]) * inv[d];
            let mut b = (n.max[d] - o[d]) * inv[d];
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            t0 = if a > t0 { a } else { t0 };
            t1 = if b < t1 { b } else { t1 };
            if t0 > t1 {
                return false;
            }
        }
        true
    }

    /// The nearest triangle a ray meets: its distance and index (none: None).
    pub fn intersect(&self, o: V, d: V, t_max: f64, any: bool) -> Option<(f64, usize)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = [1.0 / d[0], 1.0 / d[1], 1.0 / d[2]];
        let mut best = t_max;
        let mut tri: Option<usize> = None;
        let mut stack: Vec<usize> = Vec::with_capacity(64);
        stack.push(0);
        while let Some(k) = stack.pop() {
            let n = &self.nodes[k];
            if !Self::hit_box(n, o, inv, best) {
                continue;
            }
            if let Some((l, r)) = n.kids {
                stack.push(l);
                stack.push(r);
                continue;
            }
            for i in n.start..n.end {
                let k = self.order[i];
                let (a, e1, e2) = (self.a[k], self.e1[k], self.e2[k]);
                // Möller–Trumbore.
                let p = [d[1] * e2[2] - d[2] * e2[1], d[2] * e2[0] - d[0] * e2[2], d[0] * e2[1] - d[1] * e2[0]];
                let det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2];
                if det.abs() < 1e-14 {
                    continue;
                }
                let id = 1.0 / det;
                let t = sub(o, a);
                let u = (t[0] * p[0] + t[1] * p[1] + t[2] * p[2]) * id;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = [t[1] * e1[2] - t[2] * e1[1], t[2] * e1[0] - t[0] * e1[2], t[0] * e1[1] - t[1] * e1[0]];
                let v = (d[0] * q[0] + d[1] * q[1] + d[2] * q[2]) * id;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let dist = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * id;
                if dist > 1e-9 && dist < best {
                    best = dist;
                    tri = Some(k);
                    if any {
                        return Some((dist, k));
                    }
                }
            }
        }
        tri.map(|k| (best, k))
    }
}

/// The camera: where it is, what it looks at, its vertical field of view in degrees.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub position: V,
    pub target: V,
    pub fov: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    Spot,
    Distant,
}

/// A light of the drawing (raytrace.ts TraceLight), its colour linear.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub kind: LightKind,
    pub position: V,
    pub direction: V,
    pub rgb: V,
    pub intensity: f64,
    pub inner: f64,
    pub outer: f64,
    pub reach: f64,
}

/// The render's settings (raytrace.ts RenderSettings).
#[derive(Clone, Debug)]
pub struct Settings {
    pub width: usize,
    pub height: usize,
    pub sun: V,
    pub lights: Vec<Light>,
    pub sun_off: bool,
    pub size: f64,
}

const NEAR_LIGHT: f64 = 1.5;
const FAR_LIGHT: f64 = 2.2;

/// mulberry32, as the page's `random`.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Rng {
        Rng(seed)
    }
    #[inline]
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (t ^ (t >> 14)) as f64 / 4_294_967_296.0
    }
}

/// A row's own random source: the render's seed, the pass and the row, mixed.
fn row_seed(seed: u32, pass: u32, row: u32) -> u32 {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ pass.wrapping_mul(0x85eb_ca6b) ^ row.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h
}

/// A progressive render: `pass` adds one sample to every pixel; `sum` the light summed.
pub struct Render<'a> {
    scene: &'a Scene,
    cam: Camera,
    s: Settings,
    seed: u32,
    pub sum: Vec<f32>,
    pub passes: u32,
    forward: V,
    right: V,
    up: V,
    sun: V,
    sun_color: V,
    lights: Vec<Light>,
}

const SKY_TOP: [u8; 3] = [0x5b, 0x8f, 0xd6];
const SKY_LOW: [u8; 3] = [0xdc, 0xe9, 0xf5];

impl<'a> Render<'a> {
    pub fn new(scene: &'a Scene, cam: Camera, s: Settings, seed: u32) -> Render<'a> {
        let forward = norm(sub(cam.target, cam.position));
        let right = norm(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let sun = norm(s.sun);
        // Low sun reddens.
        let low = (sun[1] * 3.0).clamp(0.0, 1.0);
        let sun_color = mul(add(mul([1.0, 0.62, 0.38], 1.0 - low), mul([1.0, 0.95, 0.88], low)), 2.6);
        let lights = s.lights.iter().filter(|l| l.intensity > 0.0).map(|l| Light { direction: norm(l.direction), ..*l }).collect();
        Render { scene, cam, sum: vec![0.0; s.width * s.height * 3], s, seed, passes: 0, forward, right, up, sun, sun_color, lights }
    }

    /// Going on from a render's light summed so far and its passes (the engine keeps a render between steps).
    pub fn resume(mut self, sum: Vec<f32>, passes: u32) -> Render<'a> {
        if sum.len() == self.sum.len() {
            self.sum = sum;
            self.passes = passes;
        }
        self
    }

    fn sky_at(d: V) -> V {
        let t = d[1].max(0.0);
        let top = linear_rgb(SKY_TOP[0], SKY_TOP[1], SKY_TOP[2]);
        let low = linear_rgb(SKY_LOW[0], SKY_LOW[1], SKY_LOW[2]);
        add(mul(low, 1.0 - t), mul(top, t))
    }

    /// A direction round a normal, cosine-weighted.
    fn hemisphere(n: V, r: &mut Rng) -> V {
        let u1 = r.next();
        let u2 = r.next();
        let rr = u1.sqrt();
        let th = 2.0 * std::f64::consts::PI * u2;
        let x = rr * th.cos();
        let y = rr * th.sin();
        let z = (1.0 - u1).max(0.0).sqrt();
        let t = norm(if n[0].abs() > 0.9 { cross(n, [0.0, 1.0, 0.0]) } else { cross(n, [1.0, 0.0, 0.0]) });
        let b = cross(n, t);
        norm(add(add(mul(t, x), mul(b, y)), mul(n, z)))
    }

    /// The light a ray brings back.
    fn shade(&self, o: V, d: V, depth: u32, r: &mut Rng) -> V {
        let far = self.s.size * 20.0;
        let Some((hit_t, tri)) = self.scene.intersect(o, d, far, false) else {
            return Self::sky_at(d);
        };
        let sc = self.scene;
        let mut n = sc.n[tri];
        if dot(n, d) > 0.0 {
            n = mul(n, -1.0);
        }
        let albedo = sc.color[tri];
        let p = add(o, mul(d, hit_t));
        let eps = self.s.size * 1e-5;
        let from = add(p, mul(n, eps));
        if sc.glass[tri] {
            // Schlick's Fresnel for glass (4% straight on).
            let c = (-dot(d, n)).max(0.0);
            let f = 0.04 + 0.96 * (1.0 - c).powi(5);
            if depth > 3 {
                return mul(Self::sky_at(d), 0.5);
            }
            let mirror = norm(sub(d, mul(n, 2.0 * dot(d, n))));
            let back = self.shade(from, mirror, depth + 1, r);
            let through = self.shade(add(p, mul(n, -eps)), d, depth + 1, r);
            let tint = [0.55 + 0.45 * albedo[0], 0.55 + 0.45 * albedo[1], 0.55 + 0.45 * albedo[2]];
            return add(mul(back, f), mul([through[0] * tint[0], through[1] * tint[1], through[2] * tint[2]], (1.0 - f) * 0.85));
        }
        let mut light = [0.0; 3];
        // The sun, a little soft.
        let jitter = [(r.next() - 0.5) * 0.02, (r.next() - 0.5) * 0.02, (r.next() - 0.5) * 0.02];
        let to_sun = norm(add(self.sun, jitter));
        let cos = dot(n, to_sun);
        if cos > 0.0 && self.sun[1] > 0.0 && !self.s.sun_off && sc.intersect(from, to_sun, far, true).is_none() {
            light = add(light, mul(self.sun_color, cos));
        }
        // The drawing's lights, each with its shadow.
        for l in &self.lights {
            let to: V;
            let mut reach = far;
            let mut k = l.intensity;
            if l.kind == LightKind::Distant {
                to = mul(l.direction, -1.0);
                k *= FAR_LIGHT;
            } else {
                let v = sub(l.position, from);
                let dist = dot(v, v).sqrt();
                if dist < eps {
                    continue;
                }
                to = mul(v, 1.0 / dist);
                reach = dist - eps;
                k *= NEAR_LIGHT * ((l.reach / dist).powi(2)).min(4.0);
                if l.kind == LightKind::Spot {
                    let off = (-dot(to, l.direction)).clamp(-1.0, 1.0).acos();
                    if off > l.outer {
                        continue;
                    }
                    if off > l.inner && l.outer > l.inner {
                        k *= (l.outer - off) / (l.outer - l.inner);
                    }
                }
            }
            let c = dot(n, to);
            if c <= 0.0 {
                continue;
            }
            if sc.intersect(from, to, reach, true).is_none() {
                light = add(light, mul(l.rgb, k * c));
            }
        }
        // The sky and a bounce.
        let dir = Self::hemisphere(n, r);
        if depth < 1 {
            light = add(light, self.shade(from, dir, depth + 1, r));
        } else if sc.intersect(from, dir, far, true).is_none() {
            light = add(light, Self::sky_at(dir));
        }
        [albedo[0] * light[0], albedo[1] * light[1], albedo[2] * light[2]]
    }

    /// One sample more for every pixel, the rows traced on every core.
    pub fn pass(&mut self) {
        let (w, h) = (self.s.width, self.s.height);
        let aspect = w as f64 / h as f64;
        let tan = (self.cam.fov / 2.0).to_radians().tan();
        let pass = self.passes;
        let this = &*self;
        let rows: Vec<Vec<f32>> = (0..h)
            .into_par_iter()
            .map(|y| {
                let mut r = Rng::new(row_seed(this.seed, pass, y as u32));
                let mut row = vec![0f32; w * 3];
                for x in 0..w {
                    let sx = ((x as f64 + r.next()) / w as f64) * 2.0 - 1.0;
                    let sy = 1.0 - ((y as f64 + r.next()) / h as f64) * 2.0;
                    let d = norm(add(add(this.forward, mul(this.right, sx * tan * aspect)), mul(this.up, sy * tan)));
                    let c = this.shade(this.cam.position, d, 0, &mut r);
                    row[3 * x] = c[0] as f32;
                    row[3 * x + 1] = c[1] as f32;
                    row[3 * x + 2] = c[2] as f32;
                }
                row
            })
            .collect();
        for (y, row) in rows.iter().enumerate() {
            let at = 3 * y * w;
            for (i, v) in row.iter().enumerate() {
                self.sum[at + i] += *v;
            }
        }
        self.passes += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(y: f64, s: f64, color: V, glass: bool) -> Mesh {
        Mesh { positions: vec![-s, y, -s, s, y, -s, s, y, s, -s, y, s], indices: vec![0, 2, 1, 0, 3, 2], color, glass }
    }

    #[test]
    fn mulberry32_as_the_page_has_it() {
        // random(1) on the page: its first three.
        let mut r = Rng::new(1);
        let got = [r.next(), r.next(), r.next()];
        assert!((got[0] - 0.6270739405881613).abs() < 1e-15, "{}", got[0]);
        assert!((got[1] - 0.002735721180215478).abs() < 1e-15, "{}", got[1]);
        assert!((got[2] - 0.5274470399599522).abs() < 1e-15, "{}", got[2]);
    }

    #[test]
    fn a_ground_lit_by_the_sun_and_a_sky_above() {
        let scene = Scene::build(&[quad(0.0, 50.0, [0.5, 0.5, 0.5], false)]);
        assert_eq!(scene.triangles(), 2);
        // Straight down onto it.
        let hit = scene.intersect([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 1e9, false);
        assert!((hit.unwrap().0 - 10.0).abs() < 1e-9);
        assert!(scene.intersect([0.0, 10.0, 0.0], [0.0, 1.0, 0.0], 1e9, false).is_none());
        let cam = Camera { position: [0.0, 5.0, 20.0], target: [0.0, 0.0, 0.0], fov: 50.0 };
        let s = Settings { width: 16, height: 12, sun: [0.3, 1.0, 0.2], lights: vec![], sun_off: false, size: 100.0 };
        let mut r = Render::new(&scene, cam, s.clone(), 1);
        r.pass();
        r.pass();
        assert_eq!(r.passes, 2);
        // The top rows see sky (blue over red), the bottom ones the lit ground.
        let px = |x: usize, y: usize| [r.sum[3 * (y * 16 + x)], r.sum[3 * (y * 16 + x) + 1], r.sum[3 * (y * 16 + x) + 2]];
        let top = px(8, 0);
        assert!(top[2] > top[0]);
        let bottom = px(8, 11);
        assert!(bottom[0] > 0.2, "{bottom:?}");
        // The same each time, whatever the cores.
        let mut again = Render::new(&scene, cam, s, 1);
        again.pass();
        again.pass();
        assert_eq!(r.sum, again.sum);
    }

    #[test]
    fn a_roof_shades_the_ground_under_it() {
        let scene = Scene::build(&[quad(0.0, 50.0, [0.5, 0.5, 0.5], false), quad(3.0, 4.0, [0.5, 0.5, 0.5], false)]);
        // Under the roof, looking down at the ground beneath it.
        let cam = Camera { position: [0.0, 2.0, 0.0], target: [0.0, 0.0, 1.0], fov: 10.0 };
        let s = Settings { width: 1, height: 1, sun: [0.0, 1.0, 0.0], lights: vec![], sun_off: false, size: 100.0 };
        let mut under = Render::new(&scene, cam, s.clone(), 3);
        for _ in 0..16 {
            under.pass();
        }
        let open_scene = Scene::build(&[quad(0.0, 50.0, [0.5, 0.5, 0.5], false)]);
        let mut open = Render::new(&open_scene, cam, s, 3);
        for _ in 0..16 {
            open.pass();
        }
        assert!(under.sum[0] < open.sum[0] * 0.5, "{} {}", under.sum[0], open.sum[0]);
    }
}
