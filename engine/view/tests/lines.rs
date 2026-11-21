//! The page's linework drawn on the card over the scans (1.41): where the
//! page's own card would put it, as wide, covering each pixel as much.

use cadp_engine_view::{covers, Camera, Clip, Edl, Gpu, LineItem, LineList, LineWorld, Lines, Look, PointVertex, Points, Shading, Strips, ViewDraw, CLEAR, IDENTITY, NO_STROKES};
use std::sync::Arc;

/// One test draws at a time (as tests/picture.rs: the build computer's stand-in for a card).
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Card {
    gpu: Gpu,
    _turn: std::sync::MutexGuard<'static, ()>,
}

fn card() -> Option<Card> {
    let turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    match Gpu::new(&instance, None, Some(wgpu::TextureFormat::Rgba8Unorm)) {
        Ok(gpu) => {
            eprintln!("drawing on {}", gpu.adapter);
            Some(Card { gpu, _turn: turn })
        }
        Err(e) => {
            eprintln!("no card to draw on here: {e}");
            None
        }
    }
}

fn pixel(picture: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 4) as usize;
    [picture[at], picture[at + 1], picture[at + 2]]
}

fn near(a: [u8; 3], b: [u8; 3], slack: i32) -> bool {
    (0..3).all(|k| (a[k] as i32 - b[k] as i32).abs() <= slack)
}

fn clear_bytes() -> [u8; 3] {
    [(CLEAR[0] * 255.0).round() as u8, (CLEAR[1] * 255.0).round() as u8, (CLEAR[2] * 255.0).round() as u8]
}

/// A colour over what's under it, covering `cover` of the pixel.
fn over(line: [f32; 3], under: [u8; 3], cover: f64) -> [u8; 3] {
    let mix = |k: usize| (line[k] as f64 * 255.0 * cover + under[k] as f64 * (1.0 - cover)).round() as u8;
    [mix(0), mix(1), mix(2)]
}

/// A segment as the page packs it: its two ends, whether each end is soft, no span.
fn strip(a: [f32; 3], b: [f32; 3], soft: [f32; 2]) -> [f32; 9] {
    [a[0], a[1], a[2], b[0], b[1], b[2], soft[0], soft[1], 0.0]
}

fn batch(id: u64, strips: &[[f32; 9]], rgba: [f32; 4], world: bool) -> LineItem {
    LineItem::solid(Arc::new(Strips { id, floats: strips.iter().flatten().copied().collect() }), rgba, world)
}

fn list(key: &str, items: Vec<LineItem>) -> Arc<LineList> {
    Arc::new(LineList { key: key.into(), items })
}

/// The page's full line on a screen of one pixel a CSS pixel: 1.25 wide.
const HALF: f64 = 0.625;

fn plan(width: f64, height: f64) -> Camera {
    Camera { target: [0.0, 0.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], to_eye: [0.0, 0.0, 1.0], units_per_pixel: 1.0, rect: [0.0, 0.0, width, height] }
}

fn shot(gpu: &mut Gpu, size: [u32; 2], cam: &Camera, clouds: &[(&cadp_engine_view::Cloud, cadp_engine_view::Affine, &Look)], lines: Option<&Lines>) -> Vec<u8> {
    let (edl, shading, clip) = (Edl::default(), Shading::default(), Clip::default());
    gpu.picture_views(size, &[ViewDraw { slot: 0, camera: cam, clouds, edl: &edl, shading: &shading, clip: &clip, clear: CLEAR, brush: None, strokes: &NO_STROKES, lines }], CLEAR).expect("a picture")
}

