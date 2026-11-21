//! Delete Outside Limit Box in the engine (1.28; Leo, 2026-11-18 06:04:
//! "lets build all 8 of you recommendations to make a true desktop app").
//!
//! A scan cut to the drawing's limit boxes from its index on disk, every
//! point of it (the page could only cut the points it had loaded), and the
//! cut written straight to a file on disk, LAS or LAZ, never held whole:
//! one pass counts what's kept and where (on every core), so the file's
//! grid is known before a record's written, a second writes them in turn.
//!
//! Each point is moved into the drawing (its units, ground scale and
//! alignment, as one affine move, as the snaps' are) and tested by the
//! boxes as the page tests them (cloud-limits.ts cropsKeeper: inside, or
//! outside as a box shows, a millionth of slack; several boxes, out of
//! every box showing what's outside and in one of those showing what's
//! inside). The file is written as the page's own writer writes a cut
//! (las-writer.ts): LAS 1.4, point format 7 with colour (else 6), in the
//! drawing's units, on the scan's own grid when it holds the points (else
//! one at them, 0.001 of a unit), its units said in a WKT record; its
//! intensity, returns, class, flags and colour (16-bit colour as the file
//! had it).

use crate::index::Index;
use las::point::{Classification, Format};
use las::{Builder, Color, LazParallelism, Point, Transform, Vector, Vlr, Writer, WriterOptions};
use rayon::prelude::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// How far the cut being made has come, in millionths (one cut at a time per process).
pub static PROGRESS: AtomicU32 = AtomicU32::new(0);

fn said(f: f64) {
    PROGRESS.store((f.clamp(0.0, 1.0) * 1e6) as u32, Ordering::Relaxed);
}

/// A limit box as the page tests it: its turn worked out there (the same cosine and sine).
#[derive(Clone, Debug)]
pub struct CropBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// Its centre in plan, the cosine and sine of its turn (undone), and whether it's turned.
    pub centre: [f64; 2],
    pub cos: f64,
    pub sin: f64,
    pub turned: bool,
    /// It shows what's outside it.
    pub outside: bool,
    /// The slack either way, on each axis.
    pub slack: [f64; 3],
}

impl CropBox {
    /// Whether a place in the drawing is in it (its slack either way).
    #[inline]
    pub fn holds(&self, p: [f64; 3]) -> bool {
        let (mut x, mut y) = (p[0], p[1]);
        if self.turned {
            let dx = x - self.centre[0];
            let dy = y - self.centre[1];
            x = self.centre[0] + dx * self.cos - dy * self.sin;
            y = self.centre[1] + dx * self.sin + dy * self.cos;
        }
        x >= self.min[0] - self.slack[0]
            && x <= self.max[0] + self.slack[0]
            && y >= self.min[1] - self.slack[1]
            && y <= self.max[1] + self.slack[1]
            && p[2] >= self.min[2] - self.slack[2]
            && p[2] <= self.max[2] + self.slack[2]
    }
}

