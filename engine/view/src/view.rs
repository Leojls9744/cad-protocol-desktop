//! The engine's own window, and the threads that draw in it.
//!
//! The window is a child of the app's frame (its handle given by the app):
//! it fills the frame's inside, takes no clicks or keys (the app's page is
//! over it and takes them all), and is drawn by a thread of its own, so a
//! frame never waits on the page and the page never waits on a frame. The
//! page says what to show (`Ask`: its view, the scans, where each is in
//! the drawing and how it looks); the thread draws when something's
//! changed, and keeps count (`Stats`).
//!
//! What's on the card for each scan is the `Stage`'s: an even sample of the
//! whole scan first (its base, there whichever way the view turns), then
//! the detail for the view (the cells the view sees, at the share of their
//! points the scan's budget allows: every point, zoomed in far enough),
//! read by a loader thread on every core while frames go on, and read
//! again when the view sees cells it hasn't or wants a good deal more of
//! them.

use crate::camera::{Affine, Camera};
use crate::gpu::{Cloud, Edl, Look, Sender};
use crate::scene::{sample, wanted, Keep, Sight, Taken};
use cadp_engine_core::Index;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A scan to show: its index (by the engine's handle), where its file's
/// coordinates are in the drawing, about how many points at most, which of
/// them (`keep`) and how they look.
#[derive(Clone)]
pub struct ScanAsk {
    pub handle: u32,
    pub index: Arc<Index>,
    pub affine: Affine,
    pub most: usize,
    pub keep: Keep,
    pub look: Look,
}

/// What to show.
#[derive(Clone, Default)]
pub struct Ask {
    /// The page's view; with none, nothing but the background (or the check's ring, turning).
    pub camera: Option<Camera>,
    pub scans: Vec<ScanAsk>,
    /// The check's own ring of points, with no scan.
    pub test: bool,
    /// The background (red, green, blue, 0 to 1); the engine's own with none.
    pub clear: Option<[f64; 3]>,
    /// The scans kept on the card and not drawn (the page is drawing them itself for now).
    pub hide: bool,
    /// The page's own number for this ask: said back in `Stats::drawn` once a frame has shown it.
    pub serial: u64,
    /// Eye-dome lighting, as the page has it.
    pub edl: Edl,
    /// How long it's held before it's shown, ms: the page's own picture
    /// (its linework, over the scans) reaches the screen a refresh or so
    /// after the page paints it, and the two are to land together.
    pub hold_ms: f64,
}

/// How it's going.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub running: bool,
    /// Frames drawn since it started.
    pub frames: u64,
    /// The last frame's drawing, as the thread saw it, ms (its calls and the wait to show it).
    pub frame_ms: f64,
    /// Frames a second while it last drew one after another (a run of frames; a rest between isn't counted).
    pub fps: f64,
    /// The longest frame of that run, ms.
    pub worst_ms: f64,
    /// Points on the card.
    pub points: u64,
    /// The last sample's reading and sending, ms.
    pub load_ms: f64,
    pub size: [u32; 2],
    pub adapter: String,
    pub error: Option<String>,
    /// How many of the asked scans the last frame drew.
    pub scans: u32,
    /// How many of them with the detail for its view.
    pub detail: u32,
    /// The serial of the ask the last frame showed.
    pub drawn: u64,
}

/// What tells one scan's points on the card from another's: the scan, how many, and which of them.
type ScanKey = (u32, usize, String);

fn key_of(s: &ScanAsk) -> ScanKey {
    (s.handle, s.most, s.keep.key.clone())
}

/// A scan's base is this share of its points' budget (the rest is for the view's detail), and never under BASE_LEAST where the budget's that much.
pub const BASE_SHARE: usize = 6;
pub const BASE_LEAST: usize = 1_000_000;
/// How often a still-changing view is looked at for new detail, ms.
pub const DETAIL_EVERY_MS: u64 = 90;

fn base_most(most: usize) -> usize {
    (most / BASE_SHARE).max(BASE_LEAST.min(most)).max(1)
}

