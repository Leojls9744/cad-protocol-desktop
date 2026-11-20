//! A frame drawn on the card into a picture and read back: the check's ring
//! where the view puts it, and a scan's sample from its index.

use cadp_engine_view::{test_ring, Camera, Gpu, CLEAR, IDENTITY};

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
    let picture = gpu.picture([400, 300], Some(&cam), &[(&ring, IDENTITY)], 3.0, CLEAR).expect("a picture");
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
        v.color = [255, 0, 0, 255];
    }
    let high = gpu.upload(&high_points);
    let cam = Camera {
        target: [0.0, 0.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        to_eye: [0.0, 0.0, 1.0],
        units_per_pixel: 1.0,
        rect: [0.0, 0.0, 200.0, 200.0],
    };
    // Whichever is drawn first, the higher (nearer the eye, from overhead) shows.
    for order in [[&low, &high], [&high, &low]] {
        let picture = gpu.picture([200, 200], Some(&cam), &[(order[0], IDENTITY), (order[1], IDENTITY)], 4.0, CLEAR).expect("a picture");
        assert!(near(pixel(&picture, 200, 100, 100), [255, 0, 0], 2), "{:?}", pixel(&picture, 200, 100, 100));
    }
    let empty = gpu.picture([64, 64], None, &[(&low, IDENTITY)], 4.0, CLEAR).expect("a picture");
    assert!(near(pixel(&empty, 64, 32, 32), clear_bytes(), 2));
}