#[test]
fn a_line_is_drawn_where_the_page_puts_it_and_covers_each_pixel_as_the_pages_does() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(200.0, 120.0);
    let red = [1.0f32, 0.0, 0.0];
    // Across the plane from 10 to 90 at its origin's height; the origin 50.5 px down the view, so along the middles of row 50.
    let l = Lines { list: list("a", vec![batch(1, &[strip([10.0, 0.0, 0.0], [90.0, 0.0, 0.0], [1.0, 1.0])], [1.0, 0.0, 0.0, 1.0], false)]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    let clear = clear_bytes();
    assert_eq!(at(50, 50), [255, 0, 0], "its middle, whole");
    // A row either side: an eighth of the pixel (1.25 wide: 0.125 past the pixel's edge).
    assert!(near(at(50, 49), over(red, clear, covers(HALF, 1.0, 0.0)), 2), "{:?}", at(50, 49));
    assert!(near(at(50, 51), over(red, clear, covers(HALF, 1.0, 0.0)), 2), "{:?}", at(50, 51));
    assert_eq!(at(50, 48), clear);
    assert_eq!(at(50, 52), clear);
    // Its ends soft and round: half a pixel past each, then nothing.
    assert!(near(at(90, 50), over(red, clear, covers(HALF, 0.0, 0.5)), 2), "{:?}", at(90, 50));
    assert_eq!(at(92, 50), clear);
    assert!(near(at(9, 50), over(red, clear, covers(HALF, 0.0, 0.5)), 2), "{:?}", at(9, 50));
    assert_eq!(at(7, 50), clear);
    assert_eq!(at(10, 50), [255, 0, 0]);
    assert_eq!(at(89, 50), [255, 0, 0]);
    // Scaled and on a screen of 1.5 pixels a CSS pixel: x = (e + r·scale)·ratio, y = (f − u·scale)·ratio; as wide in the screen's pixels.
    let up = Lines { list: list("b", vec![batch(2, &[strip([20.0, 10.0, 0.0], [20.0, 30.0, 0.0], [1.0, 1.0])], [0.0, 1.0, 0.0, 1.0], false)]), scale: 2.0, e: 3.0, f: 70.0, world: None, half: HALF, ratio: 1.5, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&up));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    // (3 + 40) × 1.5 = 64.5 across; from (70 − 20) × 1.5 = 75 up to (70 − 60) × 1.5 = 15.
    assert_eq!(at(64, 40), [0, 255, 0]);
    assert!(near(at(63, 40), over([0.0, 1.0, 0.0], clear, covers(HALF, 1.0, 0.0)), 2));
    assert!(near(at(65, 40), over([0.0, 1.0, 0.0], clear, covers(HALF, 1.0, 0.0)), 2));
    assert_eq!(at(62, 40), clear);
    assert_eq!(at(64, 74), [0, 255, 0]);
    assert_eq!(at(64, 15), [0, 255, 0]);
    assert_eq!(at(64, 78), clear);
    assert_eq!(at(64, 11), clear);
    // No lines: none drawn.
    let p = shot(gpu, [200, 120], &cam, &[], None);
    assert_eq!(pixel(&p, 200, 64, 40), clear);
}

