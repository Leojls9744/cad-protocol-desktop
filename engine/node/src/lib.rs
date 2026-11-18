//! The scan engine as a Node add-on (desktop 1.19), loaded only in the
//! desktop app's engine process (src/engine-host.js), never in a page.
//!
//! openScan(path, cacheDir) indexes a scan (or opens its index made
//! before) on a worker thread and gives its handle; openProgress(path) says
//! how far it's come; snap(scans, ask) finds the points under the cursor
//! (each 14 numbers in a Float64Array: scan, index, x, y, z, pixels from the
//! cursor, nearness to the eye, class, flags, intensity, returns, red,
//! green, blue). 1.20: region(handle,
//! affine, min, max, ring, most) gives a scan's points in a box of the
//! drawing, evenly thinned to about `most`. 1.22: alignClouds(fixed,
//! moving, ask) lines one scan's points up on another's after the picked
//! points' fit (point to plane), and checkClouds(scans, foot, tolerance)
//! measures each pair that meets both ways and each scan's points' gaps, on
//! worker threads, as the page's ALIGNSCANS and SCANCHECK do it. 1.23:
//! netClassify(model, blob, points) runs Geo W. over a scan's points on a
//! worker thread (every core), as the page runs it; netProgress() says how
//! far the run in hand has come. 1.24: E57 scans opened as LAS scans are
//! (their setups kept; snap and region asked of one setup when given).
//! 1.25: scans indexed on threads of their own, a few at a time, so cloud
//! to cloud and Geo W. (on Node's worker threads) never wait behind them.
//! 1.28: renderOpen(meshes) puts a render's scene in a BVH, renderStart and
//! renderStep(job, passes) trace passes of it on every core (each step on
//! a worker thread), renderClose lets one go.
//! 1.28: cropScan(handle, ask, out) cuts a scan to the drawing's limit
//! boxes from its index, every point, written to a LAS or LAZ file on a
//! worker thread; cropProgress() says how far it's come.

use cadp_engine_core::c2c::{self, AlignOpts, GapStats};
use cadp_engine_core::crop::{self, CropAsk, CropBox};
use cadp_engine_core::render::{self as rt, Camera, Light, LightKind, Mesh, Scene, Settings};
use cadp_engine_core::net;
use cadp_engine_core::{
    build_fresh, open_made, region as within, snap as find, Candidate, Index, Progress, RegionQuery, ScanIn, SnapQuery,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Instant;

static INDEXES: LazyLock<Mutex<HashMap<u32, Arc<Index>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static OPENING: LazyLock<Mutex<HashMap<String, Arc<Progress>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicU32 = AtomicU32::new(1);

/// The numbers each point found is given as.
pub const FIELDS: u32 = 14;

#[napi]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[napi(object)]
pub struct ScanInfo {
    pub handle: u32,
    pub count: f64,
    /// Indexed now (else its index made before was opened).
    pub built: bool,
    pub ms: f64,
    pub has_rgb: bool,
    /// Each point's setup kept (an E57 of several scans, 1.24).
    pub setups: bool,
    /// Made from an E57 file (1.24).
    pub e57: bool,
    /// The file's own coordinates of its points' box.
    pub min: Vec<f64>,
    pub max: Vec<f64>,
}

/// Scans indexed at once (1.25), each on a thread of its own; any more wait their turn.
const MOST_BUILDS: usize = 4;
static BUILDS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// A turn to index, given back when dropped.
struct Turn;

impl Turn {
    fn wait() -> Turn {
        let (count, freed) = &BUILDS;
        let mut n = count.lock().unwrap();
        while *n >= MOST_BUILDS {
            n = freed.wait(n).unwrap();
        }
        *n += 1;
        Turn
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let (count, freed) = &BUILDS;
        *count.lock().unwrap() -= 1;
        freed.notify_one();
    }
}

/// An index kept by a new handle; what the page is told of it.
fn keep(ix: Index, started: Instant) -> ScanInfo {
    let handle = NEXT.fetch_add(1, Ordering::Relaxed);
    let lo = ix.file_xyz(ix.min[0], ix.min[1], ix.min[2]);
    let hi = ix.file_xyz(ix.max[0], ix.max[1], ix.max[2]);
    let info = ScanInfo {
        handle,
        count: ix.count as f64,
        built: ix.built,
        ms: started.elapsed().as_secs_f64() * 1000.0,
        has_rgb: ix.has_rgb,
        setups: ix.setups,
        e57: ix.e57,
        min: lo.to_vec(),
        max: hi.to_vec(),
    };
    INDEXES.lock().unwrap().insert(handle, Arc::new(ix));
    info
}

/// A scan indexed (or its index made before opened) on a thread of its own
/// (1.25): one made before opens at once; a new one waits for a turn.
#[napi(ts_return_type = "Promise<ScanInfo>")]
pub fn open_scan(env: &Env, path: String, cache_dir: String) -> Result<Object<'_>> {
    let (deferred, promise) = env.create_deferred::<ScanInfo, _>()?;
    let progress = Arc::new(Progress::default());
    OPENING.lock().unwrap().insert(path.clone(), progress.clone());
    let started = Instant::now();
    let key = path.clone();
    let spawned = std::thread::Builder::new().name("cadp-index".into()).spawn(move || {
        let src = PathBuf::from(&path);
        let cache = PathBuf::from(&cache_dir);
        let got = match open_made(&src, &cache) {
            Some(ix) => {
                progress.set(1.0);
                Ok(ix)
            }
            None => {
                let _turn = Turn::wait();
                build_fresh(&src, &cache, &progress)
            }
        };
        OPENING.lock().unwrap().remove(&path);
        match got {
            Ok(ix) => {
                let info = keep(ix, started);
                deferred.resolve(move |_| Ok(info));
            }
            Err(e) => deferred.reject(Error::from_reason(e.to_string())),
        }
    });
    if let Err(e) = spawned {
        OPENING.lock().unwrap().remove(&key);
        return Err(Error::from_reason(format!("couldn't start indexing: {e}")));
    }
    Ok(promise)
}

