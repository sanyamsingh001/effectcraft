use super::mesh::*;
use super::*;
use crate::{EffectEnv, flatten_params, instantiate};

fn disc_alpha(w: usize, h: usize, c: [f64; 2], r: f64) -> Vec<f32> {
    let mut a = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 + 0.5 - c[0]).powi(2) + (y as f64 + 0.5 - c[1]).powi(2)).sqrt();
            a[y * w + x] = if d <= r { 1.0 } else { 0.0 };
        }
    }
    a
}

fn check_valid(m: &Mesh) {
    assert!(!m.tris.is_empty());
    for t in &m.tris {
        let a = cross(m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]);
        assert!(a > 0.0, "flipped or degenerate triangle {t:?} area {a}");
    }
    let tri_area: f64 = m.tris.iter().map(|t| cross(m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]) * 0.5).sum();
    let poly_area = signed_area(&m.outline).abs();
    assert!((tri_area - poly_area).abs() < 1e-6 * poly_area.max(1.0), "triangles {tri_area} vs outline {poly_area}");
}

fn covered(m: &Mesh, p: [f64; 2]) -> bool {
    m.tris.iter().any(|t| {
        let (a, b, c) = (m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]);
        cross(a, b, p) >= -1e-9 && cross(b, c, p) >= -1e-9 && cross(c, a, p) >= -1e-9
    })
}

#[test]
fn distance_transform_matches_brute_force() {
    let (w, h) = (13, 9);
    let src: Vec<bool> = (0..w * h).map(|i| i % 17 == 3 || i == 50).collect();
    let d = distance_sq(w, h, &src);
    for y in 0..h {
        for x in 0..w {
            let mut best = f64::MAX;
            for yy in 0..h {
                for xx in 0..w {
                    if src[yy * w + xx] {
                        best = best.min(((x as f64 - xx as f64).powi(2)) + (y as f64 - yy as f64).powi(2));
                    }
                }
            }
            assert_eq!(d[y * w + x], best, "({x},{y})");
        }
    }
}

#[test]
fn triangulation_is_valid_and_covers_the_alpha() {
    let (w, h) = (120, 100);
    let alpha = disc_alpha(w, h, [60.0, 50.0], 38.0);
    let outline = outline(&alpha, w, h, 1.0, [0.0; 2], [60.0, 50.0], 3.0);
    let m = triangulate(&outline, &MeshOpts::default());
    check_valid(&m);
    assert!(m.tris.len() > 150 && m.tris.len() < 800, "{} triangles", m.tris.len());
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] > 0.0 {
                assert!(covered(&m, [x as f64 + 0.5, y as f64 + 0.5]), "alpha pixel ({x},{y}) not covered");
            }
        }
    }
    // Density controls the triangle count.
    let fine = triangulate(&outline, &MeshOpts { density: 100.0, ..MeshOpts::default() });
    check_valid(&fine);
    assert!(fine.tris.len() > m.tris.len() * 2);
}

#[test]
fn concave_outline_keeps_its_edges() {
    // A U shape (concave) and an L shape.
    let (w, h) = (100, 100);
    let mut alpha = vec![0.0f32; w * h];
    for y in 10..90 {
        for x in 10..90 {
            let notch = (35..65).contains(&x) && y < 70;
            alpha[y * w + x] = if notch { 0.0 } else { 1.0 };
        }
    }
    let o = outline(&alpha, w, h, 1.0, [0.0; 2], [20.0, 50.0], 0.0);
    let m = triangulate(&o, &MeshOpts { triangles: 120.0, ..Default::default() });
    check_valid(&m);
    // Nothing in the notch.
    assert!(!covered(&m, [50.0, 30.0]));
    for (x, y) in [(15.0, 15.0), (85.0, 15.0), (50.0, 80.0), (12.0, 88.0)] {
        assert!(covered(&m, [x, y]), "({x},{y})");
    }
}

