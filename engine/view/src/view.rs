//! The engine's own window, and the thread that draws in it.
//!
//! The window is a child of the app's frame (its handle given by the app):
//! it fills the frame's inside, takes no clicks or keys (the app's page is
//! over it and takes them all), and is drawn by a thread of its own, so a
//! frame never waits on the page and the page never waits on a frame. The
//! page says what to show (`Ask`: its view, the scans and where each is in
//! the drawing); the thread loads what's new, draws when something's
//! changed, and keeps count (`Stats`).

use crate::camera::{Affine, Camera};
use cadp_engine_core::Index;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A scan to show: its index (by the engine's handle), where its file's
/// coordinates are in the drawing, and about how many points at most.
#[derive(Clone)]
pub struct ScanAsk {
    pub handle: u32,
    pub index: Arc<Index>,
    pub affine: Affine,
    pub most: usize,
}

/// What to show.
#[derive(Clone, Default)]
pub struct Ask {
    /// The page's view; with none, nothing but the background (or the check's ring, turning).
    pub camera: Option<Camera>,
    pub scans: Vec<ScanAsk>,
    /// A point's size, pixels.
    pub px: f64,
    /// The check's own ring of points, with no scan.
    pub test: bool,
    /// The background (red, green, blue, 0 to 1); the engine's own with none.
    pub clear: Option<[f64; 3]>,
    /// The scans kept on the card and not drawn (the page is drawing them itself for now).
    pub hide: bool,
    /// The page's own number for this ask: said back in `Stats::drawn` once a frame has shown it.
    pub serial: u64,
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
    /// The last scan's sampling and sending, ms.
    pub load_ms: f64,
    pub size: [u32; 2],
    pub adapter: String,
    pub error: Option<String>,
    /// How many of the asked scans the last frame drew.
    pub scans: u32,
    /// The serial of the ask the last frame showed.
    pub drawn: u64,
}

struct Shared {
    ask: Mutex<(Ask, u64)>,
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
            ask: Mutex::new((Ask { px: 2.0, ..Ask::default() }, 0)),
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
    use crate::gpu::{Cloud, Gpu, CLEAR};
    use crate::scene::{sample, test_ring};
    use std::time::{Duration, Instant};

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
                desired_maximum_frame_latency: 2,
            },
        );
    };
    let mut size = [0u32; 2];
    let mut seen = u64::MAX;
    let mut ask = Ask::default();
    // What's on the card: each scan's points, by its handle and how many were asked for.
    let mut clouds: Vec<(u32, usize, Cloud)> = Vec::new();
    let mut ring: Option<Cloud> = None;
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
                ask = held.0.clone();
                dirty = true;
            }
        }
        if size[0] == 0 || size[1] == 0 {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        // Scans asked for and not on the card yet (or asked for with more points): sampled and sent.
        clouds.retain(|(handle, _, _)| ask.scans.iter().any(|s| s.handle == *handle));
        for s in &ask.scans {
            let have = clouds.iter().position(|(h, _, _)| *h == s.handle);
            if let Some(k) = have {
                if clouds[k].1 == s.most {
                    continue;
                }
                clouds.remove(k);
            }
            let t = Instant::now();
            let points = sample(&s.index, s.most);
            let cloud = gpu.upload(&points);
            clouds.push((s.handle, s.most, cloud));
            shared.stats.lock().unwrap().load_ms = t.elapsed().as_secs_f64() * 1000.0;
            dirty = true;
        }
        if ask.test && ring.is_none() {
            ring = Some(gpu.upload(&test_ring(6000, [0.0, 0.0, 0.0])));
        }
        // The check's ring turns, so it's drawn every frame; a view only when something's changed.
        if !dirty && !ask.test {
            std::thread::sleep(Duration::from_millis(4));
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
        let px = if ask.px > 0.0 { ask.px } else { 2.0 };
        let clear = match ask.clear {
            Some([r, g, b]) => [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), 1.0],
            None => CLEAR,
        };
        let mut points = 0u64;
        let mut drew = 0u32;
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
            gpu.draw(&target, size, Some(&cam), &[(ring, crate::camera::IDENTITY)], px.max(3.0), CLEAR);
        } else {
            let list: Vec<(&Cloud, Affine)> = if ask.hide {
                Vec::new()
            } else {
                ask.scans
                    .iter()
                    .filter_map(|s| clouds.iter().find(|(h, _, _)| *h == s.handle).map(|(_, _, c)| (c, s.affine)))
                    .collect()
            };
            points = list.iter().map(|(c, _)| c.count as u64).sum::<u64>().max(points);
            drew = if ask.camera.is_some() { list.len() as u32 } else { 0 };
            gpu.draw(&target, size, ask.camera.as_ref(), &list, px, clear);
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
        stats.scans = drew;
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
