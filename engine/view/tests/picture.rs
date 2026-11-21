//! A frame drawn on the card into a picture and read back: the check's ring
//! where the view puts it, and a scan's sample from its index.

use cadp_engine_core::crop::CropBox;
use cadp_engine_view::brush::pack;
use cadp_engine_view::{test_ring, Camera, Clip, Edl, Gpu, Look, PointVertex, Points, Preview, Shading, Stroke, StrokeOp, Strokes, ViewDraw, BY_CLASS, BY_HEIGHT, BY_INTENSITY, BY_ONE, CLEAR, CLIP_BOXES, IDENTITY, NO_STROKES};

/// One test draws at a time: the build computer's stand-in for a card (Windows' own, on the processor) has
/// stopped the whole run when several tests each started and ended a card of their own at once. The app has one.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A card and the turn to draw on it.
struct Card {
    gpu: Gpu,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for Card {
    type Target = Gpu;
    fn deref(&self) -> &Gpu {
        &self.gpu
    }
}

impl std::ops::DerefMut for Card {
    fn deref_mut(&mut self) -> &mut Gpu {
        &mut self.gpu
    }
}

/// The card, or none on a computer with no way to draw (the test then says so and passes).
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

fn clear_bytes() -> [u8; 3] {
    [(CLEAR[0] * 255.0).round() as u8, (CLEAR[1] * 255.0).round() as u8, (CLEAR[2] * 255.0).round() as u8]
}

fn near(a: [u8; 3], b: [u8; 3], slack: i32) -> bool {
    (0..3).all(|k| (a[k] as i32 - b[k] as i32).abs() <= slack)
}

/// Plan view of a window `size` square, a unit a pixel, looking at 0, 0.
fn plan(size: f64) -> Camera {
    Camera { target: [0.0, 0.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 1.0, 0.0], to_eye: [0.0, 0.0, 1.0], units_per_pixel: 1.0, rect: [0.0, 0.0, size, size] }
}

fn point(x: f32, y: f32, z: f32, rgb: [u8; 3], class: u8, intensity: u16) -> PointVertex {
    PointVertex { pos: [x, y, z], rgb, class, intensity, setup: 0, shade: cadp_engine_view::surface::plain(), more: [0; 4] }
}

#[test]
fn the_ring_is_drawn_where_the_view_puts_it() {
    let Some(mut gpu) = card() else { return };
    // A drawing far from 0, the ring's middle 40 east of the target; a window 400 × 300 whose
    // drawing area is its right 300 px.
    let centre = [6_000_000.0, 2_000_000.0, 100.0];
    let ring = gpu.upload(&test_ring(6000, centre));
    let cam = Camera {
        target: [centre[0] - 40.0, centre[1], centre[2]],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        to_eye: [0.0, 0.0, 1.0],
        units_per_pixel: 1.0,
        rect: [100.0, 0.0, 300.0, 300.0],
    };
    let look = Look { px: 3.0, ..Look::own() };
    let picture = gpu.picture([400, 300], Some(&cam), &[(&ring, IDENTITY, &look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    assert_eq!(picture.len(), 400 * 300 * 4);
    // The drawing area's middle is pixel 250, 150; the ring's is 40 px right of it: its white dot.
    assert!(near(pixel(&picture, 400, 290, 150), [255, 255, 255], 2), "{:?}", pixel(&picture, 400, 290, 150));
    // The ring itself 50 px from its middle: straight up, and to the left.
    let up = pixel(&picture, 400, 290, 100);
    let left = pixel(&picture, 400, 240, 150);
    assert!(!near(up, clear_bytes(), 12), "{up:?}");
    assert!(!near(left, clear_bytes(), 12), "{left:?}");
    // Between the dot and the ring: nothing.
    assert!(near(pixel(&picture, 400, 290, 125), clear_bytes(), 2));
    // Outside the drawing area nothing's drawn, though the ring would reach there (its right side is cut at 400).
    assert!(near(pixel(&picture, 400, 50, 150), clear_bytes(), 2));
    let mut outside = 0;
    for y in 0..300 {
        for x in 0..100 {
            if !near(pixel(&picture, 400, x, y), clear_bytes(), 2) {
                outside += 1;
            }
        }
    }
    assert_eq!(outside, 0);
}

#[test]
fn nearer_points_cover_further_ones_and_no_view_draws_the_background() {
    let Some(mut gpu) = card() else { return };
    // Two rings one over the other: the upper one's white dot (1 high) over the lower one's.
    let low = gpu.upload(&test_ring(600, [0.0, 0.0, 0.0]));
    let mut high_points = test_ring(600, [0.0, 0.0, 5.0]);
    for v in &mut high_points.data {
        v.rgb = [255, 0, 0];
    }
    let high = gpu.upload(&high_points);
    let cam = plan(200.0);
    let look = Look { px: 4.0, ..Look::own() };
    // Whichever is drawn first, the higher (nearer the eye, from overhead) shows.
    for order in [[&low, &high], [&high, &low]] {
        let picture = gpu.picture([200, 200], Some(&cam), &[(order[0], IDENTITY, &look), (order[1], IDENTITY, &look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
        assert!(near(pixel(&picture, 200, 100, 100), [255, 0, 0], 2), "{:?}", pixel(&picture, 200, 100, 100));
    }
    let empty = gpu.picture([64, 64], None, &[(&low, IDENTITY, &look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    assert!(near(pixel(&empty, 64, 32, 32), clear_bytes(), 2));
    // A background of the page's own.
    let own = gpu.picture([64, 64], None, &[], &Edl::default(), &Shading::default(), &Clip::default(), [0.2, 0.4, 0.6, 1.0]).expect("a picture");
    assert!(near(pixel(&own, 64, 5, 60), [51, 102, 153], 1));
}

#[test]
fn points_are_coloured_as_the_page_colours_them() {
    let Some(mut gpu) = card() else { return };
    // Four points 40 apart across the middle, each 9 px: their own colours, classes 2, 6, 7 and 2, heights 0 to 30, intensities 100 to 4000.
    let data = vec![
        point(-60.0, 0.0, 0.0, [200, 10, 10], 2, 100),
        point(-20.0, 0.0, 10.0, [10, 200, 10], 6, 1000),
        point(20.0, 0.0, 20.0, [10, 10, 200], 7, 2000),
        point(60.0, 0.0, 30.0, [200, 200, 10], 2, 4000),
    ];
    let cloud = gpu.upload(&Points { centre: [1000.0, 2000.0, 50.0], radius: 80.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data: data.clone() });
    let cam = Camera { target: [1000.0, 2000.0, 50.0], ..plan(200.0) };
    let at = |k: usize| 100 + [-60i32, -20, 20, 60][k];
    let shot = |gpu: &mut Gpu, cloud: &cadp_engine_view::Cloud, look: &Look| gpu.picture([200, 200], Some(&cam), &[(cloud, IDENTITY, look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    let base = Look { px: 9.0, ..Look::own() };
    // Their own colours.
    let p = shot(&mut gpu, &cloud, &base);
    for (k, want) in [[200, 10, 10], [10, 200, 10], [10, 10, 200], [200, 200, 10]].iter().enumerate() {
        assert!(near(pixel(&p, 200, at(k) as u32, 100), *want, 1), "own {k}: {:?}", pixel(&p, 200, at(k) as u32, 100));
    }
    // By class: each its class's colour; a class whose layer's off isn't there.
    let mut classes = [[175, 175, 175, 255]; 256];
    classes[2] = [120, 80, 40, 255];
    classes[6] = [220, 30, 30, 255];
    classes[7] = [9, 9, 9, 0];
    let by_class = Look { by: BY_CLASS, classes, ..base };
    let p = shot(&mut gpu, &cloud, &by_class);
    assert!(near(pixel(&p, 200, at(0) as u32, 100), [120, 80, 40], 1));
    assert!(near(pixel(&p, 200, at(1) as u32, 100), [220, 30, 30], 1));
    assert!(near(pixel(&p, 200, at(2) as u32, 100), clear_bytes(), 1), "{:?}", pixel(&p, 200, at(2) as u32, 100));
    assert!(near(pixel(&p, 200, at(3) as u32, 100), [120, 80, 40], 1));
    // A class turned off hides it in every colouring.
    let p = shot(&mut gpu, &cloud, &Look { classes, ..base });
    assert!(near(pixel(&p, 200, at(2) as u32, 100), clear_bytes(), 1));
    // By height, over the drawing's 50 to 80 (the scan's middle is at 50, its points 0 to 30 over it): the page's colours low to high.
    let by_height = Look { by: BY_HEIGHT, heights: [50.0, 80.0], ..base };
    let p = shot(&mut gpu, &cloud, &by_height);
    assert!(near(pixel(&p, 200, at(0) as u32, 100), [59, 82, 139], 2), "{:?}", pixel(&p, 200, at(0) as u32, 100));
    assert!(near(pixel(&p, 200, at(3) as u32, 100), [189, 223, 38], 2), "{:?}", pixel(&p, 200, at(3) as u32, 100));
    // A third of the way: between the third and fourth colours (35, 136, 142).
    assert!(near(pixel(&p, 200, at(1) as u32, 100), [35, 136, 142], 2), "{:?}", pixel(&p, 200, at(1) as u32, 100));
    // A scan with no colours of its own, asked for them: by height.
    let plain = gpu.upload(&Points { centre: [1000.0, 2000.0, 50.0], radius: 80.0, colours: false, shaded: false, shade_ms: 0.0, gap: 0.0, data: data.clone() });
    let p = shot(&mut gpu, &plain, &Look { heights: [50.0, 80.0], ..base });
    assert!(near(pixel(&p, 200, at(3) as u32, 100), [189, 223, 38], 2));
    // By intensity, read on 100 to 2000: 40 at the weakest, 255 at and past the strongest.
    let by_intensity = Look { by: BY_INTENSITY, intensity: [100.0, 2000.0], ..base };
    let p = shot(&mut gpu, &cloud, &by_intensity);
    assert!(near(pixel(&p, 200, at(0) as u32, 100), [40, 40, 40], 1));
    let mid = (40.0 + 215.0 * (900.0 / 1900.0f64)).round() as u8;
    assert!(near(pixel(&p, 200, at(1) as u32, 100), [mid, mid, mid], 2), "{:?}", pixel(&p, 200, at(1) as u32, 100));
    assert!(near(pixel(&p, 200, at(3) as u32, 100), [255, 255, 255], 1));
    // One colour.
    let p = shot(&mut gpu, &cloud, &Look { by: BY_ONE, one: [0.2, 0.6, 1.0], ..base });
    assert!(near(pixel(&p, 200, at(1) as u32, 100), [51, 153, 255], 1));
    // Intensity mixed in at 60%, read on 100 to 4000: the weakest 1 − 0.65 × 0.6 of its colour, the strongest 1 + 0.25 × 0.6 (never past white).
    let mixed = Look { by: BY_CLASS, classes, mix: [100.0, 4000.0, 0.6], ..base };
    let p = shot(&mut gpu, &cloud, &mixed);
    let weak = |v: f64| (v * (1.0 - 0.65 * 0.6)).round() as u8;
    assert!(near(pixel(&p, 200, at(0) as u32, 100), [weak(120.0), weak(80.0), weak(40.0)], 2), "{:?}", pixel(&p, 200, at(0) as u32, 100));
    let strong = |v: f64| (v * 1.15).min(255.0).round() as u8;
    assert!(near(pixel(&p, 200, at(3) as u32, 100), [strong(120.0), strong(80.0), strong(40.0)], 2), "{:?}", pixel(&p, 200, at(3) as u32, 100));
    // Squares fill their corners; discs don't.
    let p = shot(&mut gpu, &cloud, &base);
    assert!(near(pixel(&p, 200, at(0) as u32 - 4, 96), [200, 10, 10], 1));
    let p = shot(&mut gpu, &cloud, &Look { round: true, ..base });
    assert!(near(pixel(&p, 200, at(0) as u32 - 4, 96), clear_bytes(), 1));
    assert!(near(pixel(&p, 200, at(0) as u32, 100), [200, 10, 10], 1));
}

#[test]
fn eye_dome_lighting_darkens_where_the_ground_steps_as_the_pages_does() {
    let Some(mut gpu) = card() else { return };
    // A floor of grey points a unit apart, the right half 10 units higher (nearer the eye from overhead).
    let mut data = Vec::new();
    for i in -60..60 {
        for j in -60..60 {
            data.push(point(i as f32 + 0.5, j as f32 + 0.5, if i >= 0 { 10.0 } else { 0.0 }, [200, 200, 200], 2, 0));
        }
    }
    let cloud = gpu.upload(&Points { centre: [0.0, 0.0, 0.0], radius: 100.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data });
    let cam = plan(200.0);
    let look = Look { px: 2.0, ..Look::own() };
    let shot = |gpu: &mut Gpu, edl: &Edl| gpu.picture([200, 200], Some(&cam), &[(&cloud, IDENTITY, &look)], edl, &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    // Off: every point its own colour.
    let flat = shot(&mut gpu, &Edl::default());
    assert!(near(pixel(&flat, 200, 99, 100), [200, 200, 200], 1));
    // On (Standard: strength 1, radius 1.4 px, the eye 0.055 of the width off): the lower side of the step darkens next to it, never under 0.45 of its colour.
    let edl = Edl { strength: 1.0, radius: 1.4, eye: 200.0 * 0.055, noise: 0.0 };
    let lit = shot(&mut gpu, &edl);
    let row: Vec<u8> = (94..106).map(|x| pixel(&lit, 200, x, 100)[0]).collect();
    // The last pixel of the lower side (whichever the card gives the step's own edge to): the darkest allowed, a 10 px step being most of the eye's 11.
    let darkest = (200.0 * 0.45f64).round() as u8;
    let dark: Vec<usize> = row.iter().enumerate().filter(|(_, v)| **v < 150).map(|(k, _)| k).collect();
    assert!(dark.len() == 1 && (4..=5).contains(&dark[0]), "{row:?}");
    assert!((row[dark[0]] as i32 - darkest as i32).abs() <= 1, "{row:?}");
    // The higher side, and the flat ground away from the step, aren't.
    assert!(near(pixel(&lit, 200, 101, 100), [200, 200, 200], 1), "{:?}", pixel(&lit, 200, 101, 100));
    assert!(near(pixel(&lit, 200, 60, 100), [200, 200, 200], 1));
    assert!(near(pixel(&lit, 200, 140, 100), [200, 200, 200], 1));
    // A step smaller than the scanner's noise is left alone.
    let noisy = shot(&mut gpu, &Edl { noise: 11.0, ..edl });
    assert!((94..106).all(|x| near(pixel(&noisy, 200, x, 100), [200, 200, 200], 1)));
    // Far weaker (exp(−0.03 × three neighbours' log2(11))): lighter than Standard's, darker than none.
    let soft = shot(&mut gpu, &Edl { strength: 0.03, ..edl });
    let v = pixel(&soft, 200, 94 + dark[0] as u32, 100)[0];
    assert!(v > darkest + 20 && v < 190, "{v}");
    // Where there's no point: the background, not darkened.
    let far = gpu.picture([200, 200], Some(&Camera { target: [500.0, 0.0, 0.0], ..cam }), &[(&cloud, IDENTITY, &look)], &edl, &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    assert!(near(pixel(&far, 200, 100, 100), clear_bytes(), 1));
}

/// A colour's byte made `f` times as bright in linear light, as the page's shadeDetail makes it.
fn brightened(v: u8, f: f64) -> u8 {
    let c = v as f64 / 255.0;
    let linear = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let t = (linear * f).min(1.0);
    let s = if t <= 0.0031308 { t * 12.92 } else { 1.055 * t.powf(1.0 / 2.4) - 0.055 };
    (s.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[test]
fn points_are_brightened_and_darkened_by_the_shape_of_their_surface_as_the_page_s_are() {
    let Some(mut gpu) = card() else { return };
    // Six points 30 apart, each with its own shading: nothing known; lit full on; well below the surface
    // round it; on an edge; half the sky hidden; and a dark one well below its surface.
    let shades: [[u8; 4]; 6] = [cadp_engine_view::surface::plain(), [255, 0, 0, 255], [214, (-127i8) as u8, 0, 255], [214, 0, 255, 255], [214, 0, 0, 128], [214, (-127i8) as u8, 0, 255]];
    let colour = |k: usize| if k == 5 { [8, 8, 8] } else { [180, 120, 60] };
    let data: Vec<PointVertex> = shades.iter().enumerate().map(|(k, s)| PointVertex { shade: *s, ..point(k as f32 * 30.0 - 75.0, 0.0, 0.0, colour(k), 2, 0) }).collect();
    let shaded = gpu.upload(&Points { centre: [0.0; 3], radius: 100.0, colours: true, shaded: true, shade_ms: 0.0, gap: 0.0, data: data.clone() });
    let cam = plan(200.0);
    let shot = |gpu: &mut Gpu, cloud: &cadp_engine_view::Cloud, look: &Look| gpu.picture([200, 200], Some(&cam), &[(cloud, IDENTITY, look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    let at = |p: &[u8], k: usize| pixel(p, 200, (100.0 + k as f32 * 30.0 - 75.0) as u32, 100);
    let base = Look { px: 9.0, ..Look::own() };
    // No strengths: each its own colour.
    let p = shot(&mut gpu, &shaded, &base);
    for k in 0..6 {
        assert!(near(at(&p, k), colour(k), 1), "{k}: {:?}", at(&p, k));
    }
    // The page's start: lighting 60, detail 55, hollows 50.
    let look = Look { shade: [0.6, 0.55, 0.5], ..base };
    let p = shot(&mut gpu, &shaded, &look);
    let level = cadp_engine_view::surface::light_level();
    let lit = |light: f64| (1.0 + 0.6 * (light / 255.0 - level)).clamp(0.75, 1.25);
    let factors = [
        lit(214.0),
        lit(255.0),
        lit(214.0) * (1.0f64 - 0.55 * 0.55).clamp(0.7, 1.3),
        lit(214.0) * (1.0f64 - 0.55 * 0.2).clamp(0.88, 1.0),
        lit(214.0) * (1.0f64 - 0.5 * 2.0 * (1.0 - 128.0 / 255.0)).max(0.5),
        lit(214.0) * (1.0f64 - 0.55 * 0.55).clamp(0.7, 1.3),
    ];
    for k in 0..6 {
        let c = colour(k);
        let want = [brightened(c[0], factors[k]), brightened(c[1], factors[k]), brightened(c[2], factors[k])];
        assert!(near(at(&p, k), want, 2), "{k}: {:?} for {want:?} (×{:.3})", at(&p, k), factors[k]);
    }
    // Level ground's light leaves a colour as it is; full on is brighter; the rest darker.
    assert!(near(at(&p, 0), [180, 120, 60], 1));
    assert!(at(&p, 1)[0] > 185 && at(&p, 2)[0] < 160 && at(&p, 3)[0] < 175 && at(&p, 4)[0] < 140);
    // Each strength its own: lighting alone leaves the hollow and the edge as they were.
    let p = shot(&mut gpu, &shaded, &Look { shade: [0.6, 0.0, 0.0], ..base });
    assert!(near(at(&p, 3), [180, 120, 60], 1) && near(at(&p, 4), [180, 120, 60], 1) && at(&p, 1)[0] > 185);
    // Points whose shading wasn't worked out are drawn as they are, whatever's asked.
    let plain = gpu.upload(&Points { centre: [0.0; 3], radius: 100.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data });
    let p = shot(&mut gpu, &plain, &look);
    for k in 0..6 {
        assert!(near(at(&p, k), colour(k), 1), "{k}: {:?}", at(&p, k));
    }
}

/// A floor of grey points a unit apart (a pixel each in the plan view), each one's height by where it is.
fn ground(height: impl Fn(f32, f32) -> f32) -> Points {
    let mut data = Vec::new();
    for i in -80..80 {
        for j in -80..80 {
            let (x, y) = (i as f32 + 0.5, j as f32 + 0.5);
            data.push(point(x, y, height(x, y), [200, 200, 200], 2, 0));
        }
    }
    Points { centre: [0.0, 0.0, 0.0], radius: 400.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data }
}

#[test]
fn depth_shading_darkens_a_fold_and_leaves_a_slope_as_the_pages_does() {
    let Some(mut gpu) = card() else { return };
    // West of x = 20 a slope down to the east (4 a unit), then level ground: a fold along x = 20, its
    // foot the level ground's first point. East of x = 50 a slope up (1 a unit).
    let cloud = gpu.upload(&ground(|x, _| if x < 20.0 { (20.0 - x) * 4.0 } else if x > 50.0 { x - 50.0 } else { 0.0 }));
    let cam = plan(200.0);
    let look = Look { px: 1.0, ..Look::own() };
    let shot = |gpu: &mut Gpu, shading: &Shading| gpu.picture([200, 200], Some(&cam), &[(&cloud, IDENTITY, &look)], &Edl::default(), shading, &Clip::default(), CLEAR).expect("a picture");
    let grey = |p: &[u8], x: u32| pixel(p, 200, x, 100)[0];
    // Off: every point its own colour.
    let off = shot(&mut gpu, &Shading::default());
    assert!((60..180).all(|x| grey(&off, x) == 200));
    // The page's strongest (100: 0.7): the fold's foot (the pixel at x 20.5) has the slope's first point
    // 2 nearer on its west and level ground on its east: half of 2 a pixel, less the noise's half: 0.5.
    let on = shot(&mut gpu, &Shading { depth: 0.7, ..Shading::default() });
    let foot = (200.0 * (-0.7f64 * 0.5).exp()).round() as i32;
    assert!((grey(&on, 120) as i32 - foot).abs() <= 1, "the fold's foot: {} for {foot}", grey(&on, 120));
    // The slopes either side, the level ground between, and the second fold's far gentler foot (half of 1, the noise's worth): as they were.
    for x in [70, 100, 110, 118, 122, 135, 149, 150, 151, 160, 175] {
        assert_eq!(grey(&on, x), 200, "at {x}");
    }
    // Shown faint (the page's Show switches: 0.3 of their strength): the background through the rest, the fold still darker than the ground.
    let faint = shot(&mut gpu, &Shading { depth: 0.7, fade: 0.7, ..Shading::default() });
    let bg = clear_bytes()[0] as f64;
    assert!((grey(&faint, 100) as f64 - (bg + (200.0 - bg) * 0.3)).abs() <= 1.5, "faint ground: {}", grey(&faint, 100));
    assert!((grey(&faint, 120) as f64 - (bg + (foot as f64 - bg) * 0.3)).abs() <= 1.5, "the faint fold's foot: {}", grey(&faint, 120));
    // Weaker (the page's start, 20: 0.14): lighter than the strongest, darker than none.
    let soft = shot(&mut gpu, &Shading { depth: 0.14, ..Shading::default() });
    let want = (200.0 * (-0.14f64 * 0.5).exp()).round() as i32;
    assert!((grey(&soft, 120) as i32 - want).abs() <= 1, "{} for {want}", grey(&soft, 120));
    // Bigger points look further off for a neighbour (a point's width away, not the point's own square).
    assert_eq!(cadp_engine_view::shading_reach(1.0), (1.0, 2.0));
    assert_eq!(cadp_engine_view::shading_reach(2.0), (2.0, 5.0));
    assert_eq!(cadp_engine_view::shading_reach(40.0), (24.0, 24.0));
}

#[test]
fn relief_lights_a_slope_from_the_upper_left_and_face_shading_darkens_a_steep_one() {
    let Some(mut gpu) = card() else { return };
    // Four slopes of 0.2 a unit about a level middle: up to the west, east, north and south.
    let cloud = gpu.upload(&ground(|x, y| {
        if x.abs() < 20.0 && y.abs() < 20.0 {
            0.0
        } else if x.abs() > y.abs() {
            (x.abs() - 20.0) * 0.2
        } else {
            (y.abs() - 20.0) * 0.2
        }
    }));
    let cam = plan(200.0);
    let look = Look { px: 1.0, ..Look::own() };
    let shot = |gpu: &mut Gpu, cloud: &cadp_engine_view::Cloud, cam: &Camera, shading: &Shading| gpu.picture([200, 200], Some(cam), &[(cloud, IDENTITY, &look)], &Edl::default(), shading, &Clip::default(), CLEAR).expect("a picture");
    // Relief at 50 (the slopes steepened 3 times): the light's from the upper left, 45° up.
    let p = shot(&mut gpu, &cloud, &cam, &Shading { relief: 3.0, ..Shading::default() });
    let lit = |gx: f64, gy: f64| {
        let n = [-gx * 3.0, -gy * 3.0, 1.0];
        let len = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
        ((n[0] * -0.5 + n[1] * 0.5 + std::f64::consts::FRAC_1_SQRT_2) / len / std::f64::consts::FRAC_1_SQRT_2).clamp(0.45, 1.35)
    };
    let grey = |p: &[u8], x: u32, y: u32| pixel(p, 200, x, y)[0] as i32;
    let want = |f: f64| (200.0 * f).min(255.0).round() as i32;
    // Level: as it was.
    assert_eq!(grey(&p, 100, 100), 200);
    // East of the middle the ground rises to the east (nearer to the right): it faces west, toward the light's side.
    assert!((grey(&p, 160, 100) - want(lit(0.2, 0.0))).abs() <= 2, "east: {} for {}", grey(&p, 160, 100), want(lit(0.2, 0.0)));
    assert!((grey(&p, 40, 100) - want(lit(-0.2, 0.0))).abs() <= 2, "west: {} for {}", grey(&p, 40, 100), want(lit(-0.2, 0.0)));
    // North (up the screen) it rises upward: it faces down the screen, away from the light above.
    assert!((grey(&p, 100, 40) - want(lit(0.0, 0.2))).abs() <= 2, "north: {} for {}", grey(&p, 100, 40), want(lit(0.0, 0.2)));
    assert!((grey(&p, 100, 160) - want(lit(0.0, -0.2))).abs() <= 2, "south: {} for {}", grey(&p, 100, 160), want(lit(0.0, -0.2)));
    assert!(grey(&p, 160, 100) > 215 && grey(&p, 100, 160) > 215 && grey(&p, 40, 100) < 120 && grey(&p, 100, 40) < 120);
    // Face shading at 100 (0.6 of a wall's darkening): a slope of 1 in 1 stands 45° against Z.
    let steep = gpu.upload(&ground(|x, _| if x > 0.0 { x } else { 0.0 }));
    let faces = Shading { faces: 0.6, ..Shading::default() };
    let p = shot(&mut gpu, &steep, &cam, &faces);
    assert_eq!(grey(&p, 60, 100), 200);
    let slope = want(1.0 - 0.6 * (1.0 - std::f64::consts::FRAC_1_SQRT_2));
    assert!((grey(&p, 150, 100) - slope).abs() <= 1, "a 45° slope: {} for {slope}", grey(&p, 150, 100));
    // The crease stays a crease: the level ground's last point takes the level side's way, not a blend.
    assert_eq!(grey(&p, 99, 100), 200);
    // Level ground looked at from the south, 40° down from overhead: still level against Z (its depth
    // falls away up the screen, which is the view's doing, not the ground's), so as it was.
    let (s, c) = (40f64.to_radians().sin(), 40f64.to_radians().cos());
    let tilted = Camera { target: [0.0, 0.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, c, s], to_eye: [0.0, -s, c], units_per_pixel: 1.0, rect: [0.0, 0.0, 200.0, 200.0] };
    let level = gpu.upload(&ground(|_, _| 0.0));
    let p = shot(&mut gpu, &level, &tilted, &faces);
    for (x, y) in [(100, 100), (70, 80), (130, 120)] {
        assert!((grey(&p, x, y) - 200).abs() <= 3, "level ground from the side at {x}, {y}: {}", grey(&p, x, y));
    }
    // And a wall there (the slope of 1 in 1 from this side is steeper still against the eye, the same against Z).
    let p = shot(&mut gpu, &steep, &tilted, &faces);
    assert!((grey(&p, 150, 100) - slope).abs() <= 8, "the slope from the side: {} for {slope}", grey(&p, 150, 100));
}

/// A limit box as the page gives it: its turn undone, a millionth of slack.
fn limit(min: [f64; 3], max: [f64; 3], angle: f64, outside: bool) -> CropBox {
    let a = -angle.to_radians();
    CropBox { min, max, centre: [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0], cos: a.cos(), sin: a.sin(), turned: a != 0.0, outside, slack: [1e-6; 3] }
}

#[test]
fn the_card_leaves_out_what_the_limit_boxes_and_the_slice_don_t_show() {
    let Some(mut gpu) = card() else { return };
    // A floor far from 0 (the boxes are in the drawing), rising to the east: its height the distance east of its middle, a tenth.
    let far = [6_000_000.0, 2_000_000.0, 100.0];
    let mut floor = ground(|x, _| x * 0.1);
    floor.centre = far;
    let cloud = gpu.upload(&floor);
    let cam = Camera { target: far, ..plan(200.0) };
    let look = Look { px: 1.0, ..Look::own() };
    let shot = |gpu: &mut Gpu, affine: &cadp_engine_view::Affine, clip: &Clip| gpu.picture([200, 200], Some(&cam), &[(&cloud, *affine, &look)], &Edl::default(), &Shading::default(), clip, CLEAR).expect("a picture");
    let on = |p: &[u8], x: u32, y: u32| pixel(p, 200, x, y) == [200, 200, 200];
    let off = |p: &[u8], x: u32, y: u32| near(pixel(p, 200, x, y), clear_bytes(), 1);
    let boxed = |min: [f64; 2], max: [f64; 2], angle: f64, outside: bool| limit([far[0] + min[0], far[1] + min[1], 0.0], [far[0] + max[0], far[1] + max[1], 500.0], angle, outside);
    let clip = |boxes: Vec<CropBox>, slice: Option<CropBox>| Clip { boxes, slice, key: String::new() }.keyed();
    // None: the whole floor.
    let p = shot(&mut gpu, &IDENTITY, &Clip::default());
    assert!(on(&p, 100, 100) && on(&p, 30, 30) && on(&p, 170, 170));
    // One box showing its inside: 40 east to 60 east, 10 south to 30 north (up the screen is north).
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![boxed([40.0, -10.0], [60.0, 30.0], 0.0, false)], None));
    assert!(on(&p, 150, 100) && on(&p, 141, 72) && on(&p, 159, 109), "inside the box");
    assert!(off(&p, 100, 100) && off(&p, 138, 100) && off(&p, 162, 100) && off(&p, 150, 68) && off(&p, 150, 112), "outside it");
    // A second box: in either. And one showing what's outside it: a hole in the first.
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![boxed([40.0, -10.0], [60.0, 30.0], 0.0, false), boxed([-60.0, -60.0], [-40.0, -40.0], 0.0, false), boxed([45.0, 0.0], [55.0, 10.0], 0.0, true)], None));
    assert!(on(&p, 150, 80) && on(&p, 50, 150) && off(&p, 150, 95) && off(&p, 100, 100));
    // Only a hole: everything but its inside.
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![boxed([45.0, 0.0], [55.0, 10.0], 0.0, true)], None));
    assert!(on(&p, 100, 100) && on(&p, 30, 30) && off(&p, 150, 95) && on(&p, 143, 95) && on(&p, 157, 95));
    // Turned 45° about its middle (0, 0), 80 long and 10 wide: along the north-east diagonal.
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![boxed([-40.0, -5.0], [40.0, 5.0], 45.0, false)], None));
    assert!(on(&p, 100, 100) && on(&p, 125, 75) && on(&p, 75, 125), "along the turned box");
    assert!(off(&p, 125, 125) && off(&p, 75, 75) && off(&p, 135, 100) && off(&p, 131, 69), "off it and past its end");
    // A slice: the level band 1 thick at 102 is the floor from 15 to 25 east of its middle.
    let band = limit([-1e12, -1e12, 101.5], [1e12, 1e12, 102.5], 0.0, false);
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![], Some(band.clone())));
    assert!(on(&p, 120, 100) && on(&p, 116, 30) && on(&p, 124, 170) && off(&p, 113, 100) && off(&p, 127, 100) && off(&p, 100, 100));
    // With a box: where both show.
    let p = shot(&mut gpu, &IDENTITY, &clip(vec![boxed([-100.0, 0.0], [100.0, 20.0], 0.0, false)], Some(band)));
    assert!(on(&p, 120, 90) && off(&p, 120, 110) && off(&p, 100, 90));
    // A scan moved in the drawing (turned a quarter turn about its middle and 30 north): the boxes are the drawing's, not the scan's.
    let quarter = [0.0, -1.0, 0.0, far[0] + far[1], 1.0, 0.0, 0.0, far[1] - far[0] + 30.0, 0.0, 0.0, 1.0, 0.0];
    let p = shot(&mut gpu, &quarter, &clip(vec![boxed([40.0, -10.0], [60.0, 30.0], 0.0, false)], None));
    assert!(on(&p, 150, 100) && on(&p, 141, 72) && off(&p, 138, 100) && off(&p, 150, 112) && off(&p, 100, 100), "the same place in the drawing");
}

#[test]
fn fill_gaps_draws_each_point_as_big_as_its_spacing() {
    let Some(mut gpu) = card() else { return };
    // Points 6 apart along a row (a pixel a unit): their spacing 6 (the byte's 128: the average, 6); one with none known; one twice as far from its neighbours.
    let mut data: Vec<PointVertex> = (-5..=5).map(|i| PointVertex { more: [128, 0, 0, 0], ..point(i as f32 * 6.0 + 0.5, 0.5, 0.0, [200, 200, 200], 2, 0) }).collect();
    data.push(point(0.5, 30.5, 0.0, [200, 200, 200], 2, 0));
    data.push(PointVertex { more: [144, 0, 0, 0], ..point(0.5, -30.5, 0.0, [200, 200, 200], 2, 0) });
    let measured = gpu.upload(&Points { centre: [0.0; 3], radius: 100.0, colours: true, shaded: false, shade_ms: 0.0, gap: 6.0, data: data.clone() });
    let cam = plan(200.0);
    let shot = |gpu: &mut Gpu, cloud: &cadp_engine_view::Cloud, look: &Look, cam: &Camera| gpu.picture([200, 200], Some(cam), &[(cloud, IDENTITY, look)], &Edl::default(), &Shading::default(), &Clip::default(), CLEAR).expect("a picture");
    let on = |p: &[u8], x: u32, y: u32| pixel(p, 200, x, y) == [200, 200, 200];
    let run = |p: &[u8], y: u32| (60..140).filter(|x| on(p, *x, y)).count();
    // Off: a pixel each, 5 of background between them.
    let base = Look { px: 1.0, ..Look::own() };
    let p = shot(&mut gpu, &measured, &base, &cam);
    assert_eq!(run(&p, 99), 11);
    // On: each 6 px, the row closed up; the one with no spacing known stays the size set; the sparser one is 12.
    let fill = Look { fill: 12.0, ..base };
    let p = shot(&mut gpu, &measured, &fill, &cam);
    assert_eq!(run(&p, 99), 66, "the row, closed up");
    assert_eq!((90..110).filter(|x| on(&p, *x, 69)).count(), 1, "no spacing known: the size set");
    assert_eq!((85..115).filter(|x| on(&p, *x, 130)).count(), 12, "twice the spacing: twice as big");
    // Never over the most: at 5, the row's points are 5 and the sparse one too.
    let p = shot(&mut gpu, &measured, &Look { fill: 5.0, ..base }, &cam);
    assert_eq!(run(&p, 99), 55);
    assert_eq!((85..115).filter(|x| on(&p, *x, 130)).count(), 5);
    // Never under the size set: zoomed out to 3 units a pixel the spacing is 2 px, the size set of 4 stays.
    let far = Camera { units_per_pixel: 3.0, ..cam };
    let p = shot(&mut gpu, &measured, &Look { px: 4.0, fill: 12.0, ..base }, &far);
    assert_eq!((85..115).filter(|x| on(&p, *x, 90)).count(), 4, "the lone point up the screen");
    // Closer (half a unit a pixel): 12 px of spacing, 12 px points.
    let near_by = Camera { units_per_pixel: 0.5, ..cam };
    let p = shot(&mut gpu, &measured, &fill, &near_by);
    assert_eq!((95..107).filter(|x| on(&p, *x, 99)).count(), 12);
    // Not measured (its points read with Fill gaps off): the size set, whatever's asked.
    let plain = gpu.upload(&Points { centre: [0.0; 3], radius: 100.0, colours: true, shaded: false, shade_ms: 0.0, gap: 0.0, data });
    let p = shot(&mut gpu, &plain, &fill, &cam);
    assert_eq!(run(&p, 99), 11);
}

#[test]
fn a_frame_draws_two_views_each_in_its_own_part_of_the_window() {
    let Some(mut gpu) = card() else { return };
    // The floor with a step (its east half 10 higher), grey; drawn in the window's left 190 px from overhead, and in its right 200 from the south, 40° down.
    let step = gpu.upload(&ground(|x, _| if x >= 0.0 { 10.0 } else { 0.0 }));
    let left = Camera { rect: [0.0, 0.0, 190.0, 200.0], ..plan(200.0) };
    let (s, c) = (40f64.to_radians().sin(), 40f64.to_radians().cos());
    let right = Camera { target: [0.0, 0.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, c, s], to_eye: [0.0, -s, c], units_per_pixel: 1.0, rect: [200.0, 0.0, 200.0, 200.0] };
    let small = Look { px: 2.0, ..Look::own() };
    let red = Look { by: BY_ONE, one: [1.0, 0.0, 0.0], px: 2.0, ..Look::own() };
    let edl = Edl { strength: 1.0, radius: 1.4, eye: 200.0 * 0.055, noise: 0.0 };
    let none = Edl::default();
    let (shading, clip) = (Shading::default(), Clip::default());
    let boxed = Clip { boxes: vec![limit([-1e6, -30.0, -1e6], [1e6, 30.0, 1e6], 0.0, false)], slice: None, key: String::new() }.keyed();
    let left_clouds = [(&step, IDENTITY, &small)];
    let right_clouds = [(&step, IDENTITY, &red)];
    let bg = [0.2, 0.4, 0.6, 1.0];
    let views = [
        ViewDraw { slot: 0, camera: &left, clouds: &left_clouds, edl: &edl, shading: &shading, clip: &clip, clear: CLEAR, brush: None, strokes: &NO_STROKES, lines: None },
        ViewDraw { slot: 1, camera: &right, clouds: &right_clouds, edl: &none, shading: &shading, clip: &boxed, clear: [0.0, 0.0, 0.0, 1.0], brush: None, strokes: &NO_STROKES, lines: None },
    ];
    let p = gpu.picture_views([400, 200], &views, bg).expect("a picture");
    let at = |x: u32, y: u32| pixel(&p, 400, x, y);
    // The left view: the same scan in its own colours, its own eye-dome lighting (the step's foot dark), nothing clipped.
    assert!(near(at(60, 100), [200, 200, 200], 1) && near(at(60, 20), [200, 200, 200], 1));
    assert!((93..97).any(|x| at(x, 100)[0] < 150), "the step's edge, lit by the left view's own lighting");
    // Between the views (the split's bar): the frame's own background.
    assert!(near(at(194, 100), [51, 102, 153], 1));
    // The right view: the same scan in one colour, from the side, by its own limit box (a band 60 across), its own background beyond; no lighting.
    assert!(near(at(300, 100), [255, 0, 0], 1), "{:?}", at(300, 100));
    assert!(near(at(300, 10), [0, 0, 0], 1) && near(at(300, 190), [0, 0, 0], 1), "past its box: its own background");
    assert!((250..399).all(|x| { let v = at(x, 100); near(v, [255, 0, 0], 1) || near(v, [0, 0, 0], 1) }), "no lighting in the right view");
    // The left view's ground ends 15 px short of its right edge: its last pixels as the rest of it, then its own background to the edge.
    assert!(near(at(172, 150), [200, 200, 200], 1), "{:?}", at(172, 150));
    assert!(near(at(188, 150), clear_bytes(), 1), "{:?}", at(188, 150));
    // One view alone, in the second place: the first's part is the frame's background.
    let p = gpu.picture_views([400, 200], &views[1..], bg).expect("a picture");
    assert!(near(pixel(&p, 400, 60, 100), [51, 102, 153], 1) && near(pixel(&p, 400, 300, 100), [255, 0, 0], 1));
}

#[test]
fn the_card_tests_as_many_limit_boxes_as_the_engine_takes() {
    let Some(mut gpu) = card() else { return };
    assert_eq!(CLIP_BOXES, 64);
    let far = [6_000_000.0, 2_000_000.0, 100.0];
    let mut floor = ground(|_, _| 0.0);
    floor.centre = far;
    let cloud = gpu.upload(&floor);
    let cam = Camera { target: far, ..plan(200.0) };
    let look = Look { px: 1.0, ..Look::own() };
    let on = |p: &[u8], x: u32, y: u32| pixel(p, 200, x, y) == [200, 200, 200];
    // 64 boxes, each a 6 × 6 square, eight rows of eight 16 apart: every one shows its own square, nothing between them.
    let mut boxes = Vec::new();
    for k in 0..CLIP_BOXES {
        let (x, y) = (-60.0 + (k % 8) as f64 * 16.0, -60.0 + (k / 8) as f64 * 16.0);
        boxes.push(limit([far[0] + x, far[1] + y, -1e6], [far[0] + x + 6.0, far[1] + y + 6.0, 1e6], 0.0, false));
    }
    let clip = Clip { boxes: boxes.clone(), slice: None, key: String::new() }.keyed();
    let p = gpu.picture([200, 200], Some(&cam), &[(&cloud, IDENTITY, &look)], &Edl::default(), &Shading::default(), &clip, CLEAR).expect("a picture");
    for k in 0..CLIP_BOXES {
        // A square's middle on screen: east of the view's middle, and north is up.
        let (x, y) = (100 + (-60 + (k % 8) as i32 * 16 + 3), 100 - (-60 + (k / 8) as i32 * 16 + 3));
        assert!(on(&p, x as u32, y as u32), "box {k} shows its square");
        assert!(!on(&p, x as u32 + 8, y as u32) && !on(&p, x as u32, y as u32 + 8), "nothing between box {k} and the next");
    }
    let shown = (0..200).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|(x, y)| on(&p, *x, *y)).count();
    assert_eq!(shown, 64 * 36);
    // The last of them one that shows what's outside it, over the first's square: that square's gone, the other 62 stay.
    let mut holed = boxes.clone();
    holed[63] = limit([far[0] - 61.0, far[1] - 61.0, -1e6], [far[0] - 53.0, far[1] - 53.0, 1e6], 0.0, true);
    let clip = Clip { boxes: holed, slice: None, key: String::new() }.keyed();
    let p = gpu.picture([200, 200], Some(&cam), &[(&cloud, IDENTITY, &look)], &Edl::default(), &Shading::default(), &clip, CLEAR).expect("a picture");
    let shown = (0..200).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|(x, y)| on(&p, *x, *y)).count();
    assert_eq!(shown, 62 * 36);
}

/// One view of a floor with the brush's preview and strokes.
fn brushed(gpu: &mut Gpu, cam: &Camera, clouds: &[(&cadp_engine_view::Cloud, cadp_engine_view::Affine, &Look)], brush: Option<&Preview>, strokes: &Strokes, size: [u32; 2]) -> Vec<u8> {
    let (edl, shading, clip) = (Edl::default(), Shading::default(), Clip::default());
    gpu.picture_views(size, &[ViewDraw { slot: 0, camera: cam, clouds, edl: &edl, shading: &shading, clip: &clip, clear: CLEAR, brush, strokes, lines: None }], CLEAR).expect("a picture")
}

#[test]
fn the_brush_s_preview_tints_what_it_would_paint_and_shows_its_dabs_painted() {
    let Some(mut gpu) = card() else { return };
    // A floor far from 0 with a step (its east half 10 higher), in a drawing area that starts 100 px into a window 300 wide.
    let far = [6_000_000.0, 2_000_000.0, 100.0];
    let mut floor = ground(|x, _| if x >= 0.0 { 10.0 } else { 0.0 });
    floor.centre = far;
    let cloud = gpu.upload(&floor);
    let cam = Camera { target: far, rect: [100.0, 0.0, 200.0, 200.0], ..plan(200.0) };
    let look = Look { px: 1.0, ..Look::own() };
    let clouds = [(&cloud, IDENTITY, &look)];
    let grey = [200u8, 200, 200];
    // The brush's colour mixed into a point's, 45% of the way.
    let tinted = [((200.0 * 0.55 + 255.0 * 0.45) as f64).round() as u8, (200.0f64 * 0.55).round() as u8, (200.0f64 * 0.55).round() as u8];
    // Hovering 40 px left of the drawing area's middle (the window's 160, 100), 12 px from its middle out, in red.
    let hover = Preview { at: [160.0, 100.0], r: 12.0, painting: false, color: [1.0, 0.0, 0.0], hide: false, cut: None, dabs: Vec::new() };
    let p = brushed(&mut gpu, &cam, &clouds, Some(&hover), &NO_STROKES, [300, 200]);
    let at = |p: &[u8], x: u32, y: u32| pixel(p, 300, x, y);
    assert!(near(at(&p, 160, 100), tinted, 1), "{:?} for {tinted:?}", at(&p, 160, 100));
    assert!(near(at(&p, 167, 107), tinted, 1) && near(at(&p, 150, 100), tinted, 1), "inside its circle");
    assert_eq!(at(&p, 174, 100), grey, "outside it");
    assert_eq!(at(&p, 160, 114), grey);
    // Every pixel tinted is within a pixel of its circle, and it's a disc of about its size.
    let mut count = 0;
    for y in 0..200u32 {
        for x in 100..300u32 {
            if near(at(&p, x, y), tinted, 1) {
                let d = ((x as f64 + 0.5 - 160.0).powi(2) + (y as f64 + 0.5 - 100.0).powi(2)).sqrt();
                assert!(d <= 13.0, "tinted {d} px from the brush's middle");
                count += 1;
            }
        }
    }
    assert!((count as f64 - std::f64::consts::PI * 144.0).abs() < 40.0, "{count} pixels tinted");
    // No brush: as it was.
    let p = brushed(&mut gpu, &cam, &clouds, None, &NO_STROKES, [300, 200]);
    assert_eq!(at(&p, 160, 100), grey);
    // Painting: under it, and under its latest dabs, in its colour; the dabs before its last sixteen aren't shown.
    let mut dabs: Vec<[f64; 2]> = vec![[120.0, 30.0], [130.0, 30.0]];
    dabs.extend((0..16).map(|k| [140.0 + k as f64 * 6.0, 160.0]));
    let paint = Preview { painting: true, dabs: dabs.clone(), ..hover.clone() };
    let p = brushed(&mut gpu, &cam, &clouds, Some(&paint), &NO_STROKES, [300, 200]);
    assert_eq!(at(&p, 160, 100), [255, 0, 0], "under the brush");
    assert_eq!(at(&p, 140, 160), [255, 0, 0], "under its sixteenth dab back");
    assert_eq!(at(&p, 230, 165), [255, 0, 0], "under its last");
    assert_eq!(at(&p, 125, 30), grey, "the dabs before those: as the points are");
    assert_eq!(at(&p, 200, 100), grey);
    // Painting takes the points out: gone at once.
    let hide = Preview { hide: true, ..paint.clone() };
    let p = brushed(&mut gpu, &cam, &clouds, Some(&hide), &NO_STROKES, [300, 200]);
    assert!(near(at(&p, 160, 100), clear_bytes(), 1) && near(at(&p, 230, 165), clear_bytes(), 1));
    assert_eq!(at(&p, 200, 100), grey);
    // Front surface only: over the step's edge with its cut 5 over the low ground (the drawing's z 105), only the high side is the brush's.
    let front = Preview { at: [200.0, 100.0], cut: Some(far[2] + 5.0), ..hover.clone() };
    let p = brushed(&mut gpu, &cam, &clouds, Some(&front), &NO_STROKES, [300, 200]);
    assert!(near(at(&p, 205, 100), tinted, 1), "the high side");
    assert_eq!(at(&p, 195, 100), grey, "the low side, behind its cut");
}

#[test]
fn a_stroke_shows_on_the_card_as_the_page_would_paint_it() {
    let Some(mut gpu) = card() else { return };
    // A floor far from 0, coloured by class: class 2 green, class 6 red, class 9 (its layer off) not drawn.
    let far = [6_000_000.0, 2_000_000.0, 100.0];
    let mut floor = ground(|x, _| x * 0.1);
    floor.centre = far;
    let cloud = gpu.upload(&floor);
    let cam = Camera { target: far, ..plan(200.0) };
    let mut look = Look { by: BY_CLASS, px: 1.0, ..Look::own() };
    look.classes[2] = [0, 200, 0, 255];
    look.classes[6] = [200, 0, 0, 255];
    look.classes[9] = [0, 0, 200, 0];
    let clouds = [(&cloud, IDENTITY, &look)];
    let (green, red) = ([0u8, 200, 0], [200u8, 0, 0]);
    let at = |p: &[u8], x: u32, y: u32| pixel(p, 200, x, y);
    let gone = |p: &[u8], x: u32, y: u32| near(pixel(p, 200, x, y), clear_bytes(), 1);
    let plan_way = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let stroke = |op: StrokeOp, dabs: Vec<f64>| Stroke { op, right: plan_way.0, up: plan_way.1, dabs, near: None, off: Vec::new() };
    // A dab's place across and up the plan view is the drawing's own x and y.
    let dab = |x: f64, y: f64, r: f64| vec![far[0] + x, far[1] + y, r];
    let shot = |gpu: &mut Gpu, list: Vec<Stroke>, key: &str| brushed(gpu, &cam, &clouds, None, &Strokes::new(key.into(), list), [200, 200]);
    // None: all green.
    let p = shot(&mut gpu, Vec::new(), "");
    assert_eq!(at(&p, 100, 100), green);
    // A class painted 30 east of the middle, 10 across its middle out: red inside its circle, green outside.
    let p = shot(&mut gpu, vec![stroke(StrokeOp::Class(6), dab(30.0, 0.0, 10.0))], "class");
    assert_eq!(at(&p, 130, 100), red);
    assert_eq!(at(&p, 137, 93), red);
    assert_eq!(at(&p, 141, 100), green);
    assert_eq!(at(&p, 100, 100), green);
    // Every pixel on the card is as the page's own test of that point says (but within a hair of a dab's edge).
    let long: Vec<f64> = (0..40).flat_map(|k| dab(-70.0 + k as f64 * 3.5, 30.0 * ((k as f64) * 0.4).sin(), 6.0)).collect();
    let list = vec![stroke(StrokeOp::Class(6), long.clone())];
    let packed = pack(&list);
    let p = shot(&mut gpu, list, "long");
    let mut reds = 0;
    for y in 0..200u32 {
        for x in 0..200u32 {
            // The point drawn at this pixel: half a unit into it, north up.
            let (px, py) = (x as f64 - 100.0 + 0.5, 100.0 - y as f64 - 0.5);
            if px.abs() >= 80.0 || py.abs() >= 80.0 {
                continue;
            }
            let nearest = (0..40).map(|k| ((px + far[0] - long[3 * k]).powi(2) + (py + far[1] - long[3 * k + 1]).powi(2)).sqrt()).fold(f64::INFINITY, f64::min);
            if (nearest - 6.0).abs() < 0.01 {
                continue;
            }
            let want = if packed.leaves([far[0] + px, far[1] + py, far[2] + px * 0.1], 2) == Some(6) { red } else { green };
            assert_eq!(at(&p, x, y), want, "at {x}, {y}");
            reds += (want == red) as u32;
        }
    }
    assert!(reds > 2000, "{reds}");
    // A class whose layer is off: painted into it, the points are gone.
    let p = shot(&mut gpu, vec![stroke(StrokeOp::Class(9), dab(30.0, 0.0, 10.0))], "off");
    assert!(gone(&p, 130, 100));
    assert_eq!(at(&p, 100, 100), green);
    // Hidden: gone inside. Kept only: gone outside.
    let p = shot(&mut gpu, vec![stroke(StrokeOp::Out, dab(30.0, 0.0, 10.0))], "out");
    assert!(gone(&p, 130, 100) && gone(&p, 124, 106));
    assert_eq!(at(&p, 141, 100), green);
    let p = shot(&mut gpu, vec![stroke(StrokeOp::Keep, dab(30.0, 0.0, 10.0))], "keep");
    assert_eq!(at(&p, 130, 100), green);
    assert!(gone(&p, 141, 100) && gone(&p, 100, 100) && gone(&p, 30, 170));
    // A stroke leaves a class alone: nothing of class 2 hidden.
    let p = shot(&mut gpu, vec![Stroke { off: vec![2], ..stroke(StrokeOp::Out, dab(30.0, 0.0, 10.0)) }], "left");
    assert_eq!(at(&p, 130, 100), green);
    // Several at once: a class given, then that class's points hidden where another stroke crosses it, the rest kept.
    let p = shot(&mut gpu, vec![stroke(StrokeOp::Class(6), dab(0.0, 0.0, 20.0)), Stroke { off: vec![2], ..stroke(StrokeOp::Out, dab(15.0, 0.0, 10.0)) }], "two");
    assert_eq!(at(&p, 95, 100), red);
    assert!(gone(&p, 112, 100), "the class's points under the second stroke");
    assert_eq!(at(&p, 123, 100), green, "the second stroke leaves the ground");
    // Front surface only: a dab over the middle whose cut is the drawing's z 100 (the floor rises to the east): only its east half.
    let p = shot(&mut gpu, vec![Stroke { near: Some(vec![far[2]]), ..stroke(StrokeOp::Class(6), dab(0.0, 0.0, 10.0)) }], "front");
    assert_eq!(at(&p, 105, 100), red);
    assert_eq!(at(&p, 95, 100), green);
    // The scan moved in the drawing (a quarter turn about its middle, and 30 north): the stroke is the drawing's, not the scan's.
    let quarter = [0.0, -1.0, 0.0, far[0] + far[1], 1.0, 0.0, 0.0, far[1] - far[0] + 30.0, 0.0, 0.0, 1.0, 0.0];
    let moved = [(&cloud, quarter, &look)];
    let p = brushed(&mut gpu, &cam, &moved, None, &Strokes::new("moved".into(), vec![stroke(StrokeOp::Class(6), dab(30.0, 0.0, 10.0))]), [200, 200]);
    assert_eq!(at(&p, 130, 100), red);
    assert_eq!(at(&p, 141, 100), green);
    assert_eq!(at(&p, 100, 100), green);
    // From the side (looking north, 40° down): a stroke painted in plan is still a column through the plan.
    let (s, c) = (40f64.to_radians().sin(), 40f64.to_radians().cos());
    let side = Camera { target: far, right: [1.0, 0.0, 0.0], up: [0.0, c, s], to_eye: [0.0, -s, c], units_per_pixel: 1.0, rect: [0.0, 0.0, 200.0, 200.0] };
    let p = brushed(&mut gpu, &side, &clouds, None, &Strokes::new("side".into(), vec![stroke(StrokeOp::Class(6), dab(30.0, 0.0, 10.0))]), [200, 200]);
    // The point 30 east of the middle is at the screen's 130 across; its height 3 puts it 3 × cos 40° up.
    assert_eq!(at(&p, 130, 98), red);
    assert_eq!(at(&p, 100, 100), green);
    // And one painted across that side view: a circle on that screen, through the view.
    let across = Stroke { op: StrokeOp::Out, right: side.right, up: side.up, dabs: vec![far[0] + 30.0, far[1] * c + far[2] * s, 10.0], near: None, off: Vec::new() };
    let p = brushed(&mut gpu, &side, &clouds, None, &Strokes::new("across".into(), vec![across]), [200, 200]);
    assert!(gone(&p, 130, 100) && gone(&p, 136, 94));
    assert_eq!(at(&p, 142, 100), green);
    assert_eq!(at(&p, 130, 112), green);
}