#[test]
fn expansion_grows_the_outline() {
    let (w, h) = (80, 80);
    let alpha = disc_alpha(w, h, [40.0, 40.0], 20.0);
    let a0 = signed_area(&outline(&alpha, w, h, 1.0, [0.0; 2], [40.0, 40.0], 0.0)).abs();
    let a5 = signed_area(&outline(&alpha, w, h, 1.0, [0.0; 2], [40.0, 40.0], 5.0)).abs();
    let am = signed_area(&outline(&alpha, w, h, 1.0, [0.0; 2], [40.0, 40.0], -5.0)).abs();
    let pi = std::f64::consts::PI;
    assert!((a0 - pi * 400.0).abs() < 60.0, "{a0}");
    assert!((a5 - pi * 625.0).abs() < 90.0, "{a5}");
    assert!((am - pi * 225.0).abs() < 60.0, "{am}");
}

fn rect_mesh(w: f64, h: f64, tris: f64) -> Mesh {
    triangulate(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], &MeshOpts { triangles: tris, density: 50.0, expansion: 0.0 })
}

fn pin(kind: PinKind, rest: [f64; 2], pos: [f64; 2]) -> Pin {
    Pin { kind, uid: 0, rest, attachment: false, position: pos, scale: 1.0, rotation: 0.0, amount: 50.0, extent: 15.0, in_front: 50.0 }
}

#[test]
fn arap_identity_with_pins_at_rest() {
    let m = rect_mesh(100.0, 60.0, 200.0);
    check_valid(&m);
    let pins = [pin(PinKind::Position, [0.0, 0.0], [0.0, 0.0]), pin(PinKind::Position, [100.0, 60.0], [100.0, 60.0])];
    let d = solve(&m, &pins, 2);
    for (a, b) in d.iter().zip(&m.verts) {
        assert!((a[0] - b[0]).abs() < 1e-7 && (a[1] - b[1]).abs() < 1e-7, "{a:?} vs {b:?}");
    }
}

#[test]
fn arap_single_pin_translates_rigidly() {
    let m = rect_mesh(100.0, 60.0, 200.0);
    let d = solve(&m, &[pin(PinKind::Position, [50.0, 30.0], [70.0, 15.0])], 2);
    let v = m.verts[nearest(&m.verts, [50.0, 30.0]).unwrap()];
    let t = [70.0 - 50.0 + (v[0] - 50.0) * 0.0, 15.0 - 30.0];
    for (a, b) in d.iter().zip(&m.verts) {
        assert!((a[0] - b[0] - t[0]).abs() < 1e-7 && (a[1] - b[1] - t[1]).abs() < 1e-7, "{a:?} vs {b:?}");
    }
}

/// #313: the similarity solve has free rotation with only one Position pin. Large meshes
/// used to amplify round-off into a visible tilt even when the pin had never moved.
#[test]
fn first_pin_keeps_a_large_mesh_at_rest_everywhere() {
    let m = rect_mesh(800.0, 600.0, 350.0);
    for at in [[0.123456789, 0.987654321], [17.0, 29.0], [783.0, 29.0], [17.0, 571.0], [783.0, 571.0], [413.5, 287.25]] {
        for kind in [PinKind::Position, PinKind::Advanced, PinKind::Bend] {
            let d = solve(&m, &[pin(kind, at, at)], 2);
            assert!(d == m.verts, "an unchanged {kind:?} pin at {at:?} must preserve the rest mesh");
        }
    }
}

#[test]
fn one_pin_translates_a_large_mesh_without_rotation() {
    let m = rect_mesh(800.0, 600.0, 350.0);
    let delta = [112.5, -47.25];
    for at in [[17.0, 29.0], [783.0, 571.0], [413.5, 287.25]] {
        for kind in [PinKind::Position, PinKind::Advanced] {
            let d = solve(&m, &[pin(kind, at, [at[0] + delta[0], at[1] + delta[1]])], 2);
            for (actual, rest) in d.iter().zip(&m.verts) {
                assert!(
                    (actual[0] - rest[0] - delta[0]).abs() < 1e-9 && (actual[1] - rest[1] - delta[1]).abs() < 1e-9,
                    "{kind:?} pin at {at:?}: {actual:?} vs translated {rest:?}"
                );
            }
        }
    }
}

