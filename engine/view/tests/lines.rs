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
    LineItem { strips: Arc::new(Strips { id, floats: strips.iter().flatten().copied().collect() }), rgba, world }
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
    let l = Lines { list: list("a", vec![batch(1, &[strip([10.0, 0.0, 0.0], [90.0, 0.0, 0.0], [1.0, 1.0])], [1.0, 0.0, 0.0, 1.0], false)]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0 };
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
    let up = Lines { list: list("b", vec![batch(2, &[strip([20.0, 10.0, 0.0], [20.0, 30.0, 0.0], [1.0, 1.0])], [0.0, 1.0, 0.0, 1.0], false)]), scale: 2.0, e: 3.0, f: 70.0, world: None, half: HALF, ratio: 1.5 };
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
    let l = Lines { list: list("a", vec![run.clone(), down.clone()]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0 };
    let p = shot(gpu, [200, 120], &cam, &[], Some(&l));
    let at = |x: u32, y: u32| pixel(&p, 200, x, y);
    let once = over(green, clear, 0.5);
    for x in [30, 48, 49, 50, 51, 60] {
        assert!(near(at(x, 50), once, 2), "at {x}: {:?}, once is {once:?}", at(x, 50));
    }
    assert_eq!(at(70, 50), [0, 0, 255], "the later batch over the earlier");
    assert_eq!(at(70, 40), [0, 0, 255]);
    // The other way round: the run over the line.
    let l = Lines { list: list("b", vec![down, run]), scale: 1.0, e: 0.0, f: 50.5, world: None, half: HALF, ratio: 1.0 };
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
    let l = Lines { list: list("a", vec![b.clone()]), scale: 1.0, e: -999.0, f: -999.0, world: Some(front), half: HALF, ratio: 1.0 };
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
    let l = Lines { list: list("a", vec![across]), scale: 1.0, e: 0.0, f: 100.5, world: None, half: HALF, ratio: 1.0 };
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