#[test]
fn a_run_s_joints_are_drawn_once_and_batches_in_their_turn() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(200.0, 120.0);
    let clear = clear_bytes();
    // Two segments carrying on from one another (the joint cut), half solid: every pixel along them covered once, the joint's too.
    let green = [0.0f32, 1.0, 0.0];
    let run = batch(3, &[strip([10.0, 0.0, 0.0], [50.0, 0.0, 0.0], [1.0, 0.0]), strip([50.0, 0.0, 0.0], [90.0, 0.0, 0.0], [0.0, 1.0])], [0.0, 1.0, 0.0, 0.5], false);
    // Then a line down across it, in another batch: drawn after, so over it.
    let down = batch(4, &[strip([70.5, 20.0, 0.0], [70.5, -20.0, 0.0], [1.0, 1.0])], [0.0, 0.0, 1.0, 1.0], false);
    let l = Lines { list: list("a", vec![run.clone(), down.clone()]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    let once = over(green, clear, 0.5);
    for x in [30, 48, 49, 50, 51, 60] {
        assert!(near(at(x, 50), once, 2), "at {x}: {:?}, once is {once:?}", at(x, 50));
    }
    assert_eq!(at(70, 50), [0, 0, 255], "the later batch over the earlier");
    assert_eq!(at(70, 40), [0, 0, 255]);
    // The other way round: the run over the line.
    let l = Lines { list: list("b", vec![down, run]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    assert!(near(pixel(&p, 200, 70, 50), over(green, [0, 0, 255], 0.5), 2));
    // The same strips in another list, another colour: the list's.
    let again = Lines { list: list("c", vec![LineItem { rgba: [1.0, 1.0, 0.0, 1.0], ..l.list.items[1].clone() }]), ..l.clone() };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&again));
    assert_eq!(pixel(&p, 200, 30, 50), [255, 255, 0]);
    assert_eq!(pixel(&p, 200, 70, 40), clear);
}

#[test]
fn lines_in_the_drawing_s_own_places_follow_the_view_s_right_and_up() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(200.0, 120.0);
    let clear = clear_bytes();
    // From 10 to 60 north at 20 up, 5 east (and 300 east: across is north in this view, so east doesn't show).
    let b = batch(5, &[strip([5.0, 10.0, 20.0], [300.0, 60.0, 20.0], [1.0, 1.0])], [1.0, 0.0, 1.0, 1.0], true);
    // Looking east: north is right, up is up. The world's origin 100 px across the view and 60.5 down.
    let front = LineWorld { right: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], e: 100.0, f: 60.5 };
    let l = Lines { list: list("a", vec![b.clone()]), scale: 1.0, e: -999.0, f: -999.0, world: Some(front), half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    assert_eq!(at(130, 40), [255, 0, 255], "110 to 160 across, 40.5 down");
    assert_eq!(at(111, 40), [255, 0, 255]);
    assert_eq!(at(159, 40), [255, 0, 255]);
    assert_eq!(at(105, 40), clear);
    assert_eq!(at(165, 40), clear);
    assert_eq!(at(130, 43), clear);
    // Turned a quarter about the line of sight (right is up, up is south): the same line runs up the screen.
    let turned = LineWorld { right: [0.0, 0.0, 1.0], up: [0.0, -1.0, 0.0], e: 100.5, f: 20.0 };
    let l = Lines { world: Some(turned), ..l };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    // Across: 100.5 + 20; down: 20 + 10 to 20 + 60.
    assert_eq!(at(120, 50), [255, 0, 255]);
    assert_eq!(at(120, 31), [255, 0, 255]);
    assert_eq!(at(120, 79), [255, 0, 255]);
    assert_eq!(at(120, 25), clear);
    assert_eq!(at(120, 85), clear);
    // Without the view's right and up they aren't drawn (the plane's batches would be).
    let l = Lines { world: None, ..l };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    assert!((0..120).all(|y| (0..200).all(|x| pixel(&p, 200, x, y) == clear)));
}