/// How far a scan's indexing has come (0 to 1), or -1 when none is.
#[napi]
pub fn open_progress(path: String) -> f64 {
    OPENING.lock().unwrap().get(&path).map(|p| p.get()).unwrap_or(-1.0)
}

#[napi(object)]
pub struct SnapScan {
    pub handle: u32,
    /// The file's coordinates into the drawing: 12 numbers, row by row with the shift after each.
    pub affine: Vec<f64>,
    /// The classes a snap may take: 8 numbers of 32 bits (class c is bit c % 32 of number c / 32); all when missing.
    pub allow: Option<Vec<u32>>,
    /// Only this setup's points (an E57 setup attached alone, 1.24).
    pub setup: Option<u32>,
}

#[napi(object)]
pub struct SnapAsk {
    pub target: Vec<f64>,
    pub right: Vec<f64>,
    pub up: Vec<f64>,
    pub to_eye: Vec<f64>,
    pub units_per_pixel: f64,
    pub half_w: f64,
    pub half_h: f64,
    pub cursor: Vec<f64>,
    pub reach: f64,
    pub front_px: f64,
    pub window: f64,
    pub below: Option<f64>,
    /// Only points this near the eye in `near`: [low, high].
    pub band: Option<Vec<f64>>,
    pub most: u32,
    pub front_most: u32,
}

#[napi(object)]
pub struct SnapFound {
    pub front: Float64Array,
    pub near: Float64Array,
    pub tried: f64,
    pub cells: f64,
    pub ms: f64,
}

fn three(v: &[f64], what: &str) -> Result<[f64; 3]> {
    if v.len() != 3 || v.iter().any(|x| !x.is_finite()) {
        return Err(Error::from_reason(format!("{what} needs three numbers")));
    }
    Ok([v[0], v[1], v[2]])
}

fn packed(list: &[Candidate]) -> Float64Array {
    let mut out = Vec::with_capacity(list.len() * FIELDS as usize);
    for c in list {
        out.extend_from_slice(&[
            c.scan as f64,
            c.index as f64,
            c.p[0],
            c.p[1],
            c.p[2],
            c.d,
            c.h,
            c.class as f64,
            c.flags as f64,
            c.intensity as f64,
            c.returns as f64,
            c.rgb[0] as f64,
            c.rgb[1] as f64,
            c.rgb[2] as f64,
        ]);
    }
    Float64Array::from(out)
}