#[test]
fn advanced_pin_rotation_and_scale_still_deform_the_mesh() {
    let m = rect_mesh(100.0, 60.0, 200.0);
    let at = [50.0, 30.0];
    let p = Pin { rotation: 90.0, scale: 1.5, ..pin(PinKind::Advanced, at, at) };
    let d = solve(&m, &[p], 2);
    let a = nearest(&m.verts, [0.0, 30.0]).unwrap();
    let b = nearest(&m.verts, [100.0, 30.0]).unwrap();
    let edge = [d[b][0] - d[a][0], d[b][1] - d[a][1]];
    assert!(edge[1] > 80.0 && edge[0].abs() < 5.0, "rotated edge {edge:?}");
}

#[test]
fn arap_two_pins_rotate_rigidly() {
    let m = rect_mesh(100.0, 40.0, 200.0);
    let (a, b) = (m.verts[nearest(&m.verts, [0.0, 20.0]).unwrap()], m.verts[nearest(&m.verts, [100.0, 20.0]).unwrap()]);
    let c = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let ang = 30f64.to_radians();
    let rot = |p: [f64; 2]| {
        let (s, co) = ang.sin_cos();
        let d = [p[0] - c[0], p[1] - c[1]];
        [c[0] + d[0] * co - d[1] * s, c[1] + d[0] * s + d[1] * co]
    };
    let pins = [pin(PinKind::Position, a, rot(a)), pin(PinKind::Position, b, rot(b))];
    let d = solve(&m, &pins, 0);
    for (p, r) in d.iter().zip(&m.verts) {
        let e = rot(*r);
        assert!((p[0] - e[0]).abs() < 1e-3 && (p[1] - e[1]).abs() < 1e-3, "{p:?} vs {e:?}");
    }
}

fn distortion(m: &Mesh, d: &[[f64; 2]], near: [f64; 2], radius: f64) -> f64 {
    let mut s = 0.0;
    for t in &m.tris {
        let c = [(m.verts[t[0]][0] + m.verts[t[1]][0] + m.verts[t[2]][0]) / 3.0, (m.verts[t[0]][1] + m.verts[t[1]][1] + m.verts[t[2]][1]) / 3.0];
        if (c[0] - near[0]).hypot(c[1] - near[1]) > radius {
            continue;
        }
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let l0 = (m.verts[i][0] - m.verts[j][0]).hypot(m.verts[i][1] - m.verts[j][1]);
            let l1 = (d[i][0] - d[j][0]).hypot(d[i][1] - d[j][1]);
            s += (l1 / l0 - 1.0).powi(2);
        }
    }
    s
}

#[test]
fn starch_reduces_distortion() {
    let m = rect_mesh(200.0, 40.0, 300.0);
    // Squash the bar: both ends pushed inwards and one end lifted.
    let base = vec![pin(PinKind::Position, [0.0, 20.0], [30.0, 20.0]), pin(PinKind::Position, [200.0, 20.0], [160.0, 60.0])];
    let plain = solve(&m, &base, 2);
    let mut st = base.clone();
    st.push(Pin { amount: 100.0, extent: 30.0, ..pin(PinKind::Starch, [100.0, 20.0], [100.0, 20.0]) });
    let starched = solve(&m, &st, 2);
    let (a, b) = (distortion(&m, &plain, [100.0, 20.0], 25.0), distortion(&m, &starched, [100.0, 20.0], 25.0));
    assert!(b < a * 0.5, "starch {b} vs plain {a}");
}