struct Job {
    key: ScanKey,
    index: Arc<Index>,
    affine: Affine,
    keep: Keep,
    most: usize,
    /// The view its detail is for; none: its base.
    camera: Option<Camera>,
}

struct Done {
    key: ScanKey,
    detail: bool,
    cloud: Cloud,
    taken: Taken,
    ms: f64,
}

struct LoaderShared {
    job: Mutex<Option<Job>>,
    wake: Condvar,
    done: Mutex<Vec<Done>>,
    busy: AtomicBool,
    stop: AtomicBool,
    failed: Mutex<Option<String>>,
}

/// The thread that reads samples and sends them to the card, one at a time, while frames go on.
struct Loader {
    shared: Arc<LoaderShared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Loader {
    fn start(sender: Sender) -> Result<Loader, String> {
        let shared = Arc::new(LoaderShared {
            job: Mutex::new(None),
            wake: Condvar::new(),
            done: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            failed: Mutex::new(None),
        });
        let theirs = shared.clone();
        let thread = std::thread::Builder::new()
            .name("cadp-view-load".into())
            .spawn(move || {
                let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loader_run(&theirs, &sender)));
                if let Err(p) = ended {
                    let said = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    *theirs.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("reading a scan for the card stopped: {}", said.unwrap_or_else(|| "it didn't say why".into())));
                    theirs.busy.store(false, Ordering::SeqCst);
                }
            })
            .map_err(|e| format!("couldn't start the thread that reads scans for the card: {e}"))?;
        Ok(Loader { shared, thread: Some(thread) })
    }

    fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::SeqCst)
    }

    fn give(&self, job: Job) {
        self.shared.busy.store(true, Ordering::SeqCst);
        *self.shared.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
        self.shared.wake.notify_one();
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn loader_run(shared: &LoaderShared, sender: &Sender) {
    loop {
        let job = {
            let mut held = shared.job.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(job) = held.take() {
                    break job;
                }
                held = shared.wake.wait_timeout(held, Duration::from_millis(50)).unwrap_or_else(|e| e.into_inner()).0;
            }
        };
        let t = Instant::now();
        let sight = job.camera.as_ref().map(|c| Sight::new(c, &job.affine));
        let most = if job.camera.is_some() { job.most } else { base_most(job.most) };
        let (points, taken) = sample(&job.index, most, &job.keep, &job.affine, sight.as_ref());
        let cloud = sender.upload(&points);
        drop(points);
        shared.done.lock().unwrap_or_else(|e| e.into_inner()).push(Done { key: job.key, detail: job.camera.is_some(), cloud, taken, ms: t.elapsed().as_secs_f64() * 1000.0 });
        shared.busy.store(false, Ordering::SeqCst);
    }
}

/// A scan's points on the card: its base, and the detail for a view.
struct Held {
    key: ScanKey,
    base: Option<Cloud>,
    detail: Option<(Cloud, Taken)>,
}

/// What's on the card for the scans asked for, and the loader that puts it there.
pub struct Stage {
    loader: Loader,
    held: Vec<Held>,
    /// The view last looked at for detail, and when.
    looked: Option<(Camera, Instant)>,
    /// The last sample's reading and sending, ms.
    pub load_ms: f64,
}

impl Stage {
    pub fn new(sender: Sender) -> Result<Stage, String> {
        Ok(Stage { loader: Loader::start(sender)?, held: Vec::new(), looked: None, load_ms: 0.0 })
    }

    /// Whether a sample is being read now.
    pub fn loading(&self) -> bool {
        self.loader.busy()
    }