/// The points under the cursor of the scans asked of.
#[napi]
pub fn snap(scans: Vec<SnapScan>, ask: SnapAsk) -> Result<SnapFound> {
    let started = Instant::now();
    let q = SnapQuery {
        target: three(&ask.target, "target")?,
        right: three(&ask.right, "right")?,
        up: three(&ask.up, "up")?,
        to_eye: three(&ask.to_eye, "toEye")?,
        units_per_pixel: ask.units_per_pixel,
        half_w: ask.half_w,
        half_h: ask.half_h,
        cursor: match ask.cursor.as_slice() {
            [x, y] if x.is_finite() && y.is_finite() => [*x, *y],
            _ => return Err(Error::from_reason("cursor needs two numbers")),
        },
        reach: ask.reach.clamp(0.0, 256.0),
        front_px: ask.front_px.clamp(0.0, 64.0),
        window: ask.window.max(0.0),
        below: ask.below.filter(|b| b.is_finite()),
        band: match ask.band.as_deref() {
            Some([lo, hi]) if lo.is_finite() && hi.is_finite() => Some([*lo, *hi]),
            Some(_) => return Err(Error::from_reason("band needs two numbers")),
            None => None,
        },
        most: (ask.most as usize).min(20_000),
        front_most: (ask.front_most as usize).min(4_096),
    };
    let held: Vec<(Arc<Index>, [f64; 12], [u64; 4], Option<u16>)> = {
        let open = INDEXES.lock().unwrap();
        let mut out = Vec::with_capacity(scans.len());
        for s in &scans {
            let Some(ix) = open.get(&s.handle) else {
                return Err(Error::from_reason(format!("scan {} isn't open", s.handle)));
            };
            if s.affine.len() != 12 || s.affine.iter().any(|v| !v.is_finite()) {
                return Err(Error::from_reason("affine needs twelve numbers"));
            }
            let mut a = [0.0; 12];
            a.copy_from_slice(&s.affine);
            let mut allow = [u64::MAX; 4];
            if let Some(bits) = &s.allow {
                if bits.len() != 8 {
                    return Err(Error::from_reason("allow needs eight numbers"));
                }
                for (k, w) in allow.iter_mut().enumerate() {
                    *w = bits[k * 2] as u64 | (bits[k * 2 + 1] as u64) << 32;
                }
            }
            out.push((ix.clone(), a, allow, s.setup.map(|k| k.min(u16::MAX as u32) as u16)));
        }
        out
    };
    let ins: Vec<ScanIn> = held
        .iter()
        .map(|(ix, a, allow, setup)| ScanIn { index: ix.as_ref(), affine: *a, allow: *allow, setup: *setup })
        .collect();
    let got = find(&ins, &q);
    Ok(SnapFound {
        front: packed(&got.front),
        near: packed(&got.near),
        tried: got.tried as f64,
        cells: got.cells as f64,
        ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

#[napi(object)]
pub struct RegionFound {
    /// Where the points are placed from (the box's middle).
    pub origin: Vec<f64>,
    pub count: u32,
    pub xyz: Float32Array,
    pub intensity: Uint16Array,
    pub classification: Uint8Array,
    pub flags: Uint8Array,
    pub returns: Uint8Array,
    /// Bytes, three a point; none when the scan has no colour.
    pub rgb: Option<Uint8Array>,
    /// Each point's setup (an E57 of several scans, 1.24); none otherwise.
    pub setup_of: Option<Uint16Array>,
    /// Every point in the box came.
    pub complete: bool,
    pub tried: f64,
    pub cells: f64,
    pub ms: f64,
}

/// A scan's points in a box of the drawing (and in a ring, flat x, y pairs, when given), about `most` at most; one setup's alone when `setup` is given (1.24).
#[napi]
pub fn region(
    handle: u32,
    affine: Vec<f64>,
    min: Vec<f64>,
    max: Vec<f64>,
    ring: Option<Vec<f64>>,
    most: u32,
    setup: Option<u32>,
) -> Result<RegionFound> {
    let started = Instant::now();
    let ix = INDEXES
        .lock()
        .unwrap()
        .get(&handle)
        .cloned()
        .ok_or_else(|| Error::from_reason(format!("scan {handle} isn't open")))?;
    if affine.len() != 12 || affine.iter().any(|v| !v.is_finite()) {
        return Err(Error::from_reason("affine needs twelve numbers"));
    }
    let mut a = [0.0; 12];
    a.copy_from_slice(&affine);
    let ring = match ring {
        Some(r) if r.len() >= 6 && r.len() % 2 == 0 && r.iter().all(|v| v.is_finite()) => {
            Some(r.chunks_exact(2).map(|p| [p[0], p[1]]).collect())
        }
        Some(_) => return Err(Error::from_reason("ring needs three or more x, y pairs")),
        None => None,
    };
    let q = RegionQuery {
        min: three(&min, "min")?,
        max: three(&max, "max")?,
        ring,
        most: (most as usize).clamp(1, 50_000_000),
        setup: setup.map(|k| k.min(u16::MAX as u32) as u16),
    };
    let got = within(&ix, &a, &q);
    Ok(RegionFound {
        origin: got.origin.to_vec(),
        count: got.class.len() as u32,
        xyz: Float32Array::from(got.xyz),
        intensity: Uint16Array::from(got.intensity),
        classification: Uint8Array::from(got.class),
        flags: Uint8Array::from(got.flags),
        returns: Uint8Array::from(got.returns),
        rgb: got.rgb.map(Uint8Array::from),
        setup_of: got.setup.map(Uint16Array::from),
        complete: got.complete,
        tried: got.tried as f64,
        cells: got.cells as f64,
        ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

#[napi(object)]
pub struct Gaps {
    pub checked: f64,
    pub overlap: f64,
    pub mean: f64,
    pub rms: f64,
    pub p95: f64,
    pub within: f64,
    pub alone: f64,
}

fn gaps_js(g: &GapStats) -> Gaps {
    Gaps { checked: g.checked, overlap: g.overlap, mean: g.mean, rms: g.rms, p95: g.p95, within: g.within, alone: g.alone }
}

#[napi(object)]
pub struct AlignAsk {
    /// The picked points' fit (12 numbers) and how far its pairs missed (root mean square).
    pub fit: Vec<f64>,
    pub fit_rms: f64,
    /// About the vertical only (a levelled scanner).
    pub level: bool,
    /// Cloud to cloud after the fit.
    pub refine: bool,
    /// One foot in the drawing's units.
    pub foot: f64,
    pub tolerance: f64,
}

#[napi(object)]
pub struct AlignFound {
    pub align: Vec<f64>,
    pub before: Gaps,
    pub after: Gaps,
    pub iterations: u32,
    /// The direction the overlap barely holds: its ratio and which unknown (none when it holds them all).
    pub degenerate_ratio: Option<f64>,
    pub degenerate_along: Option<u32>,
    pub refined: bool,
    pub ms: f64,
}

pub struct AlignTask {
    fixed: Vec<f64>,
    moving: Vec<f64>,
    opts: AlignOpts,
    started: Instant,
}

impl Task for AlignTask {
    type Output = c2c::Aligned;
    type JsValue = AlignFound;

    fn compute(&mut self) -> Result<Self::Output> {
        Ok(c2c::align(&self.fixed, &self.moving, &self.opts))
    }

    fn resolve(&mut self, _env: Env, a: Self::Output) -> Result<Self::JsValue> {
        Ok(AlignFound {
            align: a.align.to_vec(),
            before: gaps_js(&a.before),
            after: gaps_js(&a.after),
            iterations: a.iterations as u32,
            degenerate_ratio: a.degenerate.map(|d| d.0),
            degenerate_along: a.degenerate.map(|d| d.1 as u32),
            refined: a.refined,
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn points(v: &[f64], what: &str) -> Result<Vec<f64>> {
    if v.len() % 3 != 0 || v.iter().any(|x| !x.is_finite()) {
        return Err(Error::from_reason(format!("{what} needs x, y, z numbers")));
    }
    Ok(v.to_vec())
}

/// The moving scan's points lined up on the held one's, after the picked points' fit, on a worker thread.
#[napi]
pub fn align_clouds(fixed: Float64Array, moving: Float64Array, ask: AlignAsk) -> Result<AsyncTask<AlignTask>> {
    if ask.fit.len() != 12 || ask.fit.iter().any(|v| !v.is_finite()) {
        return Err(Error::from_reason("fit needs twelve numbers"));
    }
    if !(ask.foot > 0.0) || !(ask.tolerance > 0.0) || !ask.fit_rms.is_finite() {
        return Err(Error::from_reason("foot and tolerance need to be above zero"));
    }
    let mut fit = [0.0; 12];
    fit.copy_from_slice(&ask.fit);
    Ok(AsyncTask::new(AlignTask {
        fixed: points(&fixed, "fixed")?,
        moving: points(&moving, "moving")?,
        opts: AlignOpts { fit, fit_rms: ask.fit_rms, level: ask.level, refine: ask.refine, foot: ask.foot, tolerance: ask.tolerance },
        started: Instant::now(),
    }))
}

#[napi(object)]
pub struct PairFound {
    pub i: u32,
    pub j: u32,
    /// The first's sample against the second's surfaces, and each one's surface's facing; then the other way.
    pub ab: Float32Array,
    pub fab: Uint8Array,
    pub ba: Float32Array,
    pub fba: Uint8Array,
}

#[napi(object)]
pub struct CheckFound {
    pub samples: Vec<Float64Array>,
    pub pairs: Vec<PairFound>,
    pub gaps: Vec<Float32Array>,
    pub ms: f64,
}

pub struct CheckTask {
    scans: Vec<Vec<f64>>,
    foot: f64,
    tolerance: f64,
    started: Instant,
}

impl Task for CheckTask {
    type Output = c2c::Checked;
    type JsValue = CheckFound;

    fn compute(&mut self) -> Result<Self::Output> {
        Ok(c2c::check(&self.scans, self.foot, self.tolerance))
    }

    fn resolve(&mut self, _env: Env, c: Self::Output) -> Result<Self::JsValue> {
        Ok(CheckFound {
            samples: c.samples.into_iter().map(Float64Array::from).collect(),
            pairs: c
                .pairs
                .into_iter()
                .map(|p| PairFound {
                    i: p.i as u32,
                    j: p.j as u32,
                    ab: Float32Array::from(p.ab),
                    fab: Uint8Array::from(p.fab),
                    ba: Float32Array::from(p.ba),
                    fba: Uint8Array::from(p.fba),
                })
                .collect(),
            gaps: c.gaps.into_iter().map(Float32Array::from).collect(),
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// Every pair of the scans that meet measured both ways, and each scan's points' gaps, on a worker thread.
#[napi]
pub fn check_clouds(scans: Vec<Float64Array>, foot: f64, tolerance: f64) -> Result<AsyncTask<CheckTask>> {
    if !(foot > 0.0) || !(tolerance > 0.0) {
        return Err(Error::from_reason("foot and tolerance need to be above zero"));
    }
    let mut list = Vec::with_capacity(scans.len());
    for (k, s) in scans.iter().enumerate() {
        list.push(points(s, &format!("scan {k}"))?);
    }
    Ok(AsyncTask::new(CheckTask { scans: list, foot, tolerance, started: Instant::now() }))
}

#[napi(object)]
pub struct NetLayerJs {
    pub inp: u32,
    pub out: u32,
    pub w: u32,
    pub b: u32,
}

#[napi(object)]
pub struct NetModelJs {
    pub branches: Vec<Vec<NetLayerJs>>,
    pub head: Vec<NetLayerJs>,
    pub classes: u32,
    /// A version 2 model's extra inputs, by name (returns, normal, curvature, break).
    pub inputs: Vec<String>,
}

#[napi(object)]
pub struct NetPoints {
    /// Feet.
    pub px: Float64Array,
    pub py: Float64Array,
    pub pz: Float64Array,
    /// 0 to 1.
    pub intensity: Float64Array,
    /// 0 to 255, three a point; none without colour.
    pub rgb: Option<Float64Array>,
    pub return_number: Option<Uint8Array>,
    pub number_of_returns: Option<Uint8Array>,
}

#[napi(object)]
pub struct NetFound {
    pub cls: Uint8Array,
    pub sure: Float32Array,
    pub cubes: u32,
    pub ms: f64,
}

pub struct NetTask {
    model: net::Model,
    px: Vec<f64>,
    py: Vec<f64>,
    pz: Vec<f64>,
    intensity: Vec<f64>,
    rgb: Option<Vec<f64>>,
    returns: Option<(Vec<u8>, Vec<u8>)>,
    started: Instant,
}

impl Task for NetTask {
    type Output = (Vec<u8>, Vec<f32>, usize);
    type JsValue = NetFound;

    fn compute(&mut self) -> Result<Self::Output> {
        let returns = self.returns.as_ref().map(|(n, o)| net::Returns { number: n, of: o });
        Ok(net::classify(&self.model, &self.px, &self.py, &self.pz, &self.intensity, self.rgb.as_deref(), returns, 2048))
    }

    fn resolve(&mut self, _env: Env, (cls, sure, cubes): Self::Output) -> Result<Self::JsValue> {
        Ok(NetFound {
            cls: Uint8Array::from(cls),
            sure: Float32Array::from(sure),
            cubes: cubes as u32,
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn layers(list: &[NetLayerJs]) -> Vec<net::Layer> {
    list.iter().map(|l| net::Layer { inp: l.inp as usize, out: l.out as usize, w: l.w as usize, b: l.b as usize }).collect()
}

/// Geo W. run over a scan's points on a worker thread (every core): each point's class and how sure.
#[napi]
pub fn net_classify(model: NetModelJs, blob: Float32Array, points: NetPoints) -> Result<AsyncTask<NetTask>> {
    if model.branches.len() != 3 {
        return Err(Error::from_reason("the model needs three branches"));
    }
    for name in &model.inputs {
        if !["returns", "normal", "curvature", "break"].contains(&name.as_str()) {
            return Err(Error::from_reason(format!("the model asks for an input the engine doesn't make: {name}")));
        }
    }
    let has = |n: &str| model.inputs.iter().any(|i| i == n);
    let m = net::Model {
        branches: [layers(&model.branches[0]), layers(&model.branches[1]), layers(&model.branches[2])],
        head: layers(&model.head),
        classes: model.classes as usize,
        returns: has("returns"),
        normal: has("normal"),
        curvature: has("curvature"),
        brk: has("break"),
        blob: blob.to_vec(),
    };
    m.check().map_err(Error::from_reason)?;
    let n = points.px.len();
    if points.py.len() != n || points.pz.len() != n || points.intensity.len() != n {
        return Err(Error::from_reason("each point needs x, y, z and intensity"));
    }
    if points.px.iter().chain(points.py.iter()).chain(points.pz.iter()).any(|v| !v.is_finite()) {
        return Err(Error::from_reason("the points need finite places"));
    }
    let rgb = match &points.rgb {
        Some(c) if c.len() == n * 3 => Some(c.to_vec()),
        Some(_) => return Err(Error::from_reason("colour needs three numbers a point")),
        None => None,
    };
    let returns = match (&points.return_number, &points.number_of_returns) {
        (Some(a), Some(b)) if a.len() == n && b.len() == n => Some((a.to_vec(), b.to_vec())),
        (None, None) => None,
        _ => return Err(Error::from_reason("returns need two numbers a point")),
    };
    Ok(AsyncTask::new(NetTask {
        model: m,
        px: points.px.to_vec(),
        py: points.py.to_vec(),
        pz: points.pz.to_vec(),
        intensity: points.intensity.to_vec(),
        rgb,
        returns,
        started: Instant::now(),
    }))
}

/// How far the classification in hand has come, 0 to 1.
#[napi]
pub fn net_progress() -> f64 {
    net::progress()
}

/// A limit box as the page tests it (its turn's cosine and sine worked out there).
#[napi(object)]
pub struct CropBoxJs {
    pub min: Vec<f64>,
    pub max: Vec<f64>,
    pub centre: Vec<f64>,
    pub cos: f64,
    pub sin: f64,
    pub turned: bool,
    pub outside: bool,
    pub slack: Vec<f64>,
}

/// What's cut: the file into the drawing, the boxes, the scan's own grid there, its units' WKT.
#[napi(object)]
pub struct CropAskJs {
    pub affine: Vec<f64>,
    pub boxes: Vec<CropBoxJs>,
    pub setup: Option<u32>,
    pub scale: Vec<f64>,
    pub offset: Vec<f64>,
    pub wkt: String,
    pub laz: bool,
}

#[napi(object)]
pub struct CropFound {
    pub kept: f64,
    pub processed: f64,
    pub min: Vec<f64>,
    pub max: Vec<f64>,
    pub bytes: f64,
    pub ms: f64,
}

pub struct CropTask {
    ix: Arc<Index>,
    ask: CropAsk,
    out: PathBuf,
    started: Instant,
}

impl Task for CropTask {
    type Output = crop::CropDone;
    type JsValue = CropFound;

    fn compute(&mut self) -> Result<Self::Output> {
        crop::crop(&self.ix, &self.ask, &self.out).map_err(|e| {
            let _ = std::fs::remove_file(&self.out);
            Error::from_reason(e)
        })
    }

    fn resolve(&mut self, _env: Env, d: Self::Output) -> Result<Self::JsValue> {
        Ok(CropFound {
            kept: d.kept as f64,
            processed: d.processed as f64,
            min: d.min.to_vec(),
            max: d.max.to_vec(),
            bytes: d.bytes as f64,
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn three_finite(v: &[f64], what: &str) -> Result<[f64; 3]> {
    if v.len() != 3 || v.iter().any(|x| x.is_nan()) {
        return Err(Error::from_reason(format!("{what} needs three numbers")));
    }
    Ok([v[0], v[1], v[2]])
}

/// A scan cut to the limit boxes from its index, written to `out` (made new), on a worker thread.
#[napi(ts_return_type = "Promise<CropFound>")]
pub fn crop_scan(handle: u32, ask: CropAskJs, out: String) -> Result<AsyncTask<CropTask>> {
    let ix = INDEXES
        .lock()
        .unwrap()
        .get(&handle)
        .cloned()
        .ok_or_else(|| Error::from_reason(format!("scan {handle} isn't open")))?;
    if ask.affine.len() != 12 || ask.affine.iter().any(|v| !v.is_finite()) {
        return Err(Error::from_reason("affine needs twelve numbers"));
    }
    if out.is_empty() {
        return Err(Error::from_reason("the cut file needs a place"));
    }
    let mut affine = [0.0; 12];
    affine.copy_from_slice(&ask.affine);
    let mut boxes = Vec::with_capacity(ask.boxes.len());
    for b in &ask.boxes {
        let c = &b.centre;
        if c.len() != 2 || !b.cos.is_finite() || !b.sin.is_finite() {
            return Err(Error::from_reason("a box needs its centre and turn"));
        }
        boxes.push(CropBox {
            min: three_finite(&b.min, "a box's min")?,
            max: three_finite(&b.max, "a box's max")?,
            centre: [c[0], c[1]],
            cos: b.cos,
            sin: b.sin,
            turned: b.turned,
            outside: b.outside,
            slack: three_finite(&b.slack, "a box's slack")?,
        });
    }
    Ok(AsyncTask::new(CropTask {
        ix,
        ask: CropAsk {
            affine,
            boxes,
            setup: ask.setup.map(|k| k.min(u16::MAX as u32) as u16),
            scale: three_finite(&ask.scale, "scale")?,
            offset: three_finite(&ask.offset, "offset")?,
            wkt: ask.wkt,
            laz: ask.laz,
        },
        out: PathBuf::from(out),
        started: Instant::now(),
    }))
}

/// How far the cut in hand has come, 0 to 1.
#[napi]
pub fn crop_progress() -> f64 {
    crop::PROGRESS.load(Ordering::Relaxed) as f64 / 1e6
}

/// A mesh of a render's scene: its corners, triangles, colour (linear 0–1) and whether it's glass.
#[napi(object)]
pub struct RenderMeshJs {
    pub positions: Float64Array,
    pub indices: Uint32Array,
    pub color: Vec<f64>,
    pub glass: bool,
}

/// A light of the drawing, its colour linear.
#[napi(object)]
pub struct RenderLightJs {
    pub kind: String,
    pub position: Vec<f64>,
    pub direction: Vec<f64>,
    pub rgb: Vec<f64>,
    pub intensity: f64,
    pub inner: f64,
    pub outer: f64,
    pub reach: f64,
}

/// A render asked for: the camera, the picture's size, the sun and lights, the scene's size and the seed.
#[napi(object)]
pub struct RenderAskJs {
    pub position: Vec<f64>,
    pub target: Vec<f64>,
    pub fov: f64,
    pub width: u32,
    pub height: u32,
    pub sun: Vec<f64>,
    pub lights: Vec<RenderLightJs>,
    pub sun_off: bool,
    pub size: f64,
    pub seed: u32,
}

static SCENES: LazyLock<Mutex<HashMap<u32, Arc<Scene>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct RenderJob {
    scene: Arc<Scene>,
    cam: Camera,
    settings: Settings,
    seed: u32,
    sum: Vec<f32>,
    passes: u32,
}

static RENDERS: LazyLock<Mutex<HashMap<u32, RenderJob>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn v3(v: &[f64], what: &str) -> Result<[f64; 3]> {
    if v.len() != 3 || v.iter().any(|x| !x.is_finite()) {
        return Err(Error::from_reason(format!("{what} needs three numbers")));
    }
    Ok([v[0], v[1], v[2]])
}

/// A render's scene: its meshes' triangles in a BVH; its handle.
#[napi]
pub fn render_open(meshes: Vec<RenderMeshJs>) -> Result<u32> {
    let mut list = Vec::with_capacity(meshes.len());
    for m in &meshes {
        list.push(Mesh {
            positions: m.positions.to_vec(),
            indices: m.indices.to_vec(),
            color: v3(&m.color, "a mesh's colour")?,
            glass: m.glass,
        });
    }
    let scene = Scene::build(&list);
    let handle = NEXT.fetch_add(1, Ordering::Relaxed);
    SCENES.lock().unwrap().insert(handle, Arc::new(scene));
    Ok(handle)
}

/// A render of a scene started (no passes yet); its job.
#[napi]
pub fn render_start(scene: u32, ask: RenderAskJs) -> Result<u32> {
    let sc = SCENES.lock().unwrap().get(&scene).cloned().ok_or_else(|| Error::from_reason("the render's scene isn't open"))?;
    let (w, h) = (ask.width as usize, ask.height as usize);
    if w == 0 || h == 0 || w * h > 40_000_000 {
        return Err(Error::from_reason("the picture's size isn't usable"));
    }
    let mut lights = Vec::with_capacity(ask.lights.len());
    for l in &ask.lights {
        lights.push(Light {
            kind: match l.kind.as_str() {
                "spot" => LightKind::Spot,
                "distant" => LightKind::Distant,
                _ => LightKind::Point,
            },
            position: v3(&l.position, "a light's position")?,
            direction: v3(&l.direction, "a light's direction")?,
            rgb: v3(&l.rgb, "a light's colour")?,
            intensity: l.intensity,
            inner: l.inner,
            outer: l.outer,
            reach: l.reach,
        });
    }
    let job = RenderJob {
        scene: sc,
        cam: Camera { position: v3(&ask.position, "position")?, target: v3(&ask.target, "target")?, fov: ask.fov },
        settings: Settings { width: w, height: h, sun: v3(&ask.sun, "sun")?, lights, sun_off: ask.sun_off, size: ask.size },
        seed: ask.seed,
        sum: vec![0.0; w * h * 3],
        passes: 0,
    };
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    RENDERS.lock().unwrap().insert(id, job);
    Ok(id)
}

#[napi(object)]
pub struct RenderStepped {
    pub sum: Float32Array,
    pub passes: u32,
    pub ms: f64,
}

pub struct RenderTask {
    id: u32,
    passes: u32,
    started: Instant,
}

impl Task for RenderTask {
    type Output = (Vec<f32>, u32);
    type JsValue = RenderStepped;

    fn compute(&mut self) -> Result<Self::Output> {
        // Taken out while it's traced (a stop meanwhile lets it go after).
        let job = RENDERS.lock().unwrap().remove(&self.id).ok_or_else(|| Error::from_reason("the render was stopped"))?;
        let mut r = rt::Render::new(&job.scene, job.cam, job.settings.clone(), job.seed).resume(job.sum, job.passes);
        for _ in 0..self.passes {
            r.pass();
        }
        let out = (r.sum.clone(), r.passes);
        let sum = std::mem::take(&mut r.sum);
        let passes = r.passes;
        drop(r);
        RENDERS.lock().unwrap().insert(self.id, RenderJob { sum, passes, ..job });
        Ok(out)
    }

    fn resolve(&mut self, _env: Env, (sum, passes): Self::Output) -> Result<Self::JsValue> {
        Ok(RenderStepped { sum: Float32Array::from(sum), passes, ms: self.started.elapsed().as_secs_f64() * 1000.0 })
    }
}

/// `passes` more of a render, on a worker thread (every core): its light summed so far.
#[napi(ts_return_type = "Promise<RenderStepped>")]
pub fn render_step(job: u32, passes: u32) -> Result<AsyncTask<RenderTask>> {
    if !RENDERS.lock().unwrap().contains_key(&job) {
        return Err(Error::from_reason("the render isn't there"));
    }
    Ok(AsyncTask::new(RenderTask { id: job, passes: passes.clamp(1, 1000), started: Instant::now() }))
}

/// A render, or a render's scene, let go.
#[napi]
pub fn render_close(job: Option<u32>, scene: Option<u32>) {
    if let Some(j) = job {
        RENDERS.lock().unwrap().remove(&j);
    }
    if let Some(s) = scene {
        SCENES.lock().unwrap().remove(&s);
    }
}
