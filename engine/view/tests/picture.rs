//! A frame drawn on the card into a picture and read back: the check's ring
//! where the view puts it, and a scan's sample from its index.

use cadp_engine_view::{test_ring, Camera, Edl, Gpu, Look, PointVertex, Points, BY_CLASS, BY_HEIGHT, BY_INTENSITY, BY_ONE, CLEAR, IDENTITY};

/// The card, or none on a computer with no way to draw (the test then says so and passes).
fn card() -> Option<Gpu> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    match Gpu::new(&instance, None, Some(wgpu::TextureFormat::Rgba8Unorm)) {
        Ok(gpu) => {
            eprintln!("drawing on {}", gpu.adapter);
            Some(gpu)
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
    PointVertex { pos: [x, y, z], rgb, class, intensity, setup: 0 }
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
    let picture = gpu.picture([400, 300], Some(&cam), &[(&ring, IDENTITY, &look)], &Edl::default(), CLEAR).expect("a picture");
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
        let picture = gpu.picture([200, 200], Some(&cam), &[(order[0], IDENTITY, &look), (order[1], IDENTITY, &look)], &Edl::default(), CLEAR).expect("a picture");
        assert!(near(pixel(&picture, 200, 100, 100), [255, 0, 0], 2), "{:?}", pixel(&picture, 200, 100, 100));
    }
    let empty = gpu.picture([64, 64], None, &[(&low, IDENTITY, &look)], &Edl::default(), CLEAR).expect("a picture");
    assert!(near(pixel(&empty, 64, 32, 32), clear_bytes(), 2));
    // A background of the page's own.
    let own = gpu.picture([64, 64], None, &[], &Edl::default(), [0.2, 0.4, 0.6, 1.0]).expect("a picture");
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
    let cloud = gpu.upload(&Points { centre: [1000.0, 2000.0, 50.0], radius: 80.0, colours: true, data: data.clone() });
    let cam = Camera { target: [1000.0, 2000.0, 50.0], ..plan(200.0) };
    let at = |k: usize| 100 + [-60i32, -20, 20, 60][k];
    let shot = |gpu: &mut Gpu, cloud: &cadp_engine_view::Cloud, look: &Look| gpu.picture([200, 200], Some(&cam), &[(cloud, IDENTITY, look)], &Edl::default(), CLEAR).expect("a picture");
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
    let plain = gpu.upload(&Points { centre: [1000.0, 2000.0, 50.0], radius: 80.0, colours: false, data: data.clone() });
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
    let cloud = gpu.upload(&Points { centre: [0.0, 0.0, 0.0], radius: 100.0, colours: true, data });
    let cam = plan(200.0);
    let look = Look { px: 2.0, ..Look::own() };
    let shot = |gpu: &mut Gpu, edl: &Edl| gpu.picture([200, 200], Some(&cam), &[(&cloud, IDENTITY, &look)], edl, CLEAR).expect("a picture");
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
    let far = gpu.picture([200, 200], Some(&Camera { target: [500.0, 0.0, 0.0], ..cam }), &[(&cloud, IDENTITY, &look)], &edl, CLEAR).expect("a picture");
    assert!(near(pixel(&far, 200, 100, 100), clear_bytes(), 1));
}