impl CropBox {
    /// Whether a box of the drawing, `lo` to `hi`, may reach into it (said yes to where it only comes near: its corners aren't looked at one by one).
    pub fn reaches(&self, lo: [f64; 3], hi: [f64; 3]) -> bool {
        let (mut cx, mut cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
        let (mut ex, mut ey) = ((hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0);
        if self.turned {
            let dx = cx - self.centre[0];
            let dy = cy - self.centre[1];
            cx = self.centre[0] + dx * self.cos - dy * self.sin;
            cy = self.centre[1] + dx * self.sin + dy * self.cos;
            let (c, s) = (self.cos.abs(), self.sin.abs());
            (ex, ey) = (c * ex + s * ey, s * ex + c * ey);
        }
        cx + ex >= self.min[0] - self.slack[0]
            && cx - ex <= self.max[0] + self.slack[0]
            && cy + ey >= self.min[1] - self.slack[1]
            && cy - ey <= self.max[1] + self.slack[1]
            && hi[2] >= self.min[2] - self.slack[2]
            && lo[2] <= self.max[2] + self.slack[2]
    }
}

/// Whether the boxes keep a point (the page's cropsKeeper; the boxes off left out there).
#[inline]
pub fn keeps(boxes: &[CropBox], p: [f64; 3]) -> bool {
    if boxes.is_empty() {
        return true;
    }
    if boxes.len() == 1 {
        return boxes[0].holds(p) != boxes[0].outside;
    }
    let mut any_inside = false;
    for b in boxes.iter().filter(|b| b.outside) {
        if b.holds(p) {
            return false;
        }
    }
    for b in boxes.iter().filter(|b| !b.outside) {
        any_inside = true;
        if b.holds(p) {
            return true;
        }
    }
    !any_inside
}

/// What's asked.
#[derive(Clone, Debug)]
pub struct CropAsk {
    /// The file's coordinates into the drawing.
    pub affine: [f64; 12],
    pub boxes: Vec<CropBox>,
    /// Only this setup's points (an E57 setup attached alone).
    pub setup: Option<u16>,
    /// The scan's own grid in the drawing (its steps 0 where it has none).
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    /// The drawing's units, said in the file (a WKT).
    pub wkt: String,
    /// Written compressed (LAZ).
    pub laz: bool,
}

/// What was cut.
#[derive(Clone, Debug, Default)]
pub struct CropDone {
    pub kept: u64,
    pub processed: u64,
    /// The kept points' bounds in the drawing.
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub bytes: u64,
}

/// The step written for a scan without one of its own (las-writer.ts LAS_STEP).
pub const LAS_STEP: f64 = 0.001;
const OFFSET_ROUND: f64 = 1000.0;

#[inline]
fn moved(a: &[f64; 12], p: [f64; 3]) -> [f64; 3] {
    [
        a[0] * p[0] + a[1] * p[1] + a[2] * p[2] + a[3],
        a[4] * p[0] + a[5] * p[1] + a[6] * p[2] + a[7],
        a[8] * p[0] + a[9] * p[1] + a[10] * p[2] + a[11],
    ]
}

/// Whether every place from `lo` to `hi` is within a record's 32 bits of `at`.
fn holds_steps(lo: f64, hi: f64, at: f64, step: f64) -> bool {
    ((lo - at) / step).round() >= i32::MIN as f64 && ((hi - at) / step).round() <= i32::MAX as f64
}

/// The grid written on, axis by axis, as las-writer.ts gridFor works it out.
pub fn grid_for(scale: [f64; 3], offset: [f64; 3], lo: [f64; 3], hi: [f64; 3]) -> Result<([f64; 3], [f64; 3]), String> {
    let mut s_out = [0.0; 3];
    let mut o_out = [0.0; 3];
    const AXES: [&str; 3] = ["east to west", "north to south", "in elevation"];
    for a in 0..3 {
        let own = scale[a];
        let has_own = own > 0.0 && own.is_finite();
        let s = if has_own { own } else { LAS_STEP };
        s_out[a] = s;
        let at = offset[a];
        let none = !(lo[a] <= hi[a]);
        if has_own && at.is_finite() && (none || holds_steps(lo[a], hi[a], at, s)) {
            o_out[a] = at;
            continue;
        }
        if none {
            continue;
        }
        let near = (lo[a] / OFFSET_ROUND).floor() * OFFSET_ROUND;
        if !near.is_finite() || !holds_steps(lo[a], hi[a], near, s) {
            return Err(format!(
                "The cut can't be written as a LAS file: its points spread over {:.3} units {}, more than a LAS file holds at a step of {}. Nothing was written: look for stray points far from the rest, or cut them off with a limit box, and try again.",
                hi[a] - lo[a],
                AXES[a],
                s
            ));
        }
        o_out[a] = near;
    }
    Ok((s_out, o_out))
}

/// The day the file's made, as the page's writer dates it (UTC).
fn today() -> Option<chrono::NaiveDate> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    chrono::NaiveDate::from_num_days_from_ce_opt(719_163 + (secs / 86_400) as i32)
}

/// What the first pass finds: the points kept and gone through, their bounds, and whether any has returns.
#[derive(Clone, Copy)]
struct Tally {
    kept: u64,
    processed: u64,
    lo: [f64; 3],
    hi: [f64; 3],
    returns: bool,
}

impl Tally {
    fn none() -> Tally {
        Tally { kept: 0, processed: 0, lo: [f64::INFINITY; 3], hi: [f64::NEG_INFINITY; 3], returns: false }
    }
    fn join(a: Tally, b: Tally) -> Tally {
        let mut t = Tally { kept: a.kept + b.kept, processed: a.processed + b.processed, returns: a.returns || b.returns, ..a };
        for k in 0..3 {
            t.lo[k] = a.lo[k].min(b.lo[k]);
            t.hi[k] = a.hi[k].max(b.hi[k]);
        }
        t
    }
}

const SLAB: u64 = 1 << 20;

/// The scan cut to the boxes, written to `out` (made new); what was cut.
pub fn crop(ix: &Index, ask: &CropAsk, out: &Path) -> Result<CropDone, String> {
    said(0.0);
    let n = ix.count;
    let slabs = n.div_ceil(SLAB).max(1);
    let done = std::sync::atomic::AtomicU64::new(0);
    let a = &ask.affine;
    // First: what's kept, and where.
    let tally = (0..slabs)
        .into_par_iter()
        .map(|s| {
            let mut t = Tally::none();
            for i in s * SLAB..((s + 1) * SLAB).min(n) {
                let r = ix.point(i);
                if ask.setup.is_some_and(|k| k != r.setup) {
                    continue;
                }
                t.processed += 1;
                let p = moved(a, ix.file_xyz(r.x, r.y, r.z));
                if !keeps(&ask.boxes, p) {
                    continue;
                }
                t.kept += 1;
                if r.returns != 0 {
                    t.returns = true;
                }
                for k in 0..3 {
                    t.lo[k] = t.lo[k].min(p[k]);
                    t.hi[k] = t.hi[k].max(p[k]);
                }
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            said(0.3 * d as f64 / slabs as f64);
            t
        })
        .reduce(Tally::none, Tally::join);
    let (scale, offset) = grid_for(ask.scale, ask.offset, tally.lo, tally.hi)?;

    let mut b = Builder::from((1, 4));
    let mut format = Format::new(if ix.has_rgb { 7 } else { 6 }).map_err(|e| e.to_string())?;
    format.is_compressed = ask.laz;
    b.point_format = format;
    b.transforms = Vector {
        x: Transform { scale: scale[0], offset: offset[0] },
        y: Transform { scale: scale[1], offset: offset[1] },
        z: Transform { scale: scale[2], offset: offset[2] },
    };
    b.system_identifier = "EXTRACTION".into();
    b.generating_software = "CAD Protocol".into();
    b.date = today();
    if !ask.wkt.is_empty() {
        b.has_wkt_crs = true;
        let mut data = ask.wkt.clone().into_bytes();
        data.push(0);
        b.vlrs.push(Vlr { user_id: "LASF_Projection".into(), record_id: 2112, description: "Coordinate System WKT".into(), data });
    }
    let header = b.into_header().map_err(|e| e.to_string())?;
    let file = File::create(out).map_err(|e| format!("The cut file couldn't be made: {e}"))?;
    let options = WriterOptions::default().with_laz_parallelism(LazParallelism::Yes);
    let mut w = Writer::with_options(BufWriter::with_capacity(8 << 20, file), header, options).map_err(|e| e.to_string())?;
    // Then each point kept, in turn.
    let mut written: u64 = 0;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..n {
        let r = ix.point(i);
        if ask.setup.is_some_and(|k| k != r.setup) {
            continue;
        }
        let p = moved(a, ix.file_xyz(r.x, r.y, r.z));
        if !keeps(&ask.boxes, p) {
            continue;
        }
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        let (rn, nr) = if tally.returns { (r.returns & 15, r.returns >> 4) } else { (1, 1) };
        let colour = |v: u16| if ix.wide { v } else { v.min(255) * 257 };
        let point = Point {
            x: p[0],
            y: p[1],
            z: p[2],
            intensity: r.intensity,
            return_number: rn,
            number_of_returns: nr,
            classification: Classification::new(r.class).unwrap_or(Classification::Reserved(r.class)),
            is_synthetic: r.flags & 1 != 0,
            is_key_point: r.flags & 2 != 0,
            is_withheld: r.flags & 4 != 0,
            is_overlap: r.flags & 8 != 0,
            gps_time: Some(0.0),
            color: if ix.has_rgb { Some(Color::new(colour(r.rgb[0]), colour(r.rgb[1]), colour(r.rgb[2]))) } else { None },
            ..Default::default()
        };
        w.write_point(point).map_err(|e| format!("The cut file couldn't be written: {e}"))?;
        written += 1;
        if written % SLAB == 0 {
            said(0.3 + 0.7 * written as f64 / tally.kept.max(1) as f64);
        }
    }
    w.close().map_err(|e| format!("The cut file couldn't be finished: {e}"))?;
    drop(w);
    let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    said(1.0);
    Ok(CropDone {
        kept: written,
        processed: tally.processed,
        min: if written > 0 { lo } else { [0.0; 3] },
        max: if written > 0 { hi } else { [0.0; 3] },
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min: [f64; 3], max: [f64; 3], outside: bool) -> CropBox {
        CropBox { min, max, centre: [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0], cos: 1.0, sin: 0.0, turned: false, outside, slack: [1e-6; 3] }
    }

    #[test]
    fn one_box_keeps_inside_or_outside() {
        let b = [square([0.0; 3], [10.0; 3], false)];
        assert!(keeps(&b, [5.0, 5.0, 5.0]));
        assert!(!keeps(&b, [11.0, 5.0, 5.0]));
        let b = [square([0.0; 3], [10.0; 3], true)];
        assert!(!keeps(&b, [5.0, 5.0, 5.0]));
        assert!(keeps(&b, [11.0, 5.0, 5.0]));
    }

    #[test]
    fn several_boxes_as_the_page_tests_them() {
        let b = [square([0.0; 3], [10.0; 3], false), square([4.0; 3], [6.0; 3], true)];
        assert!(keeps(&b, [1.0, 1.0, 1.0]));
        assert!(!keeps(&b, [5.0, 5.0, 5.0]));
        assert!(!keeps(&b, [20.0, 5.0, 5.0]));
        let only_out = [square([0.0; 3], [1.0; 3], true), square([4.0; 3], [6.0; 3], true)];
        assert!(keeps(&only_out, [20.0, 5.0, 5.0]));
    }

    #[test]
    fn a_turned_box_turns_the_point_back() {
        // A box 10 by 2 turned 90°: it runs north and south.
        let a = -90f64.to_radians();
        let b = [CropBox { min: [-5.0, -1.0, -1.0], max: [5.0, 1.0, 1.0], centre: [0.0, 0.0], cos: a.cos(), sin: a.sin(), turned: true, outside: false, slack: [1e-6; 3] }];
        assert!(keeps(&b, [0.0, 4.0, 0.0]));
        assert!(!keeps(&b, [4.0, 0.0, 0.0]));
    }

    #[test]
    fn the_grid_is_the_scans_own_when_it_holds_them() {
        let (s, o) = grid_for([0.01; 3], [100.0; 3], [101.0; 3], [200.0; 3]).unwrap();
        assert_eq!(s, [0.01; 3]);
        assert_eq!(o, [100.0; 3]);
        // No grid of its own: one at the points, 0.001 a step.
        let (s, o) = grid_for([0.0; 3], [0.0; 3], [6_400_123.0, 2_000_456.0, 10.0], [6_401_000.0, 2_001_000.0, 20.0]).unwrap();
        assert_eq!(s, [LAS_STEP; 3]);
        assert_eq!(o, [6_400_000.0, 2_000_000.0, 0.0]);
        assert!(grid_for([0.0; 3], [0.0; 3], [0.0; 3], [5e6, 1.0, 1.0]).is_err());
    }
}
