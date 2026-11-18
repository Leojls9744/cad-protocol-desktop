//! E57 scans (1.24): read as the page reads them, indexed as LAS scans are.

use cadp_engine_core::e57::{self, Places, WritePoint, WriteScan};
use cadp_engine_core::{build_or_open, region, snap, Progress, RegionQuery, ScanIn, SnapQuery};
use std::path::PathBuf;

const IDENTITY: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
const ALL: [u64; 4] = [u64::MAX; 4];

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cadp-e57-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A floor's points, 0.1 m apart, `n` × `n`, about (0, 0).
fn floor(n: usize, z: f64) -> Vec<WritePoint> {
    let mut out = Vec::new();
    for i in 0..n {
        for j in 0..n {
            out.push(WritePoint { at: [i as f64 * 0.1, j as f64 * 0.1, z], intensity: (i * 7 % 2048) as f64, rgb: [i as u8, j as u8, 9], invalid: false });
        }
    }
    out
}

/// Three setups: doubles moved by a pose, whole millimetres, and range and angles; some points marked invalid, some at the scanner.
fn three(path: &std::path::Path) {
    let mut a = floor(40, 0.0);
    a[3].invalid = true;
    a[5].at = [0.0, 0.0, 0.0];
    let b = floor(30, 1.0);
    let c: Vec<WritePoint> = (0..500)
        .map(|k| {
            let az = k as f64 * 0.01;
            WritePoint { at: [10.0, az, -0.2], intensity: 1000.0, rgb: [0, 0, 0], invalid: false }
        })
        .collect();
    // A quarter turn about up, then a shift.
    let h = std::f64::consts::FRAC_1_SQRT_2;
    e57::write(
        path,
        &[
            WriteScan { name: "Station 1".into(), pose: Some(([h, 0.0, 0.0, h], [500_000.0, 4_000_000.0, 100.0])), places: Places::Float, intensity_whole: Some(2047), colour: true, invalid_field: true, points: a },
            WriteScan { name: "Station 2".into(), pose: Some(([1.0, 0.0, 0.0, 0.0], [500_010.0, 4_000_000.0, 100.0])), places: Places::Scaled { scale: 0.001, min: -1_000_000, max: 1_000_000 }, intensity_whole: Some(2047), colour: true, invalid_field: false, points: b },
            WriteScan { name: "Station 3".into(), pose: Some(([1.0, 0.0, 0.0, 0.0], [500_000.0, 4_000_020.0, 101.5])), places: Places::Spherical, intensity_whole: None, colour: false, invalid_field: false, points: c },
        ],
    )
    .unwrap();
}