    /// What the loader's read taken in, what's no longer asked for let go,
    /// and the next sample asked of it (a base first, then the detail for
    /// the view where a scan's hasn't it). True when what's on the card
    /// changed (a frame's due).
    pub fn step(&mut self, ask: &Ask) -> Result<bool, String> {
        if let Some(e) = self.loader.shared.failed.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return Err(e);
        }
        let mut changed = false;
        // No longer asked for: off the card.
        let before = self.held.len();
        self.held.retain(|h| ask.scans.iter().any(|s| key_of(s) == h.key));
        changed |= self.held.len() != before;
        for done in std::mem::take(&mut *self.loader.shared.done.lock().unwrap_or_else(|e| e.into_inner())) {
            // Asked for no longer (the scans changed while it was read): dropped.
            if !ask.scans.iter().any(|s| key_of(s) == done.key) {
                continue;
            }
            self.load_ms = done.ms;
            let at = match self.held.iter().position(|h| h.key == done.key) {
                Some(k) => k,
                None => {
                    self.held.push(Held { key: done.key.clone(), base: None, detail: None });
                    self.held.len() - 1
                }
            };
            if done.detail {
                self.held[at].detail = Some((done.cloud, done.taken));
            } else {
                self.held[at].base = Some(done.cloud);
            }
            changed = true;
        }
        if self.loader.busy() {
            return Ok(changed);
        }
        // A scan with no base yet: that first.
        for s in &ask.scans {
            let key = key_of(s);
            if !self.held.iter().any(|h| h.key == key && h.base.is_some()) {
                self.loader.give(Job { key, index: s.index.clone(), affine: s.affine, keep: s.keep.clone(), most: s.most, camera: None });
                return Ok(changed);
            }
        }
        // Then the view's detail, looked at as the view changes (not every frame of a move).
        let Some(cam) = ask.camera.filter(|_| !ask.hide && !ask.test) else {
            return Ok(changed);
        };
        if let Some((seen, when)) = &self.looked {
            if *seen == cam || when.elapsed() < Duration::from_millis(DETAIL_EVERY_MS) {
                return Ok(changed);
            }
        }
        self.looked = Some((cam, Instant::now()));
        for s in &ask.scans {
            let key = key_of(s);
            let Some(h) = self.held.iter().find(|h| h.key == key) else { continue };
            let want = wanted(&s.index, s.most, Some(&Sight::new(&cam, &s.affine)));
            if want.of == 0 {
                continue;
            }
            // The base holds it already (a small scan, or all of it in view at no more than the base's share).
            if h.detail.as_ref().is_some_and(|(_, t)| t.covers(&want)) {
                continue;
            }
            self.loader.give(Job { key, index: s.index.clone(), affine: s.affine, keep: s.keep.clone(), most: s.most, camera: Some(cam) });
            // Looked at again once this one's in (another scan may want its own).
            self.looked = None;
            break;
        }
        Ok(changed)
    }

    /// The points to draw for the ask: each scan's base, and its detail over it.
    pub fn list<'a>(&'a self, ask: &'a Ask) -> Vec<(&'a Cloud, Affine, &'a Look)> {
        let mut out = Vec::new();
        if ask.hide {
            return out;
        }
        for s in &ask.scans {
            let key = key_of(s);
            let Some(h) = self.held.iter().find(|h| h.key == key) else { continue };
            if let Some(base) = &h.base {
                out.push((base, s.affine, &s.look));
                if let Some((detail, _)) = &h.detail {
                    out.push((detail, s.affine, &s.look));
                }
            }
        }
        out
    }

    /// How many of the asked scans are on the card (their base at least), and how many with a view's detail.
    pub fn held(&self, ask: &Ask) -> (u32, u32) {
        let mut on = 0;
        let mut detail = 0;
        for s in &ask.scans {
            let key = key_of(s);
            if let Some(h) = self.held.iter().find(|h| h.key == key && h.base.is_some()) {
                on += 1;
                if h.detail.is_some() {
                    detail += 1;
                }
            }
        }
        (on, detail)
    }
}

/// The most an ask is held, ms (a few refreshes of the slowest screen).
pub const HOLD_MOST_MS: f64 = 100.0;

/// The asks as they came, each shown once it's been held as long as it
/// asks (`Ask::hold_ms`): the newest of those due. A view that's stopped
/// moving is shown as soon as its own hold is up.
#[derive(Default)]
pub struct Pace {
    waiting: std::collections::VecDeque<(Instant, Ask)>,
}

