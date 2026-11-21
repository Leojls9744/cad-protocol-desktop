//! What's on the card for a view, end to end without a window: a scan's
//! base, then the detail for the view, read by the loader while frames go
//! on, drawn into a picture.

use cadp_engine_core::{build_or_open, Progress};
use cadp_engine_view::{Ask, Camera, Clip, Edl, Gpu, Keep, Look, ScanAsk, Shading, Stage, CLEAR, IDENTITY};
use las::{point::Classification, Builder, Point, Writer};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cadp-view-stage-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A floor 200 × 150 units, a point every 0.25 (480,000), far from 0, class 2, red in its west half and blue in its east.
fn floor(path: &std::path::Path) -> usize {
    let mut b = Builder::from((1, 2));
    b.point_format = las::point::Format::new(3).unwrap();
    b.transforms = las::Vector {
        x: las::Transform { scale: 0.001, offset: 6_000_000.0 },
        y: las::Transform { scale: 0.001, offset: 2_000_000.0 },
        z: las::Transform { scale: 0.001, offset: 0.0 },
    };
    let mut w = Writer::from_path(path, b.into_header().unwrap()).unwrap();
    let mut n = 0;
    for i in 0..800 {
        for j in 0..600 {
            let p = Point {
                x: 6_000_000.0 + i as f64 * 0.25,
                y: 2_000_000.0 + j as f64 * 0.25,
                z: 100.0,
                intensity: 9,
                classification: Classification::new(2).unwrap(),
                gps_time: Some(0.0),
                color: Some(if i < 400 { las::Color::new(220, 20, 20) } else { las::Color::new(20, 20, 220) }),
                ..Default::default()
            };
            w.write_point(p).unwrap();
            n += 1;
        }
    }
    w.close().unwrap();
    n
}

fn card() -> Option<Gpu> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    match Gpu::new(&instance, None, Some(wgpu::TextureFormat::Rgba8Unorm)) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("no card to draw on here: {e}");
            None
        }
    }
}

