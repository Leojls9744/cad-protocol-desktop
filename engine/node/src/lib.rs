//! The scan engine as a Node add-on (desktop 1.19), loaded only in the
//! desktop app's engine process (src/engine-host.js), never in a page.
//!
//! openScan(path, cacheDir) indexes a scan (or opens its index made
//! before) on a worker thread and gives its handle; openProgress(path) says
//! how far it's come; snap(scans, ask) finds the points under the cursor
//! (each 14 numbers in a Float64Array: scan, index, x, y, z, pixels from the
//! cursor, nearness to the eye, class, flags, intensity, returns, red,
//! green, blue); closeScan(handle) lets one go.

use cadp_engine_core::{build_or_open, snap as find, Candidate, Index, Progress, ScanIn, SnapQuery};
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