impl Pace {
    fn due_at(came: Instant, ask: &Ask) -> Instant {
        let hold = if ask.hold_ms.is_finite() { ask.hold_ms.clamp(0.0, HOLD_MOST_MS) } else { 0.0 };
        came + Duration::from_secs_f64(hold / 1000.0)
    }

    pub fn push(&mut self, came: Instant, ask: Ask) {
        self.waiting.push_back((came, ask));
    }

    /// The ask to show at `now`: the newest whose hold is up, those before it let go; none when none's due.
    pub fn due(&mut self, now: Instant) -> Option<Ask> {
        let mut out = None;
        while let Some((came, ask)) = self.waiting.front() {
            if Self::due_at(*came, ask) > now {
                break;
            }
            out = self.waiting.pop_front().map(|(_, a)| a);
        }
        out
    }

    /// When the next one's hold is up.
    pub fn next(&self) -> Option<Instant> {
        self.waiting.front().map(|(came, ask)| Self::due_at(*came, ask))
    }
}

struct Shared {
    ask: Mutex<(Ask, u64)>,
    /// Told as a new ask comes (the drawing thread wakes for it at once).
    came: Condvar,
    stats: Mutex<Stats>,
    stop: AtomicBool,
}

/// The engine's window, drawing.
pub struct View {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl View {
    /// Started in the frame whose window handle is `parent`. Only on Windows.
    pub fn start(parent: isize) -> Result<View, String> {
        if !cfg!(windows) {
            return Err("the engine draws in a window of its own only on Windows".into());
        }
        if parent == 0 {
            return Err("no window to draw in".into());
        }
        let shared = Arc::new(Shared {
            // Until the page says what to show: its own background, nothing on it.
            ask: Mutex::new((Ask::default(), 0)),
            came: Condvar::new(),
            stats: Mutex::new(Stats::default()),
            stop: AtomicBool::new(false),
        });
        let theirs = shared.clone();
        let thread = std::thread::Builder::new()
            .name("cadp-view".into())
            .spawn(move || {
                // A panic (the card's driver, a frame the card wouldn't take) ends the drawing and is said, not the engine's process.
                let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(parent, &theirs))).unwrap_or_else(|p| {
                    let said = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                    Err(format!("the engine's drawing stopped: {}", said.unwrap_or_else(|| "it didn't say why".into())))
                });
                let mut stats = theirs.stats.lock().unwrap_or_else(|e| e.into_inner());
                stats.running = false;
                if let Err(e) = ended {
                    stats.error = Some(e);
                }
            })
            .map_err(|e| format!("couldn't start the drawing thread: {e}"))?;
        Ok(View { shared, thread: Some(thread) })
    }

    /// What to show from now on.
    pub fn show(&self, ask: Ask) {
        let mut held = self.shared.ask.lock().unwrap_or_else(|e| e.into_inner());
        held.0 = ask;
        held.1 += 1;
        self.shared.came.notify_one();
    }

    pub fn stats(&self) -> Stats {
        self.shared.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Stopped, its window gone.
    pub fn stop(mut self) {
        self.end();
    }

    fn end(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.came.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.end();
    }
}

#[cfg(not(windows))]
fn run(_parent: isize, _shared: &Shared) -> Result<(), String> {
    Err("the engine draws in a window of its own only on Windows".into())
}