/// Stepped until the loader's idle and nothing more comes (or 20 s).
fn settle(stage: &mut Stage, ask: &Ask) {
    let until = Instant::now() + Duration::from_secs(20);
    let mut quiet = 0;
    while Instant::now() < until && quiet < 30 {
        let changed = stage.step(ask).expect("the stage");
        quiet = if changed || stage.loading() { 0 } else { quiet + 1 };
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pixel(picture: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 4) as usize;
    [picture[at], picture[at + 1], picture[at + 2]]
}

#[test]
fn a_scans_base_comes_first_then_the_detail_for_the_view() {
    let Some(mut gpu) = card() else { return };
    let dir = scratch("detail");
    let src = dir.join("floor.las");
    let n = floor(&src);
    let ix = Arc::new(build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap());
    let mut stage = Stage::new(gpu.sender()).expect("a stage");
    // A budget of a twelfth of its points: the base a sixth of that (the least it can be is all of it here, so: all of the budget).
    let most = n / 12;
    let scan = ScanAsk { handle: 1, index: ix.clone(), affine: IDENTITY, most, keep: Keep::all().keyed(), look: Look { px: 1.0, ..Look::own() } };
    // The whole floor in a drawing area 400 × 300 px: half a unit a pixel.
    let whole = Camera {
        target: [6_000_100.0, 2_000_075.0, 100.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        to_eye: [0.0, 0.0, 1.0],
        units_per_pixel: 0.5,
        rect: [0.0, 0.0, 400.0, 300.0],
    };
    let mut ask = Ask { camera: Some(whole), scans: vec![scan.clone()], ..Ask::default() };
    // Nothing yet: asked for, a frame draws the background.
    assert_eq!(stage.held(&ask), (0, 0));
    assert!(stage.list(&ask).is_empty());
    settle(&mut stage, &ask);
    // Its base, and the view's detail (the whole floor at the budget: no more than the base has, but its own sample).
    let (on, _) = stage.held(&ask);
    assert_eq!(on, 1);
    let drawn: u64 = stage.list(&ask).iter().map(|(c, _, _)| c.count as u64).sum();
    assert!(drawn as usize >= most, "{drawn} drawn of a budget of {most}");
    // Drawn 5 px a point (a twelfth of its points is one to three pixels here).
    let big = Look { px: 5.0, ..Look::own() };
    let list: Vec<_> = stage.list(&ask).iter().map(|(c, a, _)| (*c, *a, &big)).collect();
    let picture = gpu.picture([400, 300], ask.camera.as_ref(), &list, &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    drop(list);
    // Red in the west, blue in the east.
    assert_eq!(pixel(&picture, 400, 100, 150), [220, 20, 20]);
    assert_eq!(pixel(&picture, 400, 300, 150), [20, 20, 220]);
    // Zoomed in on 10 × 7.5 units where the colours meet: 0.025 units a pixel, the points 10 px apart.
    let close = Camera { target: [6_000_100.0, 2_000_075.0, 100.0], units_per_pixel: 0.025, ..whole };
    ask.camera = Some(close);
    ask.serial = 2;
    // Before its detail's read: the base's few points there.
    let before: u64 = stage.list(&ask).iter().map(|(c, _, _)| c.count as u64).sum();
    settle(&mut stage, &ask);
    assert_eq!(stage.held(&ask), (1, 1));
    let list = stage.list(&ask);
    assert_eq!(list.len(), 2);
    // The view's detail first, then the base: where both have a point, it's the detail's that shows.
    let base = list[1].0.count as usize;
    assert!(base >= most && base <= most + ix.cells * 2 && list[0].0.count as usize != base, "the detail's {} then the base's {base} (a budget of {most})", list[0].0.count);
    // Every point the view sees is on the card now: 40 × 30 in the drawing area itself, each 10 px from the next.
    let look = Look { px: 3.0, ..Look::own() };
    let list: Vec<_> = list.iter().map(|(c, a, _)| (*c, *a, &look)).collect();
    let picture = gpu.picture([400, 300], ask.camera.as_ref(), &list, &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    let mut dots = 0;
    for gy in 0..30u32 {
        for gx in 0..40u32 {
            // The target is a point, at the drawing area's middle (200, 150); the rest every 10 px from it each way.
            let p = pixel(&picture, 400, gx * 10, gy * 10);
            if p == [220, 20, 20] || p == [20, 20, 220] {
                dots += 1;
            }
        }
    }
    assert!(dots >= 40 * 30 - 80, "{dots} of the view's 1200 points drawn");
    let after: u64 = list.iter().map(|(c, _, _)| c.count as u64).sum();
    assert!(after != before || before as usize == n, "{before} then {after}");
    // The same view again: nothing read again.
    assert!(!stage.step(&ask).unwrap());
    assert!(!stage.loading());
    // A second view of the same scan (the split view's other side), looking at its north-east corner: the scan's base
    // is shared, and that view gets detail of its own, the first view's left as it was.
    let corner = Camera { target: [6_000_195.0, 2_000_145.0, 100.0], units_per_pixel: 0.025, rect: [400.0, 0.0, 400.0, 300.0], ..whole };
    let other = Ask { camera: Some(corner), scans: vec![scan.clone()], serial: 9, ..Ask::default() };
    let both = [ask.clone(), other.clone()];
    assert_eq!(stage.held_in(1, &other), (1, 0));
    let first: Vec<u32> = stage.list_in(0, &ask).iter().map(|(c, _, _)| c.count).collect();
    for _ in 0..600 {
        let changed = stage.step_all(&both).expect("a step");
        if !changed && !stage.loading() && stage.held_in(1, &other) == (1, 1) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(stage.held_in(1, &other), (1, 1));
    assert_eq!(stage.held_in(0, &ask), (1, 1));
    assert_eq!(stage.list_in(0, &ask).iter().map(|(c, _, _)| c.count).collect::<Vec<u32>>(), first);
    let second = stage.list_in(1, &other);
    assert_eq!(second.len(), 2);
    assert_eq!(second[1].0.count, first[1], "the same base in both views");
    assert_ne!(second[0].0.count, first[0], "its own detail");
    // The second view gone: its detail's let go, the first's kept.
    stage.step_all(std::slice::from_ref(&ask)).expect("a step");
    assert_eq!(stage.held_in(1, &other), (1, 0));
    assert_eq!(stage.held_in(0, &ask), (1, 1));
    // The scan asked for another way (here with its shading worked out: the brush's presses, its classes or its
    // isolated points are others), the view where it was. Asked for plainly, the reading it had goes at once and
    // the new one's base shows as soon as it's read.
    let again = ScanAsk { keep: Keep { shade: true, ..Keep::all() }.keyed(), ..scan.clone() };
    let plain = Ask { camera: ask.camera, scans: vec![again.clone()], serial: 20, ..Ask::default() };
    stage.step(&plain).expect("a step");
    assert_eq!((stage.held(&plain), stage.stale_in(0, &plain)), ((0, 0), 0));
    assert!(stage.list(&plain).is_empty());
    // Back as it was, whole again.
    settle(&mut stage, &ask);
    assert_eq!(stage.held(&ask), (1, 1));
    let was: Vec<u32> = stage.list(&ask).iter().map(|(c, _, _)| c.count).collect();
    // Asked for with its older reading kept meanwhile (`stale`): that's what's drawn, every point of it, until the new
    // one is whole (its base and this view's detail), though the view never moves.
    let kept = Ask { stale: true, serial: 21, ..plain.clone() };
    stage.step(&kept).expect("a step");
    assert_eq!((stage.held(&kept), stage.stale_in(0, &kept)), ((0, 0), 1));
    assert_eq!(stage.list(&kept).iter().map(|(c, _, _)| c.count).collect::<Vec<u32>>(), was);
    let mut seen_stale = 0;
    for _ in 0..4000 {
        stage.step(&kept).expect("a step");
        let (on, detail) = stage.held(&kept);
        let stale = stage.stale_in(0, &kept);
        // Never neither, and never the new one's base alone: the old whole, then the new whole.
        assert_eq!(on + stale, 1);
        if on == 1 {
            assert_eq!(detail, 1, "the new reading shows only once its detail is in");
            break;
        }
        seen_stale += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(seen_stale > 0);
    assert_eq!((stage.held(&kept), stage.stale_in(0, &kept)), ((1, 1), 0));
    // The older reading's off the card now: asked for again, it's read again.
    stage.step(&kept).expect("a step");
    assert_eq!(stage.held(&ask), (0, 0));
    // A scan it never had another reading of: nothing to keep, its base as soon as it's read.
    let fresh = Ask { camera: ask.camera, scans: vec![ScanAsk { handle: 7, ..again.clone() }], stale: true, serial: 22, ..Ask::default() };
    stage.step(&fresh).expect("a step");
    assert_eq!((stage.held(&fresh), stage.stale_in(0, &fresh)), ((0, 0), 0));
    settle(&mut stage, &fresh);
    assert_eq!((stage.held(&fresh), stage.stale_in(0, &fresh)), ((1, 1), 0));
    // As it was for what follows.
    settle(&mut stage, &ask);
    assert_eq!(stage.held(&ask), (1, 1));
    // Hidden (the page draws them itself): kept on the card, none drawn.
    ask.hide = true;
    assert!(stage.list(&ask).is_empty());
    ask.hide = false;
    // Another scan asked for instead: this one's off the card.
    let other = ScanAsk { handle: 2, ..scan };
    let ask = Ask { camera: Some(whole), scans: vec![other], ..Ask::default() };
    stage.step(&ask).unwrap();
    assert_eq!(stage.held(&ask), (0, 0));
    settle(&mut stage, &ask);
    assert_eq!(stage.held(&ask).0, 1);
    let _ = std::fs::remove_dir_all(&dir);
}
