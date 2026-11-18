//! The engine on scans written here: a LAS and a LAZ of a floor, a wall and
//! a post, read, indexed, opened again, and asked what's under the cursor.

use cadp_engine_core::{build_or_open, region, snap, Index, Progress, RegionQuery, ScanIn, SnapQuery};
use las::{point::Classification, Builder, Point, Writer};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cadp-engine-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A floor at z 100 (class 2, 0.1 apart over 20 × 20), a wall along y = 10
/// up to z 106 (class 6), and a post at (5, 5) up to 108 (class 5).
fn site() -> Vec<(f64, f64, f64, u8)> {
    let mut pts = Vec::new();
    for i in 0..200 {
        for j in 0..200 {
            pts.push((i as f64 * 0.1, j as f64 * 0.1, 100.0, 2));
        }
    }
    for i in 0..200 {
        for k in 0..60 {
            pts.push((i as f64 * 0.1, 10.0, 100.1 + k as f64 * 0.1, 6));
        }
    }
    for k in 0..80 {
        pts.push((5.0, 5.0, 100.1 + k as f64 * 0.1, 5));
    }
    pts
}

fn write(path: &Path, laz: bool) {
    let mut b = Builder::from((1, 2));
    b.point_format = las::point::Format::new(if laz { 1 } else { 3 }).unwrap();
    b.point_format.is_compressed = laz;
    b.transforms = las::Vector {
        x: las::Transform { scale: 0.001, offset: 1000.0 },
        y: las::Transform { scale: 0.001, offset: 2000.0 },
        z: las::Transform { scale: 0.001, offset: 0.0 },
    };
    let header = b.into_header().unwrap();
    let mut w = Writer::from_path(path, header).unwrap();
    for (x, y, z, c) in site() {
        let mut p = Point {
            x: 1000.0 + x,
            y: 2000.0 + y,
            z,
            intensity: 7,
            classification: Classification::new(c).unwrap(),
            ..Default::default()
        };
        if !laz {
            p.color = Some(las::Color::new(1, 2, 3));
            p.gps_time = Some(0.0);
        } else {
            p.gps_time = Some(0.0);
        }
        w.write_point(p).unwrap();
    }
    w.close().unwrap();
}

const IDENTITY: [f64; 12] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.];
const ALL: [u64; 4] = [u64::MAX; 4];