#[cfg(windows)]
fn run(parent: isize, shared: &Shared) -> Result<(), String> {
    use crate::gpu::{Gpu, CLEAR};
    use crate::scene::test_ring;

    let window = win::Child::new(parent)?;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    // The window outlives the surface: both are this thread's, the surface dropped first (declared after).
    let surface = unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: window.display_handle(),
                raw_window_handle: window.window_handle(),
            })
            .map_err(|e| format!("the engine's window can't be drawn on: {e}"))?
    };
    let mut gpu = Gpu::new(&instance, Some(&surface), None)?;
    // What the card refuses (a buffer too big, memory run out) is said, and the drawing ended, not the engine's process.
    let refused: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let refused = refused.clone();
        gpu.device.on_uncaptured_error(Box::new(move |e| {
            let mut held = refused.lock().unwrap_or_else(|p| p.into_inner());
            if held.is_none() {
                *held = Some(e.to_string());
            }
        }));
    }
    let mut stage = Stage::new(gpu.sender())?;
    {
        let mut stats = shared.stats.lock().unwrap();
        stats.running = true;
        stats.adapter = gpu.adapter.clone();
    }
    let configure = |gpu: &Gpu, size: [u32; 2]| {
        surface.configure(
            &gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: gpu.format,
                width: size[0].max(1),
                height: size[1].max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                // One frame waiting to be shown at most: a view is on screen the refresh after it's drawn, each time (the page says how long to hold it).
                desired_maximum_frame_latency: 1,
            },
        );
    };
    let mut size = [0u32; 2];
    let mut seen = u64::MAX;
    let mut ask = Ask::default();
    let mut pace = Pace::default();
    let mut ring: Option<Cloud> = None;
    let ring_look = Look { px: 3.0, ..Look::own() };
    let started = Instant::now();
    let mut second = Instant::now();
    let mut in_second = 0u32;
    // When the last frame was shown, and the longest of the run it's in.
    let mut last_frame = Instant::now();
    let mut worst = 0.0f64;
    let mut dirty = true;
    while !shared.stop.load(Ordering::SeqCst) {
        if let Some(e) = refused.lock().unwrap_or_else(|p| p.into_inner()).take() {
            return Err(format!("the graphics card refused: {e}"));
        }
        window.pump();
        // The frame's inside, as it is now: the window kept filling it, in front of the frame's own page.
        let now = window.fit();
        if now != size && now[0] > 0 && now[1] > 0 {
            size = now;
            configure(&gpu, size);
            dirty = true;
        }
        {
            let held = shared.ask.lock().unwrap();
            if held.1 != seen {
                seen = held.1;
                pace.push(Instant::now(), held.0.clone());
            }
        }
        // Shown once it's been held as long as it asks (the page's own picture lands that much later).
        if let Some(due) = pace.due(Instant::now()) {
            ask = due;
            dirty = true;
        }
        if size[0] == 0 || size[1] == 0 {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        // The scans' points, read by the loader while frames go on: what's come is on the card from this frame.
        if stage.step(&ask)? {
            dirty = true;
            shared.stats.lock().unwrap().load_ms = stage.load_ms;
        }
        if ask.test && ring.is_none() {
            ring = Some(gpu.upload(&test_ring(6000, [0.0, 0.0, 0.0])));
        }
        // The check's ring turns, so it's drawn every frame; a view only when something's changed.
        if !dirty && !ask.test {
            // Till the next ask's hold is up, a sample's read, or an ask comes (woken for it at once).
            let nap = Duration::from_millis(if stage.loading() { 2 } else { 4 });
            let wait = pace.next().map(|at| at.saturating_duration_since(Instant::now())).map_or(nap, |d| d.min(nap));
            let held = shared.ask.lock().unwrap();
            if held.1 == seen && !wait.is_zero() {
                let _ = shared.came.wait_timeout(held, wait);
            }
            continue;
        }
        // A rest since the last frame: a new run's count begins (frames a second is of frames one after another).
        if last_frame.elapsed() > Duration::from_millis(250) {
            second = Instant::now();
            in_second = 0;
            worst = 0.0;
        }
        let t = Instant::now();
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                configure(&gpu, size);
                continue;
            }
            Err(wgpu::SurfaceError::Timeout) => continue,
            Err(e) => return Err(format!("the engine's window stopped taking frames: {e}")),
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let clear = match ask.clear {
            Some([r, g, b]) => [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), 1.0],
            None => CLEAR,
        };
        let mut points = 0u64;
        let mut drew = (0u32, 0u32);
        if let (true, Some(ring)) = (ask.test, ring.as_ref()) {
            // Turning about its middle, a turn every 6 s, the ring 60% of the window's shorter side.
            let a = started.elapsed().as_secs_f64() * std::f64::consts::TAU / 6.0;
            let cam = Camera {
                target: [0.0, 0.0, 0.0],
                right: [a.cos(), a.sin(), 0.0],
                up: [-a.sin(), a.cos(), 0.0],
                to_eye: [0.0, 0.0, 1.0],
                units_per_pixel: 100.0 / (0.6 * size[0].min(size[1]) as f64),
                rect: [0.0, 0.0, size[0] as f64, size[1] as f64],
            };
            points = ring.count as u64;
            gpu.draw(&target, size, Some(&cam), &[(ring, crate::camera::IDENTITY, &ring_look)], &Edl::default(), CLEAR);
        } else {
            let list = stage.list(&ask);
            points = list.iter().map(|(c, _, _)| c.count as u64).sum::<u64>().max(points);
            if ask.camera.is_some() && !ask.hide {
                drew = stage.held(&ask);
            }
            gpu.draw(&target, size, ask.camera.as_ref(), &list, &ask.edl, clear);
        }
        frame.present();
        dirty = false;
        in_second += 1;
        last_frame = Instant::now();
        let took = t.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(took);
        let mut stats = shared.stats.lock().unwrap();
        stats.frames += 1;
        stats.frame_ms = took;
        stats.points = points;
        stats.scans = drew.0;
        stats.detail = drew.1;
        stats.drawn = ask.serial;
        stats.size = size;
        let over = second.elapsed().as_secs_f64();
        if over >= 0.5 {
            stats.fps = in_second as f64 / over;
            stats.worst_ms = worst;
            in_second = 0;
            worst = 0.0;
            second = Instant::now();
        }
    }
    Ok(())
}

