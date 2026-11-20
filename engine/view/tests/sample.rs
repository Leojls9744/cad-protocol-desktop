//! A scan's even sample for the card, from its index: a floor written
//! here, indexed, sampled whole and thinned.

use cadp_engine_core::{build_or_open, Progress};
use cadp_engine_view::{sample, Keep};
use las::{point::Classification, Builder, Point, Writer};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cadp-view-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A floor 40 × 30 at z 100 to 103 (a slope), a point every 0.1, far from 0; coloured or not.
fn floor(path: &std::path::Path, coloured: bool) -> usize {
    floor_with(path, coloured, |_, _| 2)
}

/// The floor, each point's class by where it is.
fn floor_with(path: &std::path::Path, coloured: bool, class: impl Fn(usize, usize) -> u8) -> usize {
    let mut b = Builder::from((1, 2));
    b.point_format = las::point::Format::new(if coloured { 3 } else { 1 }).unwrap();
    b.transforms = las::Vector {
        x: las::Transform { scale: 0.001, offset: 6_000_000.0 },
        y: las::Transform { scale: 0.001, offset: 2_000_000.0 },
        z: las::Transform { scale: 0.001, offset: 0.0 },
    };
    let mut w = Writer::from_path(path, b.into_header().unwrap()).unwrap();
    let mut n = 0;
    for i in 0..400 {
        for j in 0..300 {
            let mut p = Point {
                x: 6_000_000.0 + i as f64 * 0.1,
                y: 2_000_000.0 + j as f64 * 0.1,
                z: 100.0 + i as f64 * 0.0075,
                intensity: 9,
                classification: Classification::new(class(i, j)).unwrap(),
                gps_time: Some(0.0),
                ..Default::default()
            };
            if coloured {
                p.color = Some(las::Color::new(200, 100, 50));
            }
            w.write_point(p).unwrap();
            n += 1;
        }
    }
    w.close().unwrap();
    n
}

#[test]
fn every_point_when_there_s_room_and_an_even_share_when_there_isn_t() {
    let dir = scratch("sample");
    let src = dir.join("floor.las");
    let n = floor(&src, true);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    assert_eq!(ix.count as usize, n);
    let all = sample(&ix, 10_000_000, &Keep::all());
    assert_eq!(all.data.len(), n);
    // From the scan's middle, as f32 near 0: within its half size.
    assert!((all.centre[0] - 6_000_019.95).abs() < 0.01 && (all.centre[1] - 2_000_014.95).abs() < 0.01, "{:?}", all.centre);
    assert!(all.data.iter().all(|v| v.pos[0].abs() <= 20.0 && v.pos[1].abs() <= 15.0 && v.pos[2].abs() <= 1.6));
    assert!(all.radius > 24.9 && all.radius < 25.1, "{}", all.radius);
    // Its own colours.
    assert!(all.data.iter().all(|v| v.color == [200, 100, 50, 255]));
    // A tenth: about a tenth, from every part of the floor (each cell's first share).
    let some = sample(&ix, n / 10, &Keep::all());
    assert!(some.data.len() >= n / 10 && some.data.len() <= n / 10 + ix.cells * 2, "{} of {n}, {} cells", some.data.len(), ix.cells);
    for (lo, hi) in [(-20.0, -10.0), (-10.0, 0.0), (0.0, 10.0), (10.0, 20.0)] {
        let inside = some.data.iter().filter(|v| v.pos[0] >= lo && v.pos[0] < hi).count();
        let share = inside as f64 / some.data.len() as f64;
        assert!(share > 0.2 && share < 0.3, "{share} between {lo} and {hi}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_classes_kept_are_sampled_and_about_as_many_as_asked() {
    let dir = scratch("classes");
    let src = dir.join("mixed.las");
    // The floor, a point in four made class 7 (noise).
    let n = floor_with(&src, true, |i, j| if (i + j) % 4 == 0 { 7 } else { 2 });
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    let mut no_noise = Keep::all();
    no_noise.allow[0] &= !(1u64 << 7);
    let all = sample(&ix, 10_000_000, &no_noise);
    assert_eq!(all.data.len(), n - n / 4);
    // A tenth asked for: about a tenth of the scan's points come, all kept ones (not a tenth less the quarter left out).
    let some = sample(&ix, n / 10, &no_noise);
    let got = some.data.len() as f64 / (n as f64 / 10.0);
    assert!(got > 0.9 && got < 1.15, "{got}");
    // Nothing kept: nothing comes, and the scan isn't read whole for it.
    let none = Keep { allow: [0; 4], ..Keep::all() };
    assert_eq!(sample(&ix, 1000, &none).data.len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scan_with_no_colours_is_coloured_by_its_heights() {
    let dir = scratch("heights");
    let src = dir.join("plain.las");
    floor(&src, false);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    assert!(!ix.has_rgb);
    let all = sample(&ix, 10_000_000, &Keep::all());
    let low = all.data.iter().min_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
    let high = all.data.iter().max_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
    assert_eq!(low.color, [40, 80, 220, 255]);
    assert_eq!(high.color, [225, 70, 50, 255]);
    let _ = std::fs::remove_dir_all(&dir);
}
