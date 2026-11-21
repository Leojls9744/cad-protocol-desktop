//! A scan's even sample for the card, from its index: a floor written
//! here, indexed, sampled whole and thinned.

use cadp_engine_core::{build_or_open, Progress};
use cadp_engine_view::{sample, wanted, Borrow, BorrowPoints, Camera, Keep, Sight, IDENTITY};
use std::sync::Arc;
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
    let (all, taken) = sample(&ix, 10_000_000, &Keep::all(), &IDENTITY, None);
    assert_eq!(all.data.len(), n);
    assert_eq!(taken.share, 1.0);
    assert_eq!(taken.of as usize, n);
    // From the scan's middle, as f32 near 0: within its half size.
    assert!((all.centre[0] - 6_000_019.95).abs() < 0.01 && (all.centre[1] - 2_000_014.95).abs() < 0.01, "{:?}", all.centre);
    assert!(all.data.iter().all(|v| v.pos[0].abs() <= 20.0 && v.pos[1].abs() <= 15.0 && v.pos[2].abs() <= 1.6));
    assert!(all.radius > 24.9 && all.radius < 25.1, "{}", all.radius);
    // Its own colours, its class and its intensity, for the card to colour by.
    assert!(all.colours);
    assert!(all.data.iter().all(|v| v.rgb == [200, 100, 50] && v.class == 2 && v.intensity == 9));
    // Unshaded till asked: each point lit as level ground, on its surface, seeing all the sky.
    assert!(!all.shaded && all.data.iter().all(|v| v.shade == cadp_engine_view::surface::plain()));
    // Asked for (`shade`): worked out from the points' own shape. The floor's a plane, its slope too
    // slight to tell from level by its light; nothing on it stands off it or hides the sky.
    let shade = Keep { shade: true, ..Keep::all() }.keyed();
    assert_ne!(shade.key, Keep::all().keyed().key);
    let (lit, _) = sample(&ix, 10_000_000, &shade, &IDENTITY, None);
    assert!(lit.shaded && lit.data.len() == n);
    let level = cadp_engine_view::surface::plain()[0] as i32;
    let range = |k: usize| (lit.data.iter().map(|v| if k == 1 { v.shade[1] as i8 as i32 } else { v.shade[k] as i32 }).min().unwrap(), lit.data.iter().map(|v| if k == 1 { v.shade[1] as i8 as i32 } else { v.shade[k] as i32 }).max().unwrap());
    // Its light a little over level ground's (it tips toward the light's side, 4°); on its surface; next to all the sky (the rise up the slope hides a sliver).
    let (light, relief, sky) = (range(0), range(1), range(3));
    // (But for a patch at the floor's rim that holds one row of this floor's even grid: a line has no one way to face.)
    let lit_right = lit.data.iter().filter(|v| (v.shade[0] as i32) >= level && (v.shade[0] as i32) <= level + 8).count();
    assert!(lit_right * 100 >= n * 97 && light.1 <= level + 8, "a plane's light: {light:?} about {level}, {lit_right} of {n}");
    assert!(relief.0 >= -8 && relief.1 <= 8, "a plane's relief: {relief:?}");
    assert!(sky.0 >= 240, "a plane's sky: {sky:?}");
    // A tenth: about a tenth, from every part of the floor (each cell's first share).
    let (some, taken) = sample(&ix, n / 10, &Keep::all(), &IDENTITY, None);
    assert!(some.data.len() >= n / 10 && some.data.len() <= n / 10 + ix.cells * 2, "{} of {n}, {} cells", some.data.len(), ix.cells);
    assert!((taken.share - 0.1).abs() < 1e-9);
    for (lo, hi) in [(-20.0, -10.0), (-10.0, 0.0), (0.0, 10.0), (10.0, 20.0)] {
        let inside = some.data.iter().filter(|v| v.pos[0] >= lo && v.pos[0] < hi).count();
        let share = inside as f64 / some.data.len() as f64;
        assert!(share > 0.2 && share < 0.3, "{share} between {lo} and {hi}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scan_with_no_colours_says_so() {
    let dir = scratch("heights");
    let src = dir.join("plain.las");
    floor(&src, false);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    assert!(!ix.has_rgb);
    let (all, _) = sample(&ix, 10_000_000, &Keep::all(), &IDENTITY, None);
    assert!(!all.colours);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn classes_are_read_as_the_scans_map_has_them() {
    let dir = scratch("classes");
    let src = dir.join("mixed.las");
    // The floor, a point in four made class 7.
    let n = floor_with(&src, true, |i, j| if (i + j) % 4 == 0 { 7 } else { 2 });
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    let (all, _) = sample(&ix, 10_000_000, &Keep::all(), &IDENTITY, None);
    assert_eq!(all.data.iter().filter(|v| v.class == 7).count(), n / 4);
    // The file's 7 read as 18 (the scan's class map): every one of them, the rest as they were.
    let mut map = [0u8; 256];
    for (k, m) in map.iter_mut().enumerate() {
        *m = k as u8;
    }
    map[7] = 18;
    let keep = Keep { map: Some(Arc::new(map)), ..Keep::all() }.keyed();
    let (mapped, _) = sample(&ix, 10_000_000, &keep, &IDENTITY, None);
    assert_eq!(mapped.data.iter().filter(|v| v.class == 18).count(), n / 4);
    assert_eq!(mapped.data.iter().filter(|v| v.class == 2).count(), n - n / 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_classified_scans_points_take_the_pages_classes_and_its_removed_points_are_left_out() {
    let dir = scratch("borrow");
    let src = dir.join("floor.las");
    let n = floor(&src, true);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    // The page's own points: every fifth each way (a point each half unit), in a drawing where the scan's been moved 1000 east.
    // West of x = 6,001,010 (the file's 6,000,010) it classed them 6; a strip 5 to 7 north it took out.
    let moved = [1.0, 0.0, 0.0, 1000.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let origin = [6_001_000.0, 2_000_000.0, 100.0];
    let mut xyz = Vec::new();
    let mut class = Vec::new();
    let mut removed = Vec::new();
    for i in (0..400).step_by(5) {
        for j in (0..300).step_by(5) {
            let (x, y) = (i as f32 * 0.1, j as f32 * 0.1);
            xyz.extend_from_slice(&[x, y, i as f32 * 0.0075]);
            class.push(if x < 10.0 { 6 } else { 9 });
            removed.push(u8::from((5.0..7.0).contains(&y)));
        }
    }
    let points = Arc::new(BorrowPoints::new("its points".into(), 1.0, origin, xyz));
    let borrow = Arc::new(Borrow::new("its classes".into(), points, class, Some(removed)).unwrap());
    let keep = Keep { borrow: Some(borrow), ..Keep::all() }.keyed();
    let (got, _) = sample(&ix, 10_000_000, &keep, &moved, None);
    // The strip's points are gone (20 of 300 rows, give or take the cells' edges).
    let gone = n - got.data.len();
    assert!((gone as f64 / n as f64 - 20.0 / 300.0).abs() < 0.01, "{gone} of {n}");
    assert!(got.data.iter().all(|v| !(5.05..6.95).contains(&(v.pos[1] + 14.95))));
    // West of 10 units in: the page's 6; east: its 9; none the file's 2.
    let west = got.data.iter().filter(|v| v.pos[0] + 19.95 < 9.5);
    let east = got.data.iter().filter(|v| v.pos[0] + 19.95 > 10.5);
    assert!(west.clone().count() > 1000 && west.clone().all(|v| v.class == 6));
    assert!(east.clone().count() > 1000 && east.clone().all(|v| v.class == 9));
    // Told from the scan with the file's classes, and from other classes of the page's.
    assert_ne!(keep.key, Keep::all().keyed().key);
    // Where the drawing has the scan somewhere else, none of the page's points are near: the file's own classes, nothing left out.
    let (far, _) = sample(&ix, 10_000_000, &keep, &IDENTITY, None);
    assert_eq!(far.data.len(), n);
    assert!(far.data.iter().all(|v| v.class == 2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_view_takes_the_cells_it_sees_and_more_of_each() {
    let dir = scratch("view");
    let src = dir.join("floor.las");
    let n = floor(&src, true);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    assert!(ix.cells > 8, "{} cells", ix.cells);
    // Plan view of the floor's south-west corner: 8 × 6 units of its 40 × 30, in a drawing area 800 × 600 px.
    let cam = Camera {
        target: [6_000_004.0, 2_000_003.0, 100.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        to_eye: [0.0, 0.0, 1.0],
        units_per_pixel: 0.01,
        rect: [100.0, 50.0, 800.0, 600.0],
    };
    let sight = Sight::new(&cam, &IDENTITY);
    // A budget of a fifth of the scan: of the whole scan, a fifth of each cell.
    let (whole, all) = sample(&ix, n / 5, &Keep::all(), &IDENTITY, None);
    assert!((all.share - 0.2).abs() < 1e-9);
    // For the view: only the cells it sees, every point of them (they're under the budget).
    let (seen, taken) = sample(&ix, n / 5, &Keep::all(), &IDENTITY, Some(&sight));
    assert_eq!(taken.share, 1.0);
    assert!(taken.of < ix.count / 3, "{} of {}", taken.of, ix.count);
    assert_eq!(seen.data.len() as u64, taken.of);
    // Every point in the drawing area is among them: 80 × 60 points a unit... a point every 0.1, so 80 × 60 of them.
    let inside = |v: &cadp_engine_view::PointVertex| v.pos[0] + 19.95 < 8.0 && v.pos[1] + 14.95 < 6.0;
    assert_eq!(seen.data.iter().filter(|v| inside(v)).count(), 80 * 60);
    // Far more of them there than the whole scan's sample has.
    assert!(whole.data.iter().filter(|v| inside(v)).count() < 80 * 60 / 3);
    // Its sample covers the view, a smaller one inside it and a notch closer; not a pan to the other end, nor the whole scan.
    assert!(taken.covers(&wanted(&ix, n / 5, Some(&sight))));
    let closer = Camera { units_per_pixel: 0.005, ..cam };
    assert!(taken.covers(&wanted(&ix, n / 5, Some(&Sight::new(&closer, &IDENTITY)))));
    let away = Camera { target: [6_000_036.0, 2_000_027.0, 100.0], ..cam };
    assert!(!taken.covers(&wanted(&ix, n / 5, Some(&Sight::new(&away, &IDENTITY)))));
    assert!(!taken.covers(&all));
    // The whole scan's sample covers neither view at the share the view wants (a fifth isn't every point).
    assert!(!all.covers(&wanted(&ix, n / 5, Some(&sight))));
    let _ = std::fs::remove_dir_all(&dir);
}