#[test]
fn bend_pin_rotates_around_it() {
    let m = rect_mesh(100.0, 40.0, 200.0);
    let mut bend = pin(PinKind::Bend, [100.0, 20.0], [100.0, 20.0]);
    bend.rotation = 45.0;
    let d = solve(&m, &[pin(PinKind::Position, [0.0, 20.0], [0.0, 20.0]), bend], 2);
    // The far end turned downwards (positive rotation in y-down layer space).
    let v = nearest(&m.verts, [100.0, 40.0]).unwrap();
    let u = nearest(&m.verts, [100.0, 0.0]).unwrap();
    assert!(d[v][0] < d[u][0] - 5.0, "bottom {:?} top {:?}", d[v], d[u]);
}

#[test]
fn overlap_pins_set_depth() {
    let m = rect_mesh(100.0, 40.0, 100.0);
    let mut o = pin(PinKind::Overlap, [20.0, 20.0], [20.0, 20.0]);
    o.in_front = 80.0;
    o.extent = 15.0;
    let d = depths(&m, &[o]);
    assert!(d.contains(&80.0));
    assert!(d.contains(&0.0));
}

fn puppet_fx(pins: &[(PinKind, [f64; 2], [f64; 2])]) -> PropGroup {
    let mut next = 5000;
    let mut ids = Ids(&mut next);
    let mut fx = instantiate(crate::find(ID).unwrap(), &mut ids, "Puppet", [64.0, 64.0]);
    let mut mesh = mesh_group(&mut ids, "Mesh 1", pins.first().map(|p| p.1).unwrap_or([32.0; 2]), &MeshOpts::default());
    for (k, (kind, rest, pos)) in pins.iter().enumerate() {
        let mut g = pin_group(&mut ids, &format!("Puppet Pin {}", k + 1), *kind, *rest);
        if let Some(p) = g.get_mut("position") {
            p.value = Value::Vec2(*pos);
        }
        mesh.sub_mut("deform").unwrap().children.push(g.into());
    }
    fx.children.push(mesh.into());
    fx
}

fn run(fx: &PropGroup, img: Image) -> Buf {
    let params = flatten_params(fx, &mut |p| p.value.clone());
    let ctx =
        EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: EffectEnv::default() };
    crate::apply(crate::find(ID).unwrap(), &ctx, Buf { img, offset: [0.0; 2], scale: 1.0 })
}

fn test_image() -> Image {
    let mut img = Image::new(64, 64);
    for y in 0..64u32 {
        for x in 0..64u32 {
            if (12..52).contains(&x) && (16..48).contains(&y) {
                img.set(x, y, [x as f32 / 64.0, y as f32 / 64.0, ((x + y) % 3) as f32 / 3.0, 1.0]);
            }
        }
    }
    img
}

/// Output pixel at layer pixel (x, y).
fn at(b: &Buf, x: i64, y: i64) -> [f32; 4] {
    b.img.get(x + b.offset[0] as i64, y + b.offset[1] as i64)
}

#[test]
fn render_identity_and_translation() {
    let img = test_image();
    let id = run(&puppet_fx(&[(PinKind::Position, [30.0, 30.0], [30.0, 30.0])]), img.clone());
    for y in 17..47 {
        for x in 13..51 {
            let (o, e) = (at(&id, x, y), img.get(x, y));
            assert!((0..4).all(|c| (o[c] - e[c]).abs() < 1e-5), "({x},{y}) {o:?} vs {e:?}");
        }
    }
    let moved = run(&puppet_fx(&[(PinKind::Position, [30.0, 30.0], [40.0, 35.0])]), img.clone());
    for y in 17..47 {
        for x in 13..51 {
            let o = at(&moved, x + 10, y + 5);
            let e = img.get(x, y);
            for c in 0..4 {
                assert!((o[c] - e[c]).abs() < 1e-4, "({x},{y}) {o:?} vs {e:?}");
            }
        }
    }
    // Outside the mesh: nothing.
    assert_eq!(at(&moved, 2, 2), [0.0; 4]);
}