#[test]
fn lines_are_over_the_scans_in_their_view_s_part_of_the_window_and_strips_go_when_the_page_lets_them() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let clear = clear_bytes();
    // A grey floor, in a drawing area that starts 100 px into a window 300 wide.
    let mut data = Vec::new();
    for i in -80..80 {
        for j in -80..80 {
            data.push(PointVertex { pos: [i as f32 + 0.5, j as f32 + 0.5, 0.0], rgb: [200, 200, 200], class: 2, intensity: 0, setup: 0, shade: cadp_engine_view::surface::plain(), more: [0; 4] });
        }
    }
    let cloud = gpu.upload(&Points { centre: [0.0, 0.0, 0.0], radius: 400.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data });
    let cam = Camera { rect: [100.0, 0.0, 200.0, 200.0], ..plan(200.0, 200.0) };
    let look = Look { px: 1.0, ..Look::own() };
    let clouds = [(&cloud, IDENTITY, &look)];
    // Across the view from 50 px left of its corner (outside it) to 190 in, along row 100's middles.
    let across = batch(6, &[strip([-50.0, 0.0, 0.0], [190.0, 0.0, 0.0], [1.0, 1.0])], [1.0, 0.0, 0.0, 1.0], false);
    let kept = across.strips.clone();
    let l = Lines { list: list("a", vec![across]), scale: 1.0, e: 0.0, f: 100.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [300, 200], &cam, &clouds, Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 300, x, y);
    assert_eq!(at(200, 100), [255, 0, 0], "over the floor");
    assert_eq!(at(200, 90), [200, 200, 200], "the floor beside it");
    assert!(near(at(200, 99), over([1.0, 0.0, 0.0], [200, 200, 200], covers(HALF, 1.0, 0.0)), 2), "{:?}", at(200, 99));
    assert_eq!(at(110, 100), [255, 0, 0], "from the view's corner");
    assert_eq!(at(289, 100), [255, 0, 0]);
    assert_eq!(at(295, 100), clear, "past its end");
    assert_eq!(at(60, 100), clear, "not outside its view's part of the window");
    assert_eq!(at(99, 100), clear);
    assert_eq!(gpu.line_strips(), 1);
    // Left to the page for a frame (something selected): the strips stay on the card while the page's list holds them.
    let p = shot(gpu, [300, 200], &cam, &clouds, None);
    assert_eq!(pixel(&p, 300, 200, 100), [200, 200, 200]);
    assert_eq!(gpu.line_strips(), 1);
    // The page's list let go (but the strips held by another still): kept; then every holder gone: off the card.
    drop(l);
    shot(gpu, [300, 200], &cam, &clouds, None);
    assert_eq!(gpu.line_strips(), 1);
    drop(kept);
    shot(gpu, [300, 200], &cam, &clouds, None);
    assert_eq!(gpu.line_strips(), 0);
}

/// A batch of dashed lines.
fn dashed(id: u64, strips: &[[f32; 9]], rgba: [f32; 4], dash: &[f32]) -> LineItem {
    LineItem { dash: dash.to_vec(), ..batch(id, strips, rgba, false) }
}

#[test]
fn the_engine_s_dash_pattern_is_the_page_s_own() {
    // The page's dashHereOnCpu (gl-lines.ts) run on a spread of patterns, spans and places along: the same answers.
    let mut cases = 0;
    for line in include_str!("dash_cases.txt").lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let n: Vec<f64> = line.split_whitespace().map(|v| v.parse().expect("a number")).collect();
        let (along, span, gap, away) = (n[0], n[1], n[2], n[3]);
        let dash: Vec<f32> = n[4..].iter().map(|v| *v as f32).collect();
        let got = cadp_engine_view::dash_here(along, span, &dash);
        assert_eq!(got.0, gap, "in a gap or on a dash: {line}");
        assert!((got.1 - away).abs() < 1e-4, "its nearest dot: {} for {line}", got.1);
        cases += 1;
    }
    assert!(cases >= 400);
    assert_eq!(cadp_engine_view::period_of(&[10.0, 3.0, -1.0, 3.0]), 16.0);
}

/// A row of a picture against the pattern worked on the processor: each pixel along a dashed line across the plane
/// from `from` (its `along` there 0; `span` its CAD segment's length, 0: running on), at `scale` CSS pixels a unit and
/// `ratio` of the screen's pixels each. Pixels within a hair of a dash's edge are passed over.
#[allow(clippy::too_many_arguments)]
fn row_matches(p: &[u8], width: u32, y: u32, xs: std::ops::Range<u32>, from: f64, span: f64, dash: &[f32], ratio: f64, dot: f64, rgb: [f32; 3]) -> usize {
    let clear = clear_bytes();
    let mut checked = 0;
    for x in xs {
        let along = (x as f64 + 0.5) / ratio - from;
        let here = cadp_engine_view::dash_here(along, span, dash);
        // A hair either side: the same answer, or the pixel's on an edge.
        let a = cadp_engine_view::dash_here(along - 0.02, span, dash);
        let b = cadp_engine_view::dash_here(along + 0.02, span, dash);
        if a.0 != here.0 || b.0 != here.0 || (a.1 < 0.0) != (here.1 < 0.0) || (b.1 < 0.0) != (here.1 < 0.0) {
            continue;
        }
        let line = if here.0 > 0.5 { 0.0 } else { 1.0 };
        let spot = if here.1 >= 0.0 { (dot + 0.5 - here.1 * ratio).clamp(0.0, 1.0) } else { 0.0 };
        let want = over(rgb, clear, f64::max(line, spot));
        let got = pixel(p, width, x, y);
        assert!(near(got, want, 3), "at {x} ({along:.2} along): {got:?}, the pattern says {want:?} ({here:?})");
        checked += 1;
    }
    checked
}