/// Looking straight down at (x, y), 0.01 units a pixel, a 400 × 300 view.
fn plan(x: f64, y: f64) -> SnapQuery {
    SnapQuery {
        target: [x, y, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        to_eye: [0.0, 0.0, 1.0],
        units_per_pixel: 0.01,
        half_w: 200.0,
        half_h: 150.0,
        cursor: [200.0, 150.0],
        reach: 8.0,
        front_px: 2.0,
        window: 0.0,
        below: None,
        band: None,
        most: 64,
        front_most: 16,
    }
}

/// Looking north, level, at (x, 0, z).
fn side(x: f64, z: f64) -> SnapQuery {
    SnapQuery {
        target: [x, 0.0, z],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        to_eye: [0.0, -1.0, 0.0],
        units_per_pixel: 0.01,
        half_w: 200.0,
        half_h: 150.0,
        cursor: [200.0, 150.0],
        reach: 8.0,
        front_px: 2.0,
        window: 0.5,
        below: None,
        band: None,
        most: 64,
        front_most: 16,
    }
}

#[test]
fn reads_indexes_and_opens_again() {
    let dir = scratch("open");
    let las = dir.join("site.las");
    write(&las, false);
    let cache = dir.join("cache");
    let progress = Progress::default();
    let ix = build_or_open(&las, &cache, &progress).unwrap();
    assert!(ix.built);
    assert_eq!(ix.count as usize, site().len());
    assert!(ix.has_rgb);
    assert!(progress.get() >= 1.0);
    // Every point once, each in a cell whose box holds it.
    let mut seen = 0u64;
    for c in 0..ix.cells {
        let cell = ix.cell(c);
        for i in cell.start..cell.start + cell.count as u64 {
            let p = ix.point(i);
            assert!(p.x >= cell.min[0] && p.x <= cell.max[0]);
            assert!(p.z >= cell.min[2] && p.z <= cell.max[2]);
            assert_eq!(p.intensity, 7);
            seen += 1;
        }
    }
    assert_eq!(seen, ix.count);
    assert!(ix.blocks >= 1 && ix.blocks <= ix.cells);
    drop(ix);
    // Again: the index made before.
    let again = build_or_open(&las, &cache, &Progress::default()).unwrap();
    assert!(!again.built);
    assert_eq!(again.count as usize, site().len());
    // A cut-short index isn't opened.
    let f = std::fs::read_dir(&cache).unwrap().next().unwrap().unwrap().path();
    let bytes = std::fs::read(&f).unwrap();
    let short = dir.join("short.cpe");
    std::fs::write(&short, &bytes[..bytes.len() / 2]).unwrap();
    assert!(Index::open(&short).is_err());
}

#[test]
fn reads_laz() {
    let dir = scratch("laz");
    let laz = dir.join("site.laz");
    write(&laz, true);
    let ix = build_or_open(&laz, &dir.join("cache"), &Progress::default()).unwrap();
    assert_eq!(ix.count as usize, site().len());
    let a = snap(&[ScanIn { index: &ix, affine: IDENTITY, allow: ALL, setup: None }], &plan(1003.0, 2003.0));
    let best = a.near[0];
    assert!((best.p[0] - 1003.0).abs() < 1e-6 && (best.p[1] - 2003.0).abs() < 1e-6);
    assert_eq!(best.class, 2);
}

#[test]
fn finds_the_point_under_the_cursor_in_plan_and_from_the_side() {
    let dir = scratch("snap");
    let las = dir.join("site.las");
    write(&las, false);
    let ix = build_or_open(&las, &dir.join("cache"), &Progress::default()).unwrap();
    let scans = [ScanIn { index: &ix, affine: IDENTITY, allow: ALL, setup: None }];
    // Plan, between floor points: the nearest four, 0.05 off (5 px) each way.
    let mut q = plan(1003.05, 2003.05);
    let a = snap(&scans, &q);
    assert!(!a.near.is_empty());
    assert!((a.near[0].d - (50f64).sqrt()).abs() < 1e-6, "{}", a.near[0].d);
    assert!(a.near.iter().all(|c| c.d <= 8.0));
    // On the post, from above: its top first in front.
    q = plan(1005.0, 2005.0);
    let a = snap(&scans, &q);
    assert_eq!(a.front[0].class, 5);
    assert!((a.front[0].p[2] - 108.0).abs() < 1e-6);
    // The post's class not taken: the floor under it.
    let mut no_post = ALL;
    no_post[0] &= !(1 << 5);
    let a = snap(&[ScanIn { index: &ix, affine: IDENTITY, allow: no_post, setup: None }], &q);
    assert!(a.front.iter().all(|c| c.class != 5));
    assert_eq!(a.front[0].class, 2);
    // From the side, level, at the wall's face: the wall's points, the
    // floor's behind it left out by the window.
    let a = snap(&scans, &side(1004.0, 103.0));
    assert!(!a.near.is_empty());
    assert!(a.near.iter().all(|c| c.class == 6));
    assert!((a.front[0].p[1] - 2010.0).abs() < 1e-6);
    // Points nearer the eye than `below` left out: past the wall, nothing at that height.
    let mut q = side(1004.0, 103.0);
    q.below = Some(-2010.5);
    let a = snap(&scans, &q);
    assert!(a.near.iter().all(|c| c.h <= -2010.5));
    // A band: only the wall's face, as the page asks once it knows its front.
    let mut q = side(1004.0, 103.0);
    q.window = 0.0;
    q.band = Some([-2010.05, -2009.95]);
    let a = snap(&scans, &q);
    assert!(!a.near.is_empty());
    assert!(a.near.iter().all(|c| (c.p[1] - 2010.0).abs() < 1e-6));
}

#[test]
fn moves_the_file_into_the_drawing() {
    let dir = scratch("move");
    let las = dir.join("site.las");
    write(&las, false);
    let ix = build_or_open(&las, &dir.join("cache"), &Progress::default()).unwrap();
    // Feet to metres, then shifted 10 east.
    let k = 0.3048;
    let a = [k, 0., 0., 10., 0., k, 0., 0., 0., 0., k, 0.];
    let at = [1003.0 * k + 10.0, 2003.0 * k, 0.0];
    let mut q = plan(at[0], at[1]);
    q.units_per_pixel = 0.003;
    let got = snap(&[ScanIn { index: &ix, affine: a, allow: ALL, setup: None }], &q);
    let best = got.near[0];
    assert!((best.p[0] - at[0]).abs() < 1e-6 && (best.p[1] - at[1]).abs() < 1e-6, "{:?}", best.p);
    assert!((best.p[2] - 100.0 * k).abs() < 1e-6);
}

/// How fast: 20 million points (a floor and a long wall), indexed, then
/// asked of from above and from the side. `cargo test --release -- --ignored`.
#[test]
#[ignore]
fn speed() {
    let dir = scratch("speed");
    let path = dir.join("big.las");
    let mut b = Builder::from((1, 2));
    b.point_format = las::point::Format::new(1).unwrap();
    b.transforms = las::Vector {
        x: las::Transform { scale: 0.001, offset: 0.0 },
        y: las::Transform { scale: 0.001, offset: 0.0 },
        z: las::Transform { scale: 0.001, offset: 0.0 },
    };
    let mut w = Writer::from_path(&path, b.into_header().unwrap()).unwrap();
    let t = std::time::Instant::now();
    for i in 0..4000 {
        for j in 0..4000 {
            let p = Point { x: i as f64 * 0.05, y: j as f64 * 0.05, z: 100.0 + ((i * 7 + j * 3) % 11) as f64 * 0.001, gps_time: Some(0.0), classification: Classification::Ground, ..Default::default() };
            w.write_point(p).unwrap();
        }
    }
    for i in 0..4000 {
        for k in 0..1000 {
            let p = Point { x: i as f64 * 0.05, y: 100.0, z: 100.0 + k as f64 * 0.01, gps_time: Some(0.0), classification: Classification::Building, ..Default::default() };
            w.write_point(p).unwrap();
        }
    }
    w.close().unwrap();
    eprintln!("written in {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let ix = build_or_open(&path, &dir.join("cache"), &Progress::default()).unwrap();
    eprintln!("indexed {} points in {:?}: {} cells, {} blocks", ix.count, t.elapsed(), ix.cells, ix.blocks);
    let scans = [ScanIn { index: &ix, affine: IDENTITY, allow: ALL, setup: None }];
    for (name, q) in [("plan", plan(100.0, 50.0)), ("side, level, along the wall", side(100.0, 105.0))] {
        let mut q = q;
        q.units_per_pixel = 0.02;
        q.reach = 20.0;
        q.most = 600;
        let t = std::time::Instant::now();
        let n = 50;
        let mut last = None;
        for k in 0..n {
            q.cursor = [200.0 + k as f64, 150.0];
            last = Some(snap(&scans, &q));
        }
        let a = last.unwrap();
        eprintln!("{name}: {:?} a question, {} points tried in {} cells, {} near", t.elapsed() / n, a.tried, a.cells, a.near.len());
    }
}

#[test]
fn gives_a_box_s_points_evenly_thinned() {
    let dir = scratch("region");
    let las = dir.join("site.las");
    write(&las, false);
    let ix = build_or_open(&las, &dir.join("cache"), &Progress::default()).unwrap();
    // The floor's 5 × 5 corner (0.1 apart: 51 × 51 points), every one.
    let q = RegionQuery { min: [999.95, 1999.95, 99.9], max: [1005.05, 2005.05, 100.05], ring: None, most: 1_000_000, setup: None };
    let r = region(&ix, &IDENTITY, &q);
    assert!(r.complete);
    assert_eq!(r.class.len(), 51 * 51);
    assert!(r.class.iter().all(|c| *c == 2));
    assert_eq!(r.xyz.len(), r.class.len() * 3);
    // Placed about the middle of what's found.
    let o = r.origin;
    let x0 = (0..r.class.len()).map(|i| r.xyz[i * 3] as f64 + o[0]).fold(f64::MAX, f64::min);
    assert!((x0 - 1000.0).abs() < 1e-3, "{x0}");
    // 1.22: a level slice's box is endless across; its points are still placed close by.
    let band = RegionQuery { min: [-1e12, -1e12, 99.9], max: [1e12, 1e12, 100.05], ring: None, most: 10_000_000, setup: None };
    let b = region(&ix, &IDENTITY, &band);
    assert!(b.class.len() > 51 * 51, "{}", b.class.len());
    assert!(b.origin[0] > 990.0 && b.origin[0] < 1030.0 && b.origin[1] > 1990.0 && b.origin[1] < 2030.0, "{:?}", b.origin);
    assert!(b.xyz.iter().all(|v| v.abs() < 50.0));
    let back = (0..b.class.len()).map(|i| b.xyz[i * 3] as f64 + b.origin[0]).fold(f64::MAX, f64::min);
    assert!((back - 1000.0).abs() < 1e-3, "{back}");
    assert_eq!(r.rgb.as_ref().map(|v| v.len()), Some(r.class.len() * 3));
    assert_eq!(&r.rgb.as_ref().unwrap()[0..3], &[1, 2, 3]);
    // A ring: a triangle in plan, about half the square.
    let tri = RegionQuery { ring: Some(vec![[999.95, 1999.95], [1005.05, 1999.95], [999.95, 2005.05]]), ..q.clone() };
    let t = region(&ix, &IDENTITY, &tri);
    assert!(t.class.len() > 1200 && t.class.len() < 1450, "{}", t.class.len());
    // Thinned: about a tenth of the whole floor, spread over it.
    let all = RegionQuery { min: [999.0, 1999.0, 99.0], max: [1021.0, 2021.0, 110.0], ring: None, most: 4_000, setup: None };
    let a = region(&ix, &IDENTITY, &all);
    assert!(!a.complete);
    assert!(a.class.len() >= 4_000 && a.class.len() < 6_000, "{}", a.class.len());
    let xs: Vec<f64> = (0..a.class.len()).map(|i| a.xyz[i * 3] as f64 + a.origin[0]).collect();
    let east = xs.iter().filter(|x| **x > 1010.0).count() as f64 / xs.len() as f64;
    assert!(east > 0.35 && east < 0.65, "{east}");
    // The same points each time.
    let b = region(&ix, &IDENTITY, &all);
    assert_eq!(a.xyz, b.xyz);
}