#[test]
fn render_is_deterministic_and_antialiased() {
    let img = test_image();
    let fx = puppet_fx(&[(PinKind::Position, [14.0, 30.0], [14.0, 30.0]), (PinKind::Position, [50.0, 30.0], [45.0, 10.0])]);
    let a = run(&fx, img.clone());
    let b = run(&fx, img);
    assert_eq!(a.img, b.img);
    assert_eq!(a.offset, b.offset);
    assert!(a.img.data.iter().any(|p| p[3] > 0.05 && p[3] < 0.95), "expected soft edges");
    assert!(a.img.data.iter().all(|p| p[3] <= 1.0001 && p.iter().all(|c| c.is_finite())));
}

#[test]
fn deformed_mesh_contains_points_in_either_winding() {
    let m = rect_mesh(100.0, 40.0, 150.0);
    let d: Vec<_> = m.verts.iter().map(|p| [210.0 - p[0], p[1] + 20.0]).collect();
    assert!(contains(&m, &d, [160.0, 40.0]));
    assert!(!contains(&m, &d, [50.0, 20.0]));
    assert!(!contains(&m, &[], [160.0, 40.0]));
    assert!(!contains(&m, &vec![[0.0; 2]; m.verts.len()], [0.0, 0.0]));
}

#[test]
fn unmap_inverts_the_deformation() {
    let m = rect_mesh(100.0, 40.0, 150.0);
    let pins = [pin(PinKind::Position, [0.0, 20.0], [10.0, 30.0]), pin(PinKind::Position, [100.0, 20.0], [90.0, 0.0])];
    let d = solve(&m, &pins, 1);
    let q = unmap(&m, &d, d[5]);
    assert!((q[0] - m.verts[5][0]).abs() < 1e-6 && (q[1] - m.verts[5][1]).abs() < 1e-6);
}

#[test]
fn groups_round_trip_serde() {
    let fx = puppet_fx(&[(PinKind::Position, [1.0, 2.0], [3.0, 4.0]), (PinKind::Starch, [5.0, 5.0], [5.0, 5.0])]);
    let j = serde_json::to_string(&fx).unwrap();
    let back: PropGroup = serde_json::from_str(&j).unwrap();
    assert_eq!(back, fx);
    assert_eq!(pin_count(&back), 2);
    let params = flatten_params(&back, &mut |p| p.value.clone());
    let m = parse(&params);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].pins.len(), 2);
    assert_eq!(m[0].pins[1].kind, PinKind::Starch);
    assert_eq!(m[0].pins[0].position, [3.0, 4.0]);
}

