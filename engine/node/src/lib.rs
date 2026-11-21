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
//! 1.29: classifyStage(ask) does the heavy middle of the page's rules
//! classifier (voxels, their shapes, the forest's votes, whole objects) on
//! a worker thread, every core, as the page does it.
//! 1.28: cropScan(handle, ask, out) cuts a scan to the drawing's limit
//! boxes from its index, every point, written to a LAS or LAZ file on a
//! worker thread; cropProgress() says how far it's come.
//! 1.35 (the layer trial): viewStart(parent) makes the engine a window of
//! its own in the app's frame and a thread that draws scans there on the
//! graphics card; viewShow(ask) says what (the page's view, the scans and
//! where each is in the drawing, or the check's own ring); viewStats() how
//! it's going; viewStop() ends it.

use cadp_engine_core::c2c::{self, AlignOpts, GapStats};
use cadp_engine_core::classify::{self as rules, CloudIn, StageAsk, Tree};
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

/// A limit box as the engine tests it, from the page's.
fn crop_box(b: &CropBoxJs) -> Result<CropBox> {
    let c = &b.centre;
    if c.len() != 2 || !b.cos.is_finite() || !b.sin.is_finite() {
        return Err(Error::from_reason("a box needs its centre and turn"));
    }
    Ok(CropBox {
        min: three_finite(&b.min, "a box's min")?,
        max: three_finite(&b.max, "a box's max")?,
        centre: [c[0], c[1]],
        cos: b.cos,
        sin: b.sin,
        turned: b.turned,
        outside: b.outside,
        slack: three_finite(&b.slack, "a box's slack")?,
    })
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
        boxes.push(crop_box(b)?);
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

/// A scan's points for the classifier, as the page keeps them.
#[napi(object)]
pub struct ClassifyCloudJs {
    pub origin: Vec<f64>,
    pub xyz: Float32Array,
    pub rgb: Option<Uint8Array>,
}

/// What the page worked out before the stage (scan-classify.ts), and the forest, its trees one after another.
#[napi(object)]
pub struct ClassifyAskJs {
    pub clouds: Vec<ClassifyCloudJs>,
    pub x0: f64,
    pub y0: f64,
    pub y1: f64,
    pub z_min: f64,
    pub z_max: f64,
    pub k: f64,
    pub r: f64,
    pub v: f64,
    pub rise: f64,
    pub gz: Float64Array,
    pub cols: u32,
    pub rows: u32,
    pub g: f64,
    pub ground_green: f64,
    pub coloured: bool,
    /// Each tree's nodes: features, thresholds, left, right, classes; `tree_starts` where each begins (and the end).
    pub tree_f: Float64Array,
    pub tree_t: Float64Array,
    pub tree_l: Float64Array,
    pub tree_r: Float64Array,
    pub tree_c: Float64Array,
    pub tree_starts: Vec<u32>,
}

#[napi(object)]
pub struct StageFound {
    pub ground: Vec<Uint8Array>,
    pub vox: Vec<Int32Array>,
    pub hags: Vec<Float32Array>,
    pub n: u32,
    pub cx: Float64Array,
    pub cy: Float64Array,
    pub cz: Float64Array,
    pub ch: Float64Array,
    pub sn: Float64Array,
    pub sg: Float64Array,
    pub nx: Float32Array,
    pub ny: Float32Array,
    pub nz: Float32Array,
    pub curvature: Float32Array,
    pub break_deg: Float32Array,
    pub label: Int8Array,
    pub choices: Int8Array,
    pub share: Float32Array,
    pub forest_top: Int8Array,
    pub scatter: Float32Array,
    pub flat: Float32Array,
    pub face_x: Float32Array,
    pub face_y: Float32Array,
    pub members: Uint32Array,
    pub starts: Uint32Array,
    pub ms: f64,
}

struct OwnedCloud {
    origin: [f64; 3],
    xyz: Vec<f32>,
    rgb: Option<Vec<u8>>,
}

pub struct StageTask {
    clouds: Vec<OwnedCloud>,
    nums: [f64; 12],
    cols: usize,
    rows: usize,
    coloured: bool,
    gz: Vec<f64>,
    trees: Vec<Tree>,
    started: Instant,
}

impl Task for StageTask {
    type Output = rules::Stage;
    type JsValue = StageFound;

    fn compute(&mut self) -> Result<Self::Output> {
        let [x0, y0, y1, z_min, z_max, k, r, v, rise, g, ground_green, _] = self.nums;
        let ask = StageAsk {
            clouds: self.clouds.iter().map(|c| CloudIn { origin: c.origin, xyz: &c.xyz, rgb: c.rgb.as_deref() }).collect(),
            x0,
            y0,
            y1,
            z_min,
            z_max,
            k,
            r,
            v,
            rise,
            gz: &self.gz,
            cols: self.cols,
            rows: self.rows,
            g,
            ground_green,
            coloured: self.coloured,
            trees: &self.trees,
        };
        Ok(rules::stage(&ask))
    }

    fn resolve(&mut self, _env: Env, st: Self::Output) -> Result<Self::JsValue> {
        Ok(StageFound {
            ground: st.ground.into_iter().map(Uint8Array::from).collect(),
            vox: st.vox.into_iter().map(Int32Array::from).collect(),
            hags: st.hags.into_iter().map(Float32Array::from).collect(),
            n: st.n as u32,
            cx: Float64Array::from(st.cx),
            cy: Float64Array::from(st.cy),
            cz: Float64Array::from(st.cz),
            ch: Float64Array::from(st.ch),
            sn: Float64Array::from(st.sn),
            sg: Float64Array::from(st.sg),
            nx: Float32Array::from(st.shapes.nx),
            ny: Float32Array::from(st.shapes.ny),
            nz: Float32Array::from(st.shapes.nz),
            curvature: Float32Array::from(st.shapes.curvature),
            break_deg: Float32Array::from(st.shapes.break_deg),
            label: Int8Array::from(st.label),
            choices: Int8Array::from(st.choices),
            share: Float32Array::from(st.share),
            forest_top: Int8Array::from(st.forest_top),
            scatter: Float32Array::from(st.scatter),
            flat: Float32Array::from(st.flat),
            face_x: Float32Array::from(st.face_x),
            face_y: Float32Array::from(st.face_y),
            members: Uint32Array::from(st.members),
            starts: Uint32Array::from(st.starts),
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// The rules classifier's heavy middle (its voxels to its whole objects), on a worker thread, every core.
#[napi(ts_return_type = "Promise<StageFound>")]
pub fn classify_stage(ask: ClassifyAskJs) -> Result<AsyncTask<StageTask>> {
    let mut clouds = Vec::with_capacity(ask.clouds.len());
    for c in &ask.clouds {
        if c.origin.len() != 3 || c.xyz.len() % 3 != 0 {
            return Err(Error::from_reason("a scan needs its origin and x, y, z"));
        }
        let rgb = c.rgb.as_ref().map(|r| r.to_vec());
        if rgb.as_ref().is_some_and(|r| r.len() != c.xyz.len()) {
            return Err(Error::from_reason("a scan's colours don't match its points"));
        }
        clouds.push(OwnedCloud { origin: [c.origin[0], c.origin[1], c.origin[2]], xyz: c.xyz.to_vec(), rgb });
    }
    let (cols, rows) = (ask.cols as usize, ask.rows as usize);
    if cols == 0 || rows == 0 || ask.gz.len() != cols * rows {
        return Err(Error::from_reason("the ground grid doesn't match its size"));
    }
    let starts = &ask.tree_starts;
    let mut trees = Vec::new();
    for w in starts.windows(2) {
        let (a, b) = (w[0] as usize, w[1] as usize);
        if a >= b || b > ask.tree_f.len() {
            return Err(Error::from_reason("the forest's trees don't fit"));
        }
        let ints = |v: &Float64Array| v[a..b].iter().map(|&x| x as i32).collect::<Vec<i32>>();
        let tree = Tree { f: ints(&ask.tree_f), t: ask.tree_t[a..b].to_vec(), l: ints(&ask.tree_l), r: ints(&ask.tree_r), c: ints(&ask.tree_c) };
        // Every node it goes to is in it, and every class one of the forest's.
        let len = (b - a) as i32;
        if tree.f.iter().zip(tree.l.iter().zip(tree.r.iter())).any(|(&f, (&l, &r))| f >= 10 || (f >= 0 && (l < 0 || l >= len || r < 0 || r >= len)))
            || tree.c.iter().any(|&c| !(0..7).contains(&c))
        {
            return Err(Error::from_reason("a tree of the forest isn't whole"));
        }
        trees.push(tree);
    }
    if trees.is_empty() || !(ask.r > 0.0) || !(ask.v > 0.0) || !(ask.k > 0.0) || !(ask.g > 0.0) {
        return Err(Error::from_reason("the classifier's sizes aren't usable"));
    }
    Ok(AsyncTask::new(StageTask {
        clouds,
        nums: [ask.x0, ask.y0, ask.y1, ask.z_min, ask.z_max, ask.k, ask.r, ask.v, ask.rise, ask.g, ask.ground_green, 0.0],
        cols,
        rows,
        coloured: ask.coloured,
        gz: ask.gz.to_vec(),
        trees,
        started: Instant::now(),
    }))
}

// ---------------------------------------------------------------------------
// 1.35: the engine's own drawing (cadp-engine-view), in the app's frame.
// ---------------------------------------------------------------------------

static VIEW: LazyLock<Mutex<Option<cadp_engine_view::View>>> = LazyLock::new(|| Mutex::new(None));
/// The page's own points of the scans it has classified or edited (1.36), by the scan's handle: where they are, then their
/// classes and what's taken out as they are now. A file's setups attached each as its own scan share a handle and have
/// each their own points, so a handle keeps several (the newest BORROWS_KEPT of them).
static BORROW_POINTS: LazyLock<Mutex<HashMap<u32, Vec<Arc<cadp_engine_view::BorrowPoints>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BORROWS: LazyLock<Mutex<HashMap<u32, Vec<Arc<cadp_engine_view::Borrow>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
const BORROWS_KEPT: usize = 16;

/// A scan's points kept for its handle, in place of any with the same key.
fn keep_borrow_points(handle: u32, points: Arc<cadp_engine_view::BorrowPoints>) {
    let mut held = BORROW_POINTS.lock().unwrap();
    let list = held.entry(handle).or_default();
    list.retain(|p| p.key != points.key);
    list.push(points);
    if list.len() > BORROWS_KEPT {
        list.remove(0);
    }
}

/// A scan's classes kept for its handle, in place of the classes its points had before.
fn keep_borrow(handle: u32, borrow: Arc<cadp_engine_view::Borrow>) {
    let mut held = BORROWS.lock().unwrap();
    let list = held.entry(handle).or_default();
    list.retain(|b| b.points.key != borrow.points.key);
    list.push(borrow);
    if list.len() > BORROWS_KEPT {
        list.remove(0);
    }
}

/// The shapes isolating scan points (1.37), by their key: the newest few kept (the page gives them once, then names them with each view).
static SHAPES: LazyLock<Mutex<Vec<Arc<cadp_engine_view::Shapes>>>> = LazyLock::new(|| Mutex::new(Vec::new()));
const SHAPES_KEPT: usize = 8;

/// A shape isolating scan points (1.37), as the page keeps it: its view's across and up, its corners across and up it, what it does to the points kept before it ("and", "out", or none: adds), a brushed one's dabs and their cuts toward the eye, and the classes off as it was drawn.
#[napi(object)]
pub struct ViewShapeJs {
    pub right: Vec<f64>,
    pub up: Vec<f64>,
    pub poly: Float64Array,
    pub op: Option<String>,
    pub dabs: Option<Float64Array>,
    pub near: Option<Float64Array>,
    pub off: Option<Vec<u32>>,
}

/// The shapes isolating scan points, kept under `key` for the views that name it.
#[napi]
pub fn view_shapes(key: String, shapes: Vec<ViewShapeJs>) -> Result<bool> {
    let mut list = Vec::with_capacity(shapes.len());
    for s in shapes {
        let (Some(right), Some(up)) = (view_three(&Some(s.right)), view_three(&Some(s.up))) else {
            return Err(Error::from_reason("a shape needs its view's across and up"));
        };
        if s.poly.iter().any(|v| !v.is_finite()) || s.dabs.as_ref().is_some_and(|d| d.len() % 3 != 0 || d.iter().any(|v| !v.is_finite())) {
            return Err(Error::from_reason("a shape's corners and dabs must be numbers"));
        }
        let op = match s.op.as_deref() {
            Some("and") => cadp_engine_view::Op::And,
            Some("out") => cadp_engine_view::Op::Out,
            _ => cadp_engine_view::Op::Add,
        };
        list.push(cadp_engine_view::ShapeIn {
            right,
            up,
            poly: s.poly.to_vec(),
            op,
            dabs: s.dabs.map(|d| d.to_vec()).unwrap_or_default(),
            near: s.near.map(|n| n.to_vec()),
            off: s.off.unwrap_or_default().into_iter().filter(|c| *c < 256).map(|c| c as u8).collect(),
        });
    }
    let made = Arc::new(cadp_engine_view::Shapes::new(key, list));
    let mut held = SHAPES.lock().unwrap();
    held.retain(|s| s.key != made.key);
    held.push(made);
    if held.len() > SHAPES_KEPT {
        held.remove(0);
    }
    Ok(true)
}

/// How a scan's points look (1.36), as the page has them.
#[napi(object)]
pub struct ViewLookJs {
    /// 0 their own colours, 1 by class, 2 by height, 3 by intensity, 4 one colour.
    pub by: Option<u32>,
    /// A point's size, pixels.
    pub px: Option<f64>,
    /// Discs, not squares.
    pub round: Option<bool>,
    /// The one colour: red, green, blue, 0 to 1.
    pub one: Option<Vec<f64>>,
    /// The drawing's heights the height colours run over: low, high.
    pub heights: Option<Vec<f64>>,
    /// The intensities the greys run over: low, high.
    pub intensity: Option<Vec<f64>>,
    /// Intensity mixed into the colours: read on low to high, and its share 0 to 1.
    pub mix: Option<Vec<f64>>,
    /// Each class's colour and whether its layer shows: 256 numbers, red | green << 8 | blue << 16 | shown << 24.
    pub classes: Option<Vec<u32>>,
    /// The shading from the shape of the points' surface (1.37), each 0 to 100 as the page's settings: lighting, detail and hollows. Any over 0: worked out as the points are read.
    pub shade: Option<Vec<f64>>,
    /// Fill gaps (1.37): each point as big as its spacing where it is, up to this many pixels. None or 0: off.
    pub fill: Option<f64>,
}

/// A scan to draw: its handle, its file's coordinates into the drawing (12 numbers), about how many points at most, which of them and how they look.
#[napi(object)]
pub struct ViewScanJs {
    pub handle: u32,
    pub affine: Vec<f64>,
    pub most: Option<f64>,
    /// 1.35's way of leaving classes out (eight 32-bit numbers, bit c for class c): read as those classes' layers off, where no look is given.
    pub allow: Option<Vec<u32>>,
    /// An E57's one setup alone.
    pub setup: Option<u32>,
    /// An E57's setups turned off.
    pub off: Option<Vec<u32>>,
    /// The class each of the file's 256 codes is read as (the scan's class map).
    pub map: Option<Vec<u32>>,
    /// The page's own classes and removed points for it (view_borrow's key): drawn once they're in.
    pub borrow: Option<String>,
    pub look: Option<ViewLookJs>,
}

/// What to draw: the page's view (where its drawing area's middle looks, right, up and toward the eye, drawing units a
/// pixel, and the drawing area in the window: left, top, width, height), the scans; or the check's ring.
#[napi(object)]
pub struct ViewAskJs {
    pub target: Option<Vec<f64>>,
    pub right: Option<Vec<f64>>,
    pub up: Option<Vec<f64>>,
    pub to_eye: Option<Vec<f64>>,
    pub units_per_pixel: Option<f64>,
    pub rect: Option<Vec<f64>>,
    /// 1.35's point size for every scan (a scan's look has its own since).
    pub px: Option<f64>,
    pub test: Option<bool>,
    pub scans: Option<Vec<ViewScanJs>>,
    /// The background: red, green, blue, 0 to 1.
    pub clear: Option<Vec<f64>>,
    /// The scans kept on the card, none drawn.
    pub hide: Option<bool>,
    /// The page's own number for this ask (said back as `drawn` once a frame has shown it).
    pub serial: Option<f64>,
    /// Eye-dome lighting (1.36): its strength, its radius and the eye's distance (pixels), the depth noise ignored (drawing units).
    pub edl: Option<Vec<f64>>,
    /// How long the ask is held before it's shown, ms (1.36): the page's own picture lands that much later.
    pub hold_ms: Option<f64>,
    /// The shading from the picture's depths (1.37), each as the page's own factor: depth shading (shadingFactor), relief (reliefFactor) and face shading (faceFactor).
    pub shade: Option<Vec<f64>>,
    /// How much of their strength the scans are drawn at (1.37; the page's Show switches' faint: 0.3), the background through the rest. None: all.
    pub strength: Option<f64>,
    /// The limit boxes that are on (1.37), as the page tests them, and the slice's slab as one: only what they show is drawn.
    pub boxes: Option<Vec<CropBoxJs>>,
    pub slice: Option<CropBoxJs>,
    /// The points isolated (1.37): the key of the shapes given with view_shapes. Scans are left out till they're in.
    pub isolate: Option<String>,
    /// Which of the frame's views this is (1.37): 0 or none the drawing area, 1 the split view's other side.
    pub slot: Option<u32>,
}

#[napi(object)]
pub struct ViewStatsJs {
    pub started: bool,
    pub running: bool,
    pub frames: f64,
    pub frame_ms: f64,
    pub fps: f64,
    /// The longest frame of the last run of frames, ms.
    pub worst_ms: f64,
    pub points: f64,
    pub load_ms: f64,
    /// Of that, working out the points' shading from their surfaces, ms (1.37).
    pub shade_ms: f64,
    pub width: u32,
    pub height: u32,
    pub adapter: String,
    pub error: Option<String>,
    /// How many of the asked scans the last frame drew.
    pub scans: u32,
    /// How many of them with the detail for its view (1.36).
    pub detail: u32,
    /// The serial of the ask the last frame showed.
    pub drawn: f64,
    /// Those three for each of the frame's views (1.37), three numbers a view: the drawing area's first.
    pub views: Vec<f64>,
}

fn view_three(v: &Option<Vec<f64>>) -> Option<[f64; 3]> {
    let v = v.as_ref()?;
    (v.len() == 3 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1], v[2]])
}

fn view_two(v: &Option<Vec<f64>>) -> Option<[f64; 2]> {
    let v = v.as_ref()?;
    (v.len() == 2 && v.iter().all(|x| x.is_finite())).then(|| [v[0], v[1]])
}

/// A scan's look as the page gave it; what it didn't say is as 1.35 drew (its own colours, squares, every class).
fn view_look(look: &Option<ViewLookJs>, allow: &Option<Vec<u32>>, px: Option<f64>) -> cadp_engine_view::Look {
    let mut out = cadp_engine_view::Look::own();
    if let Some(p) = px.filter(|p| p.is_finite() && *p > 0.0) {
        out.px = p;
    }
    // 1.35's classes left out: their layers off.
    if let Some(bits) = allow.as_ref().filter(|b| b.len() == 8) {
        for (c, entry) in out.classes.iter_mut().enumerate() {
            if bits[c >> 5] >> (c & 31) & 1 == 0 {
                entry[3] = 0;
            }
        }
    }
    let Some(l) = look else { return out };
    if let Some(by) = l.by.filter(|b| *b <= cadp_engine_view::BY_ONE) {
        out.by = by;
    }
    if let Some(p) = l.px.filter(|p| p.is_finite() && *p > 0.0) {
        out.px = p.min(64.0);
    }
    out.round = l.round.unwrap_or(false);
    if let Some(c) = view_three(&l.one) {
        out.one = [c[0] as f32, c[1] as f32, c[2] as f32];
    }
    if let Some(h) = view_two(&l.heights) {
        out.heights = h;
    }
    if let Some(i) = view_two(&l.intensity) {
        out.intensity = [i[0] as f32, i[1] as f32];
    }
    if let Some(m) = view_three(&l.mix) {
        out.mix = [m[0] as f32, m[1] as f32, m[2] as f32];
    }
    if let Some(classes) = l.classes.as_ref().filter(|c| c.len() == 256) {
        for (entry, v) in out.classes.iter_mut().zip(classes) {
            *entry = [(*v & 255) as u8, (*v >> 8 & 255) as u8, (*v >> 16 & 255) as u8, (*v >> 24 & 255) as u8];
        }
    }
    if let Some(f) = l.fill.filter(|f| f.is_finite() && *f > 0.0) {
        out.fill = f.min(64.0);
    }
    if let Some(sh) = view_three(&l.shade) {
        out.shade = [(sh[0] / 100.0).clamp(0.0, 1.0) as f32, (sh[1] / 100.0).clamp(0.0, 1.0) as f32, (sh[2] / 100.0).clamp(0.0, 1.0) as f32];
    }
    out
}

/// The engine's window started in the frame whose handle is `parent` (a whole number, as text).
#[napi]
pub fn view_start(parent: String) -> Result<()> {
    let hwnd: i64 = parent.trim().parse().map_err(|_| Error::from_reason("the frame's handle isn't a number"))?;
    let mut held = VIEW.lock().unwrap();
    // One at a time: a new frame's takes the old one's place.
    if let Some(old) = held.take() {
        old.stop();
    }
    let view = cadp_engine_view::View::start(hwnd as isize).map_err(Error::from_reason)?;
    *held = Some(view);
    Ok(())
}

/// The page's own points of a scan it has classified or edited (1.36): where
/// they are in the drawing (`xyz` from `origin`; `cell` the size it files
/// them by), their classes and which are taken out. The scan's points on
/// the card take the class of the nearest, and are left out where it's
/// taken out. `xyz` needn't come again while `points_key` is the same
/// (their classes change far more often than where they are): false says
/// it's needed. They're filed on a thread of their own; a view that names
/// `key` draws that scan once they're in.
#[napi]
pub fn view_borrow(
    handle: u32,
    key: String,
    points_key: String,
    cell: f64,
    origin: Vec<f64>,
    xyz: Option<Float32Array>,
    classification: Uint8Array,
    removed: Option<Uint8Array>,
) -> Result<bool> {
    if origin.len() != 3 || origin.iter().any(|v| !v.is_finite()) {
        return Err(Error::from_reason("the points' origin needs three numbers"));
    }
    let class = classification.to_vec();
    let removed = removed.map(|r| r.to_vec());
    let have = BORROW_POINTS.lock().unwrap().get(&handle).and_then(|list| list.iter().find(|p| p.key == points_key).cloned());
    match (have, xyz) {
        (Some(points), _) => {
            let Some(borrow) = cadp_engine_view::Borrow::new(key, points, class, removed) else {
                return Err(Error::from_reason("the classes aren't as many as the points"));
            };
            keep_borrow(handle, Arc::new(borrow));
            Ok(true)
        }
        (None, None) => Ok(false),
        (None, Some(xyz)) => {
            if xyz.len() % 3 != 0 || xyz.len() / 3 != class.len() || removed.as_ref().is_some_and(|r| r.len() != class.len()) {
                return Err(Error::from_reason("the classes aren't as many as the points"));
            }
            let xyz = xyz.to_vec();
            let origin = [origin[0], origin[1], origin[2]];
            std::thread::spawn(move || {
                let points = Arc::new(cadp_engine_view::BorrowPoints::new(points_key, cell, origin, xyz));
                if let Some(borrow) = cadp_engine_view::Borrow::new(key, points.clone(), class, removed) {
                    keep_borrow_points(handle, points);
                    keep_borrow(handle, Arc::new(borrow));
                }
            });
            Ok(true)
        }
    }
}

/// What the engine's window shows from now on. Scans whose handles aren't open, or whose own classes (`borrow`) aren't in yet, are left out.
#[napi]
pub fn view_show(ask: ViewAskJs) -> Result<u32> {
    let held = VIEW.lock().unwrap();
    let Some(view) = held.as_ref() else {
        return Err(Error::from_reason("the engine's window isn't started"));
    };
    let camera = match (view_three(&ask.target), view_three(&ask.right), view_three(&ask.up), view_three(&ask.to_eye), ask.units_per_pixel, &ask.rect) {
        (Some(target), Some(right), Some(up), Some(to_eye), Some(upp), Some(rect))
            if upp > 0.0 && upp.is_finite() && rect.len() == 4 && rect.iter().all(|x| x.is_finite()) =>
        {
            Some(cadp_engine_view::Camera { target, right, up, to_eye, units_per_pixel: upp, rect: [rect[0], rect[1], rect[2], rect[3]] })
        }
        _ => None,
    };
    let indexes = INDEXES.lock().unwrap();
    let borrows = BORROWS.lock().unwrap();
    // The points isolated: by the shapes the page gave under this key (none in yet: no scan drawn till they are).
    let isolate = match &ask.isolate {
        Some(key) => match SHAPES.lock().unwrap().iter().find(|s| &s.key == key) {
            Some(s) => Some(Some(s.clone())),
            None => None,
        },
        None => Some(None),
    };
    let mut scans = Vec::new();
    for s in ask.scans.unwrap_or_default() {
        let Some(isolate) = &isolate else { break };
        let Some(index) = indexes.get(&s.handle) else { continue };
        if s.affine.len() != 12 || s.affine.iter().any(|x| !x.is_finite()) {
            continue;
        }
        let mut affine = [0.0; 12];
        affine.copy_from_slice(&s.affine);
        let most = s.most.filter(|m| m.is_finite() && *m >= 1.0).unwrap_or(16_000_000.0).min(64_000_000.0) as usize;
        let mut keep = cadp_engine_view::Keep::all();
        keep.setup = s.setup.map(|k| k.min(u16::MAX as u32) as u16);
        keep.off = s.off.as_ref().map(|v| v.iter().map(|k| (*k).min(u16::MAX as u32) as u16).collect()).unwrap_or_default();
        if let Some(map) = s.map.as_ref().filter(|m| m.len() == 256) {
            let mut table = [0u8; 256];
            for (t, v) in table.iter_mut().zip(map) {
                *t = (*v).min(255) as u8;
            }
            keep.map = Some(Arc::new(table));
        }
        if let Some(key) = &s.borrow {
            // Its own classes aren't in yet (still being filed, or never sent): not drawn till they are.
            let Some(b) = borrows.get(&s.handle).and_then(|list| list.iter().find(|b| &b.key == key)) else { continue };
            keep.borrow = Some(b.clone());
        }
        let mut look = view_look(&s.look, &s.allow, ask.px);
        // No look given (1.35's page): a scan with no colours of its own by its own heights, as 1.35 drew it.
        if s.look.is_none() {
            let lo = index.file_xyz(index.min[0], index.min[1], index.min[2]);
            let hi = index.file_xyz(index.max[0], index.max[1], index.max[2]);
            look.heights = [affine[10] * lo[2] + affine[11], affine[10] * hi[2] + affine[11]];
        }
        // Lit by the shape of its surface: that's worked out as its points are read.
        keep.shade = look.shade.iter().any(|v| *v > 0.0);
        // Gaps filled: each point's spacing is measured as they're read.
        keep.gaps = look.fill > 0.0;
        keep.isolate = isolate.clone();
        scans.push(cadp_engine_view::ScanAsk { handle: s.handle, index: index.clone(), affine, most, keep: keep.keyed(), look });
    }
    let shown = scans.len() as u32;
    let serial = ask.serial.filter(|n| n.is_finite() && *n >= 0.0).unwrap_or(0.0) as u64;
    let edl = match ask.edl.as_ref().filter(|e| e.len() == 4 && e.iter().all(|v| v.is_finite())) {
        Some(e) => cadp_engine_view::Edl { strength: e[0], radius: e[1], eye: e[2], noise: e[3] },
        None => cadp_engine_view::Edl::default(),
    };
    let shading = match view_three(&ask.shade) {
        Some(s) => cadp_engine_view::Shading { depth: s[0].max(0.0), relief: s[1].max(0.0), faces: s[2].clamp(0.0, 1.0), fade: 0.0 },
        None => cadp_engine_view::Shading::default(),
    };
    let shading = cadp_engine_view::Shading { fade: ask.strength.filter(|v| v.is_finite()).map_or(0.0, |v| 1.0 - v.clamp(0.0, 1.0)), ..shading };
    // The limit boxes and the slice (one not as the engine reads it: none of them, and the page's own count of scans drawn says so).
    let mut clip = cadp_engine_view::Clip::default();
    for b in ask.boxes.iter().flatten().take(cadp_engine_view::CLIP_BOXES) {
        clip.boxes.push(crop_box(b)?);
    }
    if let Some(b) = &ask.slice {
        clip.slice = Some(crop_box(b)?);
    }
    view.show_in(ask.slot.unwrap_or(0) as usize, cadp_engine_view::Ask {
        camera,
        scans,
        test: ask.test.unwrap_or(false),
        clear: view_three(&ask.clear),
        hide: ask.hide.unwrap_or(false),
        serial,
        edl,
        shading,
        clip: clip.keyed(),
        hold_ms: ask.hold_ms.filter(|h| h.is_finite() && *h > 0.0).unwrap_or(0.0),
    });
    Ok(shown)
}

#[napi]
pub fn view_stats() -> ViewStatsJs {
    let held = VIEW.lock().unwrap();
    let s = held.as_ref().map(|v| v.stats()).unwrap_or_default();
    ViewStatsJs {
        started: held.is_some(),
        running: s.running,
        frames: s.frames as f64,
        frame_ms: s.frame_ms,
        fps: s.fps,
        worst_ms: s.worst_ms,
        points: s.points as f64,
        load_ms: s.load_ms,
        shade_ms: s.shade_ms,
        width: s.size[0],
        height: s.size[1],
        adapter: s.adapter,
        error: s.error,
        scans: s.scans,
        detail: s.detail,
        drawn: s.drawn as f64,
        views: s.views.iter().flatten().map(|v| *v as f64).collect(),
    }
}

#[napi]
pub fn view_stop() {
    if let Some(view) = VIEW.lock().unwrap().take() {
        view.stop();
    }
}
