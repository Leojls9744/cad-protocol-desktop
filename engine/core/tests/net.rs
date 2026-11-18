//! Geo W. (1.23): the trained scan classifier as the page runs it.

use cadp_engine_core::net::{classify, cubes, group_shapes, Layer, Model, Returns};

fn model(inputs: &[&str]) -> Model {
    let more = inputs.iter().map(|i| match *i {
        "returns" => 2,
        "normal" => 3,
        _ => 1,
    });
    let more: usize = more.sum();
    let mut floats = 0usize;
    let mut layer = |i: usize, o: usize| {
        let l = Layer { inp: i, out: o, w: floats, b: floats + i * o };
        floats += i * o + o;
        l
    };
    let branches = [
        vec![layer(8 + more, 8), layer(8, 8)],
        vec![layer(4 + more, 8), layer(8, 8)],
        vec![layer(4 + more, 8), layer(8, 8)],
    ];
    let head = vec![layer(26, 8), layer(8, 5)];
    let blob = (0..floats).map(|i| ((i as f32) * 0.37).sin() * 0.4).collect();
    Model {
        branches,
        head,
        classes: 5,
        returns: inputs.contains(&"returns"),
        normal: inputs.contains(&"normal"),
        curvature: inputs.contains(&"curvature"),
        brk: inputs.contains(&"break"),
        blob,
    }
}

/// A floor and a wall meeting it, a point every 0.1 ft.
fn corner() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (mut x, mut y, mut z) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..60 {
        for j in 0..60 {
            x.push(i as f64 * 0.1);
            y.push(j as f64 * 0.1);
            z.push(100.0);
            x.push(0.0);
            y.push(j as f64 * 0.1);
            z.push(100.0 + i as f64 * 0.1);
        }
    }
    (x, y, z)
}

#[test]
fn checks_a_model_fits_together() {
    assert!(model(&[]).check().is_ok());
    assert!(model(&["returns", "normal", "curvature", "break"]).check().is_ok());
    let mut bad = model(&[]);
    bad.normal = true;
    assert!(bad.check().is_err(), "first layers taking version 1's numbers only");
}

#[test]
fn classifies_every_point_by_its_cube_the_same_whatever_the_batch() {
    let (x, y, z) = corner();
    let n = x.len();
    let intensity: Vec<f64> = (0..n).map(|i| (i % 10) as f64 / 10.0).collect();
    let rgb: Vec<f64> = (0..n * 3).map(|i| (i % 256) as f64).collect();
    let rn = vec![1u8; n];
    let nr: Vec<u8> = (0..n).map(|i| 1 + (i % 2) as u8).collect();
    for inputs in [&[][..], &["returns", "normal", "curvature", "break"][..]] {
        let m = model(inputs);
        let (a, sa, cubes_a) = classify(&m, &x, &y, &z, &intensity, Some(&rgb), Some(Returns { number: &rn, of: &nr }), 2048);
        let (b, sb, cubes_b) = classify(&m, &x, &y, &z, &intensity, Some(&rgb), Some(Returns { number: &rn, of: &nr }), 7);
        assert_eq!(cubes_a, cubes_b);
        assert_eq!(a, b);
        assert_eq!(sa, sb);
        assert_eq!(a.len(), n);
        assert!(a.iter().all(|c| *c < 5));
        assert!(sa.iter().all(|s| *s > 0.0 && *s <= 1.0));
        // Cubes of 0.25 ft: the floor's 24 × 24 and the wall's, the corner's shared.
        assert!(cubes_a > 1100 && cubes_a <= 1152, "{cubes_a}");
    }
}

#[test]
fn finds_each_cubes_surface_and_the_break_where_a_wall_meets_the_floor() {
    let (x, y, z) = corner();
    let n = x.len();
    let zero = vec![0.0; n];
    let (level, of) = cubes(&x, &y, &z, &zero, None, 0.25);
    let kx: Vec<f64> = (0..level.n).map(|c| (level.x[c] / 0.25).floor()).collect();
    let ky: Vec<f64> = (0..level.n).map(|c| (level.y[c] / 0.25).floor()).collect();
    let kz: Vec<f64> = (0..level.n).map(|c| (level.z[c] / 0.25).floor()).collect();
    // The keys as the points made them.
    let mut pk = (vec![0.0; level.n], vec![0.0; level.n], vec![0.0; level.n]);
    for i in 0..n {
        let c = of[i] as usize;
        pk.0[c] = (x[i] / 0.25).floor();
        pk.1[c] = (y[i] / 0.25).floor();
        pk.2[c] = (z[i] / 0.25).floor();
    }
    let _ = (kx, ky, kz);
    let s = group_shapes(&x, &y, &z, &of, level.n, &pk.0, &pk.1, &pk.2);
    // A cube in the floor's middle: facing up, flat, no break.
    let mid = (0..level.n).find(|c| (level.x[*c] - 3.0).abs() < 0.2 && (level.y[*c] - 3.0).abs() < 0.2 && level.z[*c] == 100.0).unwrap();
    assert!((s.nz[mid] - 1.0).abs() < 1e-6 && s.curvature[mid] < 1e-6 && s.break_deg[mid] < 1e-3);
    // A cube of the wall's: facing along x.
    let wall = (0..level.n).find(|c| level.x[*c] == 0.0 && level.z[*c] > 103.0).unwrap();
    assert!(s.nx[wall].abs() > 0.999, "{}", s.nx[wall]);
    // Where they meet: a break of about 90°.
    let foot = (0..level.n).filter(|c| level.z[*c] < 100.2 && level.x[*c] < 0.3).map(|c| s.break_deg[c]).fold(0f32, f32::max);
    assert!(foot > 80.0, "{foot}");
}