fn attachment_at(mesh: &Mesh, pose: &[[f64; 2]], rest: [f64; 2]) -> Pin {
    let point = attachment_point(mesh, rest, rest).unwrap();
    let position = [0, 1].map(|c| point.vertices.iter().zip(point.weights).map(|(i, w)| pose[*i][c] * w).sum());
    Pin { attachment: true, ..pin(PinKind::Position, rest, position) }
}
fn attachment_position(mesh: &Mesh, pose: &[[f64; 2]], p: Pin) -> [f64; 2] {
    let point = attachment_point(mesh, p.rest, p.position).unwrap();
    [0, 1].map(|c| point.vertices.iter().zip(point.weights).map(|(i, w)| pose[*i][c] * w).sum())
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn assert_attachment(mesh: &Mesh, pose: &[[f64; 2]], pin: Pin) {
    let actual = attachment_position(mesh, pose, pin);
    assert!(distance(actual, pin.position) < 1e-7, "{actual:?} vs {:?}", pin.position);
}

#[test]
fn posed_position_attachment_preserves_shape_and_moves_continuously() {
    let m = rect_mesh(800.0, 160.0, 350.0);
    let old = [pin(PinKind::Position, [0.0, 80.0], [0.0, 80.0]), pin(PinKind::Position, [800.0, 80.0], [640.0, -180.0])];
    let before = solve(&m, &old, 2);
    let third = attachment_at(&m, &before, [730.0, 50.0]);
    assert!(can_attach(&m, &old, third.rest));
    let unposed = unmap(&m, &before, third.position);
    assert!(distance(unposed, third.rest) < 1e-8);
    assert_eq!(solve(&m, &[old[0], old[1], third], 2), before, "adding a pin must preserve every posed vertex");
    for delta in [0.0001, 1.0, 25.0] {
        let moved = Pin { position: [third.position[0] + delta, third.position[1]], ..third };
        let after = solve(&m, &[old[0], old[1], moved], 2);
        assert_attachment(&m, &after, moved);
        assert!(after.iter().flatten().all(|v| v.is_finite()));
        if delta < 0.001 {
            let max = after.iter().zip(&before).map(|(a, b)| distance(*a, *b)).fold(0.0, f64::max);
            assert!(max < 0.01, "tiny movement caused a jump of{max}px");
        }
        for p in old {
            let v = nearest(&m.verts, p.rest).unwrap();
            let target = [m.verts[v][0] + (p.position[0] - p.rest[0]), m.verts[v][1] + (p.position[1] - p.rest[1])];
            assert!(distance(after[v], target) < 1e-8);
        }
        let fourth = attachment_at(&m, &after, [500.0, 100.0]);
        assert!(can_attach(&m, &[old[0], old[1], moved], fourth.rest));
        assert_eq!(solve(&m, &[old[0], old[1], moved, fourth], 2), after);
        let fourth = Pin { position: [fourth.position[0], fourth.position[1] - 10.0], ..fourth };
        let both = solve(&m, &[old[0], old[1], moved, fourth], 2);
        assert_attachment(&m, &both, moved);
        assert_attachment(&m, &both, fourth);
    }
}

#[test]
fn attachment_holds_during_old_pin_animation_and_can_return_to_rest() {
    let m = rect_mesh(800.0, 160.0, 350.0);
    let left = pin(PinKind::Position, [0.0, 80.0], [0.0, 80.0]);
    let right = pin(PinKind::Position, [800.0, 80.0], [640.0, -180.0]);
    let third = attachment_at(&m, &solve(&m, &[left, right], 2), [730.0, 50.0]);
    for position in [[700.0, -100.0], [800.0, 80.0], [600.0, -220.0]] {
        let animated = Pin { position, ..right };
        let pose = solve(&m, &[left, animated, third], 2);
        assert_attachment(&m, &pose, third);
        let v = nearest(&m.verts, right.rest).unwrap();
        assert!(distance(pose[v], [m.verts[v][0] + position[0] - right.rest[0], m.verts[v][1] + position[1] - right.rest[1]]) < 1e-8);
    }
    assert_eq!(solve(&m, &[left, Pin { position: right.rest, ..right }, Pin { position: third.rest, ..third }], 2), m.verts);
}

#[test]
fn attachment_rejects_dependent_outside_and_degenerate_points() {
    let m = rect_mesh(100.0, 60.0, 200.0);
    let first = pin(PinKind::Position, m.verts[0], m.verts[0]);
    assert!(!can_attach(&m, &[first], m.verts[0]));
    assert!(!can_attach(&m, &[], [-100.0, -100.0]));
    assert!(!can_attach(&m, &[], [f64::NAN, 0.0]));
    let p = attachment_at(&m, &m.verts, [50.0, 30.0]);
    assert!(!can_attach(&m, &[p], p.rest));
    let degenerate = Mesh { verts: vec![[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]], tris: vec![[0, 1, 2]], outline: vec![] };
    assert!(!can_attach(&degenerate, &[], [50.0, 0.0]));
    let outside = Pin { attachment: true, ..pin(PinKind::Position, [-100.0, -100.0], [-90.0, -90.0]) };
    assert_eq!(solve(&m, &[outside], 2), m.verts);
    let thin = Mesh { verts: vec![[0.0, 0.0], [100.0, 0.0], [0.0, 1e-10]], tris: vec![[0, 1, 2]], outline: vec![] };
    let p = attachment_at(&thin, &thin.verts, [20.0, 2e-11]);
    assert!(can_attach(&thin, &[], p.rest));
    assert_eq!(solve(&thin, &[p], 2), thin.verts);
    let moved = Pin { position: [21.0, 2e-11], ..p };
    let pose = solve(&thin, &[moved], 2);
    assert!(pose.iter().flatten().all(|v| v.is_finite()));
    assert_attachment(&thin, &pose, moved);
}

#[test]
fn attachment_budget_is_checked_before_insertion_and_neutral_solves_are_exact() {
    let m = rect_mesh(800.0, 160.0, 350.0);
    let mut pins = vec![];
    for v in m.verts.iter().take(MAX_ATTACHMENTS) {
        assert!(can_attach(&m, &pins, *v));
        pins.push(Pin { attachment: true, ..pin(PinKind::Position, *v, *v) });
    }
    assert!(!can_attach(&m, &pins, m.verts[MAX_ATTACHMENTS]));
    assert_eq!(solve(&m, &pins, 2), m.verts);
}

#[test]
fn posed_attachment_preserves_rendered_pixels_and_serializes() {
    let mut fx = puppet_fx(&[(PinKind::Position, [15.0, 30.0], [15.0, 30.0]), (PinKind::Position, [50.0, 30.0], [40.0, 10.0])]);
    let image = test_image();
    let input = Buf { img: image, offset: [0.0; 2], scale: 1.0 };
    let params = flatten_params(&fx, &mut |p| p.value.clone());
    let meshes = overlay(&input, &params);
    let (uid, mesh, pose, existing) = &meshes[0];
    let triangle = mesh.tris[mesh.tris.len() / 2];
    let rest = [0, 1].map(|c| triangle.iter().map(|i| mesh.verts[*i][c]).sum::<f64>() / 3.0);
    let attached = attachment_at(mesh, pose, rest);
    assert!(can_attach(mesh, existing, rest));
    let apply = |group: &PropGroup| {
        let params = flatten_params(group, &mut |p| p.value.clone());
        let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [64.0; 2], seed: 1, adjustment: false, env: EffectEnv::default() };
        crate::apply(&specs().remove(0), &ctx, input.clone())
    };
    let before = apply(&fx);
    let mut next = 9000;
    let mut ids = Ids(&mut next);
    let mut pin = pin_group(&mut ids, "Puppet Pin 3", PinKind::Position, rest);
    pin.children.push(hidden(&mut ids, "attachment", Value::Bool(true)).into());
    pin.get_mut("position").unwrap().value = Value::Vec2(attached.position);
    let json = serde_json::to_value(&pin).unwrap();
    assert!(json.to_string().contains("attachment"));
    let pin: PropGroup = serde_json::from_value(json).unwrap();
    fx.find_group_mut(*uid).unwrap().sub_mut("deform").unwrap().children.push(pin.into());
    let after = apply(&fx);
    assert_eq!(after.offset, before.offset);
    assert_eq!(after.scale, before.scale);
    assert_eq!(after.img, before.img, "placing the pin must preserve every rendered pixel");
    let params = flatten_params(&fx, &mut |p| p.value.clone());
    assert!(parse(&params)[0].pins.last().unwrap().attachment);
}