#[test]
fn dashes_run_on_along_a_run_or_sit_centred_on_a_segment_and_dots_are_round() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(300.0, 120.0);
    let clear = clear_bytes();
    let yellow = [1.0f32, 1.0, 0.0];
    // A run of two segments from 10 to 250, the pattern running on along it (their third floats: how far along the run each end is).
    let run = [strip([10.0, 0.0, 0.0], [130.0, 0.0, 120.0], [1.0, 0.0]), strip([130.0, 0.0, 120.0], [250.0, 0.0, 240.0], [0.0, 1.0])];
    let pattern = [10.0f32, 5.0];
    let l = Lines { list: list("a", vec![dashed(1, &run, [1.0, 1.0, 0.0, 1.0], &pattern)]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 1.5, thin: 0.0 };
    let p = shot(gpu, [300, 120], &cam, &[], Some(&l));
    assert!(row_matches(&p, 300, 50, 12..248, 10.0, 0.0, &pattern, 1.0, 1.5, yellow) > 200);
    // (A dash and a gap, plainly.)
    assert_eq!(pixel(&p, 300, 15, 50), [255, 255, 0]);
    assert_eq!(pixel(&p, 300, 22, 50), clear);
    // The joint at 130 is mid-pattern: nothing special there.
    // Centred on a segment 100 long: it starts and ends on a dash at its vertices, six periods between.
    let seg = [[10.0f32, 0.0, 0.0, 110.0, 0.0, 100.0, 1.0, 1.0, 100.0]];
    let l = Lines { list: list("b", vec![dashed(2, &seg, [1.0, 1.0, 0.0, 1.0], &pattern)]), ..l };
    let p = shot(gpu, [300, 120], &cam, &[], Some(&l));
    assert!(row_matches(&p, 300, 50, 12..108, 10.0, 100.0, &pattern, 1.0, 1.5, yellow) > 80);
    assert_eq!(pixel(&p, 300, 12, 50), [255, 255, 0], "a dash at its start");
    assert_eq!(pixel(&p, 300, 107, 50), [255, 255, 0], "and at its end");
    // Too short for a whole period between its end dashes: solid.
    let short = [[10.0f32, 0.0, 0.0, 30.0, 0.0, 20.0, 1.0, 1.0, 20.0]];
    let l = Lines { list: list("c", vec![dashed(3, &short, [1.0, 1.0, 0.0, 1.0], &pattern)]), ..l };
    let p = shot(gpu, [300, 120], &cam, &[], Some(&l));
    assert!((11..29).all(|x| pixel(&p, 300, x, 50) == [255, 255, 0]));
    // Dots (a dash, a gap, a dot, a gap), on a screen of two pixels a CSS pixel and at twice the scale: each dot round,
    // as big across on the screen whatever the scale, the dashes twice as long.
    let dotted = [10.0f32, 3.0, -1.0, 3.0];
    let long = [[5.0f32, 0.0, 0.0, 65.0, 0.0, 60.0, 1.0, 1.0, 0.0]];
    let l = Lines { list: list("d", vec![dashed(4, &long, [1.0, 1.0, 0.0, 1.0], &dotted)]), scale: 2.0, e: 0.0, f: 25.25, world: None, half: HALF, ratio: 2.0, dot: 1.5, thin: 0.0 };
    let p = shot(gpu, [300, 120], &cam, &[], Some(&l));
    // Its `along` is the plane's 0 to 60 times the scale: 0 to 120 CSS pixels, from 10 CSS pixels in.
    assert!(row_matches(&p, 300, 50, 24..256, 10.0, 0.0, &dotted, 2.0, 1.5, yellow) > 150);
    // A dot's middle is 13 CSS pixels along each period of 16: at 23, 39, … CSS pixels; the screen's 46, 78, …
    assert_eq!(pixel(&p, 300, 46, 50), [255, 255, 0], "a dot's middle");
    assert_eq!(pixel(&p, 300, 78, 50), [255, 255, 0]);
    // Round: a pixel beside its middle, across the line, is covered too; two pixels off, not.
    assert!(near(pixel(&p, 300, 46, 51), over(yellow, clear, (1.5f64 + 0.5 - (0.5f64.hypot(1.0))).clamp(0.0, 1.0)), 40), "{:?}", pixel(&p, 300, 46, 51));
    assert_eq!(pixel(&p, 300, 46, 54), clear);
    // In the gap between the dash and the dot: nothing.
    assert_eq!(pixel(&p, 300, 42, 50), clear);
}