#[test]
fn reads_setups_as_the_page_does() {
    let d = dir("read");
    let f = d.join("site.e57");
    three(&f);
    let mut pts = Vec::new();
    let facts = e57::read(&f, |p| {
        pts.push(*p);
        true
    }, |_, _| {}).unwrap();
    assert_eq!(facts.scans, 3);
    assert!(facts.colour);
    assert_eq!(facts.records, 1600 + 900 + 500);
    // The invalid one and the two at the scanner (the floor's corner, and one put there) left out.
    assert_eq!(pts.len(), 1597 + 900 + 500);
    let first: Vec<_> = pts.iter().filter(|p| p.setup == 0).collect();
    // A quarter turn: (0.1, 0, 0) goes to (0, 0.1, 0), then the shift.
    let p = first.iter().find(|p| (p.y - 4_000_000.0 - 0.3).abs() < 1e-9 && (p.x - 500_000.0).abs() < 1e-9).expect("the turned point");
    assert!((p.z - 100.0).abs() < 1e-9);
    // Intensity by its limits (0 to 2047), colour as written.
    assert_eq!(first[0].intensity, 0);
    assert!(first.iter().any(|p| p.intensity == ((7.0 * 65535.0 / 2047.0) as f64).round() as u16));
    let second: Vec<_> = pts.iter().filter(|p| p.setup == 1).collect();
    assert!((second[0].z - 101.0).abs() < 1e-9 && (second[0].x - 500_010.0).abs() < 1e-9);
    // Range and angles: 10 m out, 0.2 rad down.
    let third: Vec<_> = pts.iter().filter(|p| p.setup == 2).collect();
    let r = third[0];
    assert!((r.x - 500_000.0 - 10.0 * (-0.2f64).cos()).abs() < 1e-9 && (r.z - 101.5 - 10.0 * (-0.2f64).sin()).abs() < 1e-9);
    assert_eq!(r.rgb, [0, 0, 0]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn indexes_an_e57_keeping_each_points_setup_and_asks_of_one_alone() {
    let d = dir("index");
    let f = d.join("site.e57");
    three(&f);
    let ix = build_or_open(&f, &d.join("cache"), &Progress::default()).unwrap();
    assert!(ix.built && ix.e57 && ix.setups && ix.has_rgb && !ix.wide);
    assert_eq!(ix.count, 1597 + 900 + 500);
    // Everything, then the second setup alone.
    let q = RegionQuery { min: [-1e12, -1e12, -1e9], max: [1e12, 1e12, 1e9], ring: None, most: 10_000_000, setup: None };
    let all = region(&ix, &IDENTITY, &q);
    assert!(all.complete);
    assert_eq!(all.class.len() as u64, ix.count);
    assert!(all.class.iter().all(|c| *c == 1));
    let setups = all.setup.as_ref().expect("each point's setup");
    assert_eq!(setups.iter().filter(|k| **k == 1).count(), 900);
    let one = region(&ix, &IDENTITY, &RegionQuery { setup: Some(1), ..q.clone() });
    assert_eq!(one.class.len(), 900);
    assert!(one.setup.unwrap().iter().all(|k| *k == 1));
    // Placed close by, to a tenth of a millimetre.
    let zs: Vec<f64> = (0..one.class.len()).map(|i| one.xyz[i * 3 + 2] as f64 + one.origin[2]).collect();
    assert!(zs.iter().all(|z| (z - 101.0).abs() < 1e-3), "{:?}", &zs[..3]);
    // A snap from above at the second setup's corner, then asked of the first setup alone: nothing there.
    let ask = SnapQuery {
        target: [500_010.0, 4_000_000.0, 0.0],
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
        most: 32,
        front_most: 8,
    };
    let got = snap(&[ScanIn { index: &ix, affine: IDENTITY, allow: ALL, setup: None }], &ask);
    assert!(!got.front.is_empty());
    assert!((got.front[0].p[2] - 101.0).abs() < 1e-3, "{:?}", got.front[0].p);
    let alone = snap(&[ScanIn { index: &ix, affine: IDENTITY, allow: ALL, setup: Some(0) }], &ask);
    assert!(alone.front.iter().all(|c| (c.p[2] - 100.0).abs() < 1e-3));
    // Opened again from its index.
    let again = build_or_open(&f, &d.join("cache"), &Progress::default()).unwrap();
    assert!(!again.built && again.setups);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn reads_a_file_another_program_wrote() {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/ColouredCubeFloat.e57");
    let mut n = 0u64;
    let mut coloured = 0u64;
    let facts = e57::read(&f, |p| {
        n += 1;
        if p.rgb != [0, 0, 0] {
            coloured += 1;
        }
        true
    }, |_, _| {})
    .unwrap();
    assert_eq!(facts.scans, 1);
    assert!(facts.colour);
    assert!(n > 0 && n <= facts.records, "{n} of {}", facts.records);
    assert!(coloured > 0);
    let d = dir("cube");
    let ix = build_or_open(&f, &d.join("cache"), &Progress::default()).unwrap();
    assert_eq!(ix.count, n);
    assert!(!ix.setups);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn says_what_isnt_an_e57() {
    let d = dir("bad");
    let f = d.join("not.e57");
    std::fs::write(&f, b"not an e57 file at all, long enough to have a header here...").unwrap();
    let err = build_or_open(&f, &d.join("cache"), &Progress::default()).err().unwrap();
    assert!(err.to_string().contains("ASTM-E57"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[ignore]
fn speed() {
    let d = dir("speed");
    let f = d.join("big.e57");
    let mut seed: u32 = 3;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed as f64 / 4_294_967_296.0
    };
    let scans: Vec<WriteScan> = (0..4)
        .map(|k| WriteScan {
            name: format!("S{k}"),
            pose: Some(([1.0, 0.0, 0.0, 0.0], [1_916_000.0 + k as f64 * 20.0, 640_000.0, 100.0])),
            places: Places::Scaled { scale: 0.001, min: -100_000_000, max: 100_000_000 },
            intensity_whole: Some(2047),
            colour: true,
            invalid_field: false,
            points: (0..5_000_000).map(|_| WritePoint { at: [rnd() * 60.0 - 30.0, rnd() * 60.0 - 30.0, rnd() * 4.0], intensity: rnd() * 2047.0, rgb: [9, 9, 9], invalid: false }).collect(),
        })
        .collect();
    e57::write(&f, &scans).unwrap();
    drop(scans);
    let size = std::fs::metadata(&f).unwrap().len();
    let t = std::time::Instant::now();
    let mut n = 0u64;
    e57::read(&f, |_| {
        n += 1;
        true
    }, |_, _| {}).unwrap();
    let read = t.elapsed();
    let t = std::time::Instant::now();
    let ix = build_or_open(&f, &d.join("cache"), &Progress::default()).unwrap();
    eprintln!("{} MB, {n} points: read {:?}, indexed {:?} ({} points)", size >> 20, read, t.elapsed(), ix.count);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn turns_down_a_scan_whose_points_hold_nothing() {
    let d = dir("empty");
    let f = d.join("one-value.e57");
    let pts: Vec<WritePoint> = (0..300).map(|k| WritePoint { at: [k as f64 * 0.01, 1.0, 2.0], intensity: 100.0, rgb: [0, 0, 0], invalid: false }).collect();
    e57::write(
        &f,
        &[WriteScan { name: "Station 1".into(), pose: None, places: Places::Scaled { scale: 0.001, min: -1_000_000, max: 1_000_000 }, intensity_whole: Some(2047), colour: false, invalid_field: false, points: pts }],
    )
    .unwrap();
    // Every field made one value, the same length, so the file's places hold.
    let mut b = std::fs::read(&f).unwrap();
    let swap = |b: &mut Vec<u8>, from: &str, to: &str| {
        assert_eq!(from.len(), to.len());
        let mut n = 0;
        while let Some(at) = b.windows(from.len()).position(|w| w == from.as_bytes()) {
            b[at..at + from.len()].copy_from_slice(to.as_bytes());
            n += 1;
        }
        n
    };
    assert_eq!(swap(&mut b, "minimum=\"-1000000\" maximum=\"1000000\"", "minimum=\"5\"        maximum=\"5\"      "), 3);
    assert_eq!(swap(&mut b, "minimum=\"0\" maximum=\"2047\"", "minimum=\"0\" maximum=\"0\"   "), 1);
    std::fs::write(&f, &b).unwrap();
    let err = e57::read(&f, |_| true, |_, _| {}).expect_err("turned down");
    assert!(err.to_string().contains("hold no values"), "{err}");
    let cache = d.join("cache");
    assert!(build_or_open(&f, &cache, &Progress::default()).is_err());
    let left: Vec<_> = std::fs::read_dir(&cache).unwrap().flatten().collect();
    assert!(left.is_empty(), "nothing left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&d);
}