#[test]
fn malformed_and_huge_attachment_rows_do_not_panic_or_propagate_nan() {
    let m = rect_mesh(100.0, 60.0, 200.0);
    assert!(!can_attach(&m, &[], [1e308, 1e308]));
    let mut handles = arap::Handles::default();
    for point in [
        arap::PointTarget { vertices: [usize::MAX, 0, 1], weights: [1.0, 0.0, 0.0], position: [10.0, 10.0] },
        arap::PointTarget { vertices: [0, 1, 2], weights: [f64::NAN, 0.5, 0.5], position: [10.0, 10.0] },
        arap::PointTarget { vertices: [0, 1, 2], weights: [1e308, 1e308, 0.0], position: [10.0, 10.0] },
        arap::PointTarget { vertices: [0, 1, 2], weights: [0.2, 0.4, 0.4], position: [f64::INFINITY, 10.0] },
    ] {
        handles.points = vec![point];
        assert_eq!(arap::deform(&m.verts, &m.tris, &[], &handles, 2), m.verts);
    }
    handles.points = vec![arap::PointTarget { vertices: [0, 1, 2], weights: [0.2, 0.4, 0.4], position: [1e308, 1e308] }];
    let pose = arap::deform(&m.verts, &m.tris, &[], &handles, 2);
    assert!(pose.iter().flatten().all(|v| v.is_finite()));
    let malformed = Pin { attachment: true, ..pin(PinKind::Position, [1e308, 1e308], [1e308, 1e308]) };
    assert_eq!(solve(&m, &[malformed], 2), m.verts);
    let extremes = [
        Pin { attachment: true, ..pin(PinKind::Position, [30.0, 20.0], [1e308, 1e308]) },
        Pin { attachment: true, ..pin(PinKind::Position, [70.0, 40.0], [-1e308, -1e308]) },
    ];
    assert!(solve(&m, &extremes, 2).iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn only_remaining_attachment_translates_without_rotation() {
    let m = rect_mesh(800.0, 600.0, 350.0);
    let delta = [112.5, -47.25];
    let rest = [17.0, 29.0];
    let p = Pin { attachment: true, ..pin(PinKind::Position, rest, [rest[0] + delta[0], rest[1] + delta[1]]) };
    let pose = solve(&m, &[p], 2);
    for (a, b) in pose.iter().zip(&m.verts) {
        assert!(distance(*a, [b[0] + delta[0], b[1] + delta[1]]) < 1e-7, "{a:?} vs translated{b:?}");
    }
}

#[test]
fn render_off_canvas_translation_preserves_the_solid() {
    let source = Image::filled(16, 12, [0.25, 0.5, 0.75, 1.0]);
    for delta in [[256.0, 0.0], [-256.0, 0.0], [0.0, 256.0], [0.0, -256.0]] {
        let output = run(&puppet_fx(&[(PinKind::Position, [8.0, 6.0], [8.0 + delta[0], 6.0 + delta[1]])]), source.clone());
        assert!(output.offset.iter().all(|v| v.is_finite()));
        assert!(output.img.data.iter().flatten().all(|v| v.is_finite()));
        for y in 0..12 {
            for x in 0..16 {
                assert_eq!(at(&output, x, y), [0.0; 4], "original canvas at ({x},{y}), delta {delta:?}");
            }
        }
        assert_eq!(at(&output, 8 + delta[0] as i64, 6 + delta[1] as i64), [0.25, 0.5, 0.75, 1.0]);
    }
}

#[test]
fn render_far_off_canvas_translation_stays_finite_and_transparent() {
    let source = Image::filled(16, 12, [0.25, 0.5, 0.75, 1.0]);
    for position in [[1e20, 6.0], [-1e20, 6.0], [8.0, 1e20], [8.0, -1e20]] {
        let output = run(&puppet_fx(&[(PinKind::Position, [8.0, 6.0], position)]), source.clone());
        assert!(output.offset.iter().all(|v| v.is_finite()), "position {position:?}");
        assert!(output.img.data.iter().all(|p| *p == [0.0; 4]), "pixel precision cannot resolve a solid at {position:?}");
    }
}