#[test]
fn a_filled_batch_is_under_the_lines() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(200.0, 120.0);
    let clear = clear_bytes();
    // Two triangles (a wide dashed polyline's dash: a band from 20 to 80 across, 10 either side of the origin's height), half solid blue.
    let band: Vec<f32> = vec![20.0, -10.0, 0.0, 80.0, -10.0, 0.0, 80.0, 10.0, 0.0, 20.0, -10.0, 0.0, 80.0, 10.0, 0.0, 20.0, 10.0, 0.0];
    let fill = LineItem { strips: Arc::new(Strips { id: 9, floats: band }), rgba: [0.0, 0.0, 1.0, 0.5], world: false, dash: Vec::new(), fill: true, thin: false, lettered: false };
    let line = batch(1, &[strip([10.0, 0.0, 0.0], [90.0, 0.0, 0.0], [1.0, 1.0])], [1.0, 0.0, 0.0, 1.0], false);
    // The line listed FIRST: the fill's still beneath it.
    let l = Lines { list: list("a", vec![line, fill]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    let blue = over([0.0, 0.0, 1.0], clear, 0.5);
    assert!(near(at(50, 45), blue, 2), "{:?}", at(50, 45));
    assert!(near(at(50, 58), blue, 2));
    assert_eq!(at(50, 50), [255, 0, 0], "the line over the fill");
    assert_eq!(at(50, 38), clear);
    assert_eq!(at(15, 45), clear);
    assert_eq!(at(85, 45), clear);
    assert_eq!(at(15, 50), [255, 0, 0]);
}

#[test]
fn text_s_letters_are_a_hairline_and_a_turning_view_s_face_the_screen_along_their_baseline() {
    let Some(mut card) = card() else { return };
    let gpu = &mut card.gpu;
    let cam = plan(200.0, 120.0);
    let clear = clear_bytes();
    let white = [1.0f32, 1.0, 1.0, 1.0];
    // A thin batch (letters in the view's plane): a hairline, one pixel wide, where a full line is a pixel and a quarter.
    let stroke = LineItem { thin: true, ..batch(1, &[strip([10.0, 0.0, 0.0], [90.0, 0.0, 0.0], [1.0, 1.0])], white, false) };
    let l = Lines { list: list("a", vec![stroke]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0, dot: 0.0, thin: 0.5 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    assert_eq!(pixel(&p, 200, 50, 50), [255, 255, 255]);
    assert_eq!(pixel(&p, 200, 50, 49), clear, "nothing of it a row away");
    assert_eq!(pixel(&p, 200, 50, 51), clear);
    // The same list with no thin width given: as wide as the others.
    let p = shot(gpu, [200, 120], &cam, &[], Some(&Lines { thin: 0.0, ..l.clone() }));
    assert!(near(pixel(&p, 200, 50, 49), over([1.0, 1.0, 1.0], clear, covers(HALF, 1.0, 0.0)), 2));
    // A turning view's letters (the page's letterStrip): the run's anchor in the drawing, each segment's two ends along
    // and above the baseline, both ends' kinds in one float (1 the first soft, 2 the second), and the baseline's angle.
    let letter = |anchor: [f32; 3], a: [f32; 2], b: [f32; 2], soft: f32, angle: f32| [anchor[0], anchor[1], anchor[2], a[0], a[1], b[0], b[1], soft, angle];
    // An L at 10, 20: its foot 30 along the baseline, its stem 12 above it at 5 along. The baseline runs east.
    let east = [letter([10.0, 20.0, 0.0], [0.0, 0.0], [30.0, 0.0], 3.0, 0.0), letter([10.0, 20.0, 0.0], [5.0, 0.0], [5.0, 12.0], 3.0, 0.0)];
    let letters = |id: u64, strips: &[[f32; 9]]| LineItem { thin: true, lettered: true, world: true, ..batch(id, strips, white, true) };
    // A plan view: the world's origin 100.5 px across the view and 60.5 down.
    let plan_view = LineWorld { right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], e: 100.5, f: 60.5 };
    let l = Lines { list: list("b", vec![letters(2, &east)]), scale: 1.0, e: -999.0, f: -999.0, world: Some(plan_view), half: HALF, ratio: 1.0, dot: 0.0, thin: 0.5 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    // The foot: from 110.5 to 140.5 across, 40.5 down. The stem: 115.5 across, from 40.5 up to 28.5.
    assert_eq!(at(125, 40), [255, 255, 255], "its foot along the baseline");
    assert_eq!(at(139, 40), [255, 255, 255]);
    assert_eq!(at(125, 39), clear);
    assert_eq!(at(115, 34), [255, 255, 255], "its stem above it");
    assert_eq!(at(115, 29), [255, 255, 255]);
    assert_eq!(at(115, 26), clear);
    assert_eq!(at(117, 34), clear);
    // The same letter with its baseline running north: its foot runs up the screen, its stem to the left of it.
    let north: Vec<[f32; 9]> = east.iter().map(|s| { let mut t = *s; t[8] = std::f32::consts::FRAC_PI_2; t }).collect();
    let l = Lines { list: list("c", vec![letters(3, &north)]), ..l };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    assert_eq!(at(110, 25), [255, 255, 255], "its foot up the screen");
    assert_eq!(at(110, 11), [255, 255, 255]);
    assert_eq!(at(104, 35), [255, 255, 255], "its stem to the left, 5 up the foot");
    assert_eq!(at(99, 35), [255, 255, 255]);
    assert_eq!(at(125, 40), clear);
    // The view turned a quarter (right is north, up is west): the east-running letter's foot runs down the screen.
    let turned = LineWorld { right: [0.0, 1.0, 0.0], up: [-1.0, 0.0, 0.0], e: 100.5, f: 60.5 };
    let l = Lines { list: list("d", vec![letters(4, &east)]), world: Some(turned), ..l };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    // Its anchor: 20 across the view's right (north), −10 up: 120.5 across, 70.5 down. The foot 30 down from there.
    assert_eq!(at(120, 85), [255, 255, 255]);
    assert_eq!(at(120, 99), [255, 255, 255]);
    assert_eq!(at(120, 104), clear);
    // No right and up: letters aren't drawn.
    let p = shot(gpu, [200, 120], &cam, &[], Some(&Lines { world: None, ..l }));
    assert!((0..120).all(|y| (0..200).all(|x| pixel(&p, 200, x, y) == clear)));
}
