//! Delete Outside Limit Box in the engine (1.28): a scan written here,
//! indexed, cut to a box (feet into metres, moved), written as LAS and LAZ,
//! and read back.

use cadp_engine_core::build_or_open;
use cadp_engine_core::crop::{crop, CropAsk, CropBox};
use cadp_engine_core::Progress;
use las::{point::Classification, Builder, Point, Read, Reader, Writer};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cadp-engine-crop-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 100 × 100 grid of points 0.1 apart at z 10 (class 2, colour), every tenth flagged withheld, returns 2 of 3.
fn write(path: &Path) {
    let mut b = Builder::from((1, 4));
    b.point_format = las::point::Format::new(7).unwrap();
    b.transforms = las::Vector {
        x: las::Transform { scale: 0.001, offset: 5000.0 },
        y: las::Transform { scale: 0.001, offset: 7000.0 },
        z: las::Transform { scale: 0.001, offset: 0.0 },
    };
    let mut w = Writer::from_path(path, b.into_header().unwrap()).unwrap();
    for i in 0..100 {
        for j in 0..100 {
            w.write_point(Point {
                x: 5000.0 + i as f64 * 0.1,
                y: 7000.0 + j as f64 * 0.1,
                z: 10.0,
                intensity: 300,
                return_number: 2,
                number_of_returns: 3,
                classification: Classification::new(2).unwrap(),
                is_withheld: (i * 100 + j) % 10 == 0,
                color: Some(las::Color::new(10 * 257, 20 * 257, 30 * 257)),
                gps_time: Some(1.0),
                ..Default::default()
            })
            .unwrap();
        }
    }
    w.close().unwrap();
}

fn plain(min: [f64; 3], max: [f64; 3]) -> CropBox {
    CropBox { min, max, centre: [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0], cos: 1.0, sin: 0.0, turned: false, outside: false, slack: [1e-6; 3] }
}

#[test]
fn a_scan_cut_to_a_box_from_its_index_as_las_and_laz() {
    let dir = scratch("box");
    let src = dir.join("site.las");
    write(&src);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    // Into the drawing: ×2 (units), then 100 east.
    let affine = [2.0, 0., 0., 100.0, 0., 2.0, 0., 0., 0., 0., 2.0, 0.];
    // The file's x 5000.0–5004.95 and y 7000.0–7002.45: drawing x 10100–10109.9, y 14000–14004.9.
    let ask = CropAsk {
        affine,
        boxes: vec![plain([10_099.95, 13_999.95, 0.0], [10_109.95, 14_004.95, 100.0])],
        setup: None,
        scale: [0.002; 3],
        offset: [10_100.0, 14_000.0, 0.0],
        wkt: "LOCAL_CS[\"test\"]".into(),
        laz: false,
    };
    for laz in [false, true] {
        let out = dir.join(if laz { "cut.laz" } else { "cut.las" });
        let done = crop(&ix, &CropAsk { laz, ..ask.clone() }, &out).unwrap();
        assert_eq!(done.processed, 10_000);
        assert_eq!(done.kept, 50 * 25);
        assert!(done.bytes > 0);
        let mut r = Reader::from_path(&out).unwrap();
        let h = r.header().clone();
        assert_eq!(h.number_of_points(), 1250);
        assert_eq!(h.version(), las::Version::new(1, 4));
        assert_eq!(h.point_format().to_u8().unwrap(), 7);
        assert_eq!(h.point_format().is_compressed, laz);
        assert!(h.vlrs().iter().any(|v| v.record_id == 2112 && v.data.starts_with(b"LOCAL_CS")));
        // On the scan's own grid in the drawing.
        assert_eq!(h.transforms().x.offset, 10_100.0);
        assert_eq!(h.transforms().x.scale, 0.002);
        let pts: Vec<Point> = r.points().map(|p| p.unwrap()).collect();
        assert_eq!(pts.len(), 1250);
        let withheld = pts.iter().filter(|p| p.is_withheld).count();
        assert!(withheld > 0 && withheld < 1250);
        for p in &pts {
            assert!(p.x >= 10_100.0 - 1e-9 && p.x <= 10_109.8 + 1e-9, "{}", p.x);
            assert!(p.y >= 14_000.0 - 1e-9 && p.y <= 14_004.8 + 1e-9, "{}", p.y);
            assert!((p.z - 20.0).abs() < 1e-9);
            assert_eq!(p.intensity, 300);
            assert_eq!((p.return_number, p.number_of_returns), (2, 3));
            assert_eq!(u8::from(p.classification), 2);
            // 16-bit colour as the file had it.
            assert_eq!(p.color.unwrap().red, 10 * 257);
        }
        assert!((done.min[0] - 10_100.0).abs() < 1e-9 && (done.max[1] - 14_004.8).abs() < 1e-9);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cut_too_wide_for_a_grid_writes_nothing() {
    let dir = scratch("wide");
    let src = dir.join("site.las");
    write(&src);
    let ix = build_or_open(&src, &dir.join("cache"), &Progress::default()).unwrap();
    // Stretched 10 billion times: no grid of 32 bits holds it.
    let affine = [1e10, 0., 0., 0., 0., 1.0, 0., 0., 0., 0., 1.0, 0.];
    let ask = CropAsk {
        affine,
        boxes: vec![plain([-1e300, -1e300, -1e300], [1e300, 1e300, 1e300])],
        setup: None,
        scale: [0.0; 3],
        offset: [0.0; 3],
        wkt: String::new(),
        laz: false,
    };
    let out = dir.join("cut.las");
    assert!(crop(&ix, &ask, &out).is_err());
    assert!(!out.exists());
    std::fs::remove_dir_all(&dir).ok();
}