#[cfg(windows)]
mod win {
    //! The engine's window: a child of the app's frame, filling its inside.

    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    use std::num::NonZeroIsize;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, IsWindow, PeekMessageW, RegisterClassW,
        SetWindowPos, TranslateMessage, MSG, PM_REMOVE, SWP_NOACTIVATE, WM_ERASEBKGND, WM_NCHITTEST, WNDCLASSW, WS_CHILD,
        WS_CLIPSIBLINGS, WS_DISABLED, WS_EX_NOACTIVATE, WS_EX_NOPARENTNOTIFY, WS_VISIBLE,
    };

    /// No bitmap of Windows' own behind it: the card's frames are all it shows.
    const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
    /// Clicks fall through it.
    const HTTRANSPARENT: LRESULT = -1;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_NCHITTEST => HTTRANSPARENT,
            WM_ERASEBKGND => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    pub struct Child {
        hwnd: HWND,
        parent: HWND,
        instance: isize,
        size: std::cell::Cell<[u32; 2]>,
    }

    impl Child {
        pub fn new(parent: isize) -> Result<Child, String> {
            unsafe {
                let parent = parent as HWND;
                if IsWindow(parent) == 0 {
                    return Err("the app's frame isn't a window any more".into());
                }
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide("CadpEngineView");
                let mut wc: WNDCLASSW = std::mem::zeroed();
                wc.lpfnWndProc = Some(proc);
                wc.hInstance = instance;
                wc.lpszClassName = class.as_ptr();
                // Registered once a process: a second try says it's there, which is fine.
                RegisterClassW(&wc);
                let mut rect: RECT = std::mem::zeroed();
                GetClientRect(parent, &mut rect);
                let hwnd = CreateWindowExW(
                    // The frame is another process's: it isn't told of this window's making or its clicks (it takes none).
                    WS_EX_NOACTIVATE | WS_EX_NOPARENTNOTIFY | WS_EX_NOREDIRECTIONBITMAP,
                    class.as_ptr(),
                    wide("CAD Protocol scans").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS,
                    0,
                    0,
                    (rect.right - rect.left).max(1),
                    (rect.bottom - rect.top).max(1),
                    parent,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if (hwnd as isize) == 0 {
                    return Err(format!("the engine's window couldn't be made ({})", std::io::Error::last_os_error()));
                }
                Ok(Child { hwnd, parent, instance: instance as isize, size: std::cell::Cell::new([0, 0]) })
            }
        }

        pub fn window_handle(&self) -> RawWindowHandle {
            let mut h = Win32WindowHandle::new(NonZeroIsize::new(self.hwnd as isize).expect("a window"));
            h.hinstance = NonZeroIsize::new(self.instance);
            RawWindowHandle::Win32(h)
        }

        pub fn display_handle(&self) -> RawDisplayHandle {
            RawDisplayHandle::Windows(WindowsDisplayHandle::new())
        }

        /// Its messages answered (there are few: it takes no clicks or keys).
        pub fn pump(&self) {
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        /// Kept filling the frame's inside, in front of the frame's own page; its size, pixels (0 × 0 once the frame's gone).
        pub fn fit(&self) -> [u32; 2] {
            unsafe {
                if IsWindow(self.parent) == 0 || IsWindow(self.hwnd) == 0 {
                    return [0, 0];
                }
                let mut rect: RECT = std::mem::zeroed();
                if GetClientRect(self.parent, &mut rect) == 0 {
                    return [0, 0];
                }
                let now = [(rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32];
                if now != self.size.get() && now[0] > 0 && now[1] > 0 {
                    // HWND_TOP: over whatever else the frame holds (its page's own window).
                    SetWindowPos(self.hwnd, std::ptr::null_mut(), 0, 0, now[0] as i32, now[1] as i32, SWP_NOACTIVATE);
                    self.size.set(now);
                }
                now
            }
        }
    }

    impl Drop for Child {
        fn drop(&mut self) {
            unsafe {
                if IsWindow(self.hwnd) != 0 {
                    DestroyWindow(self.hwnd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_is_shown_once_its_been_held_and_the_newest_due_wins() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let ask = |serial: u64, hold_ms: f64| Ask { serial, hold_ms, ..Ask::default() };
        let mut pace = Pace::default();
        // Not held: shown at once.
        pace.push(ms(0), ask(1, 0.0));
        assert_eq!(pace.due(ms(0)).map(|a| a.serial), Some(1));
        assert!(pace.due(ms(1)).is_none());
        // Held a refresh (16.7 ms): as each comes, the one before it is what's due.
        pace.push(ms(100), ask(2, 16.7));
        assert!(pace.due(ms(100)).is_none());
        assert_eq!(pace.next(), Some(ms(100) + Duration::from_secs_f64(0.0167)));
        pace.push(ms(117), ask(3, 16.7));
        assert_eq!(pace.due(ms(117)).map(|a| a.serial), Some(2));
        pace.push(ms(134), ask(4, 16.7));
        assert_eq!(pace.due(ms(134)).map(|a| a.serial), Some(3));
        // The view's stopped: its last ask shows once its own hold's up.
        assert!(pace.due(ms(140)).is_none());
        assert_eq!(pace.due(ms(151)).map(|a| a.serial), Some(4));
        assert!(pace.next().is_none());
        // Several due together (a slow frame): the newest of them, the rest let go.
        pace.push(ms(200), ask(5, 10.0));
        pace.push(ms(205), ask(6, 10.0));
        pace.push(ms(230), ask(7, 10.0));
        assert_eq!(pace.due(ms(220)).map(|a| a.serial), Some(6));
        assert_eq!(pace.due(ms(240)).map(|a| a.serial), Some(7));
        // A hold past the most, or no number: the most, or none.
        pace.push(ms(300), ask(8, 5000.0));
        assert!(pace.due(ms(399)).is_none());
        assert_eq!(pace.due(ms(400)).map(|a| a.serial), Some(8));
        pace.push(ms(500), ask(9, f64::NAN));
        assert_eq!(pace.due(ms(500)).map(|a| a.serial), Some(9));
    }

    #[test]
    fn a_scans_base_is_a_share_of_its_budget() {
        assert_eq!(base_most(24_000_000), 4_000_000);
        // A small budget: a million, or all of it.
        assert_eq!(base_most(3_000_000), 1_000_000);
        assert_eq!(base_most(250_000), 250_000);
        assert_eq!(base_most(0), 1);
    }
}
