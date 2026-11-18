//! The scan engine as a Node add-on (desktop 1.19), loaded only in the
//! desktop app's engine process (src/engine-host.js), never in a page.
//!
//! openScan(path, cacheDir) indexes a scan (or opens its index made
//! before) on a worker thread and gives its handle; openProgress(path) says
//! how far it's come; snap(scans, ask) finds the points under the cursor
//! (each 14 numbers in a Float64Array: scan, index, x, y, z, pixels from the
//! cursor, nearness to the eye, class, flags, intensity, returns, red,
//! green, blue); closeScan(handle) lets one go. 1.20: region(handle,
//! affine, min, max, ring, most) gives a scan's points in a box of the
//! drawing, evenly thinned to about `most`. 1.22: alignClouds(fixed,
//! moving, ask) lines one scan's points up on another's after the picked
//! points' fit (point to plane), and checkClouds(scans, foot, tolerance)
//! measures each pair that meets both ways and each scan's points' gaps, on
//! worker threads, as the page's ALIGNSCANS and SCANCHECK do it. 1.23:
//! netClassify(model, blob, points) runs Geo W. over a scan's points on a
//! worker thread (every core), as the page runs it; netProgress() says how
//! far the run in hand has come.

use cadp_engine_core::c2c::{self, AlignOpts, GapStats};
use cadp_engine_core::net;
use cadp_engine_core::{
    build_or_open, region as within, snap as find, Candidate, Index, Progress, RegionQuery, ScanIn, SnapQuery,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
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
    /// The file's own coordinates of its points' box.
    pub min: Vec<f64>,
    pub max: Vec<f64>,
}

pub struct OpenTask {
    path: String,
    cache: String,
    progress: Arc<Progress>,
    started: Instant,
}

impl Task for OpenTask {
    type Output = Index;
    type JsValue = ScanInfo;

    fn compute(&mut self) -> Result<Self::Output> {
        build_or_open(&PathBuf::from(&self.path), &PathBuf::from(&self.cache), &self.progress)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    fn resolve(&mut self, _env: Env, ix: Self::Output) -> Result<Self::JsValue> {
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        let lo = ix.file_xyz(ix.min[0], ix.min[1], ix.min[2]);
        let hi = ix.file_xyz(ix.max[0], ix.max[1], ix.max[2]);
        let info = ScanInfo {
            handle,
            count: ix.count as f64,
            built: ix.built,
            ms: self.started.elapsed().as_secs_f64() * 1000.0,
            has_rgb: ix.has_rgb,
            min: lo.to_vec(),
            max: hi.to_vec(),
        };
        INDEXES.lock().unwrap().insert(handle, Arc::new(ix));
        Ok(info)
    }

    fn finally(self, _env: Env) -> Result<()> {
        OPENING.lock().unwrap().remove(&self.path);
        Ok(())
    }
}

/// A scan indexed (or its index made before opened), on a worker thread.
#[napi]
pub fn open_scan(path: String, cache_dir: String) -> AsyncTask<OpenTask> {
    let progress = Arc::new(Progress::default());
    OPENING.lock().unwrap().insert(path.clone(), progress.clone());
    AsyncTask::new(OpenTask {
        path,
        cache: cache_dir,
        progress,
        started: Instant::now(),
    })
}

/// How far a scan's indexing has come (0 to 1), or -1 when none is.
#[napi]
pub fn open_progress(path: String) -> f64 {
    OPENING.lock().unwrap().get(&path).map(|p| p.get()).unwrap_or(-1.0)
}

/// A scan let go; whether it was open.
#[napi]
pub fn close_scan(handle: u32) -> bool {
    INDEXES.lock().unwrap().remove(&handle).is_some()
}

#[napi(object)]
pub struct SnapScan {
    pub handle: u32,
    /// The file's coordinates into the drawing: 12 numbers, row by row with the shift after each.
    pub affine: Vec<f64>,
    /// The classes a snap may take: 8 numbers of 32 bits (class c is bit c % 32 of number c / 32); all when missing.
    pub allow: Option<Vec<u32>>,
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
    let held: Vec<(Arc<Index>, [f64; 12], [u64; 4])> = {
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
            out.push((ix.clone(), a, allow));
        }
        out
    };
    let ins: Vec<ScanIn> = held
        .iter()
        .map(|(ix, a, allow)| ScanIn { index: ix.as_ref(), affine: *a, allow: *allow })
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
    /// Every point in the box came.
    pub complete: bool,
    pub tried: f64,
    pub cells: f64,
    pub ms: f64,
}

/// A scan's points in a box of the drawing (and in a ring, flat x, y pairs, when given), about `most` at most.
#[napi]
pub fn region(
    handle: u32,
    affine: Vec<f64>,
    min: Vec<f64>,
    max: Vec<f64>,
    ring: Option<Vec<f64>>,
    most: u32,
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
