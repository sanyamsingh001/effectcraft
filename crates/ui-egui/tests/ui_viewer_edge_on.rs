//! Diagnostic pointer proof for a visibly rendered edge-on 3D source.
//! RotationY=90 makes the existing layer XY handle plane singular, while the cube has volume.

use effectcraft_engine::{Session, project::LayerId};
use effectcraft_ui_egui::{EffectcraftApp, state::Tool};
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let item = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(item.rect[0], item.rect[1]), vec2(item.rect[2], item.rect[3]))
}
fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for step in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}
fn position(h: &Harness<'_, EffectcraftApp>, layer: u64) -> [f64; 3] {
    h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/position").unwrap().value.as_vec3()
}
fn settle_native(h: &mut Harness<'_, EffectcraftApp>) {
    for _ in 0..500 {
        h.step();
        if h.state().frames.inflight() == 0 && h.state().frames.last_ms.lock().is_ok_and(|ms| *ms > 0.0) {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the native edge-on cube's viewer frame did not finish");
}
fn visible_body(h: &mut Harness<'_, EffectcraftApp>) -> (Rect, Pos2) {
    let frame = rect(h, "viewer.comp");
    let pixels = h.state_mut().viewer_pixels().expect("actual native cube viewer pixels");
    let is_red = |color: &egui::Color32| color.r() > 40 && color.r() > color.g().saturating_add(20) && color.r() > color.b().saturating_add(20);
    let mut lo = [usize::MAX; 2];
    let mut hi = [0; 2];
    let mut count = 0;
    for (i, color) in pixels.pixels.iter().enumerate() {
        if is_red(color) {
            let p = [i % pixels.size[0], i / pixels.size[0]];
            for axis in 0..2 {
                lo[axis] = lo[axis].min(p[axis]);
                hi[axis] = hi[axis].max(p[axis]);
            }
            count += 1;
        }
    }
    assert!(count > 100 && hi[0] > lo[0] && hi[1] > lo[1], "a real edge-on volume must remain visibly rendered before testing body picking");
    let center = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2];
    assert!(is_red(&pixels.pixels[center[1] * pixels.size[0] + center[0]]), "the exact pointer target must be inside actually painted cube pixels");
    let at = |p: [usize; 2]| {
        frame.min + vec2((p[0] as f32 + 0.5) * frame.width() / pixels.size[0] as f32, (p[1] as f32 + 0.5) * frame.height() / pixels.size[1] as f32)
    };
    (Rect::from_min_max(at(lo), at(hi)), at(center))
}

#[test]
fn a_visible_native_cube_at_rotation_y_90_supports_body_pick_and_move() {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Edge-on volume picking","width":320,"height":240,"duration":0.04,"frameRate":25})).unwrap();
    let layer = session
        .execute_checked(
            "layer.newPrimitive",
            json!({"kind":"cube","width":80,"height":80,"depth":80,"position":[160,120,0],"baseColor":[1,0.1,0.05],"emissive":[1,0,0],"acceptsLights":"off"}),
        )
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/rotationY","value":90})).unwrap();
    session.state.snapping = false;
    session.history.clear();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(move |_| EffectcraftApp::new(session));
    h.state_mut().ui.tool = Tool::Selection;
    settle_native(&mut h);
    assert!(h.state().gpu_adapter().is_some() && h.state().viewer_on_gpu(), "the diagnostic requires the actual native GPU viewer");
    let owner = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap();
    assert!(owner.source.is_model() && owner.is_3d() && !owner.switches.locked);
    assert_eq!(owner.props.prop("transform/rotationY").unwrap().value.as_f64(), 90.0);
    let (painted, body) = visible_body(&mut h);

    // First exercise picking from a genuinely empty UI selection.
    h.state_mut().session.execute_checked("edit.deselectAll", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().session.state.selected_layers.is_empty());
    click(&mut h, body);
    let picked = h.state().session.state.selected_layers == vec![LayerId(layer)];

    // Retained selection is a separate movement boundary: force it even if picking failed,
    // so the diagnostic records both behaviors rather than stopping at the first assertion.
    h.state_mut().session.execute_checked("layer.select", json!({"layers":[layer]})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.state.selected_layers, vec![LayerId(layer)]);
    h.state_mut().session.history.clear();
    let before = position(&h, layer);
    drag(&mut h, body, body + vec2(20.0, 10.0));
    let after = position(&h, layer);
    let moved = after[0] > before[0] && after[1] > before[1] && after[2] == before[2];
    let undo_steps = h.state().session.history.undo.len();
    eprintln!(
        "edge-on cube body diagnostic: adapter={:?}; painted={painted:?}; body={body:?}; picked={picked}; position={before:?}->{after:?}; undo_steps={undo_steps}",
        h.state().gpu_adapter()
    );
    assert!(
        picked && moved,
        "the visible edge-on volume must be body-picked and moved independently of a singular XY scale plane: picked={picked}, position={before:?}->{after:?}"
    );
    assert_eq!(undo_steps, 1, "one body drag must produce one undo step");
    assert!(h.state_mut().session.undo());
    assert_eq!(position(&h, layer), before);
}

fn edge_on_cube(rotation: f64, scale: [f64; 3]) -> (Harness<'static, EffectcraftApp>, u64) {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Edge-on volume controls","width":320,"height":240,"duration":0.04,"frameRate":25})).unwrap();
    let layer = session
        .execute_checked(
            "layer.newPrimitive",
            json!({"kind":"cube","width":80,"height":80,"depth":80,"position":[160,120,0],"baseColor":[1,0.1,0.05],"emissive":[1,0,0],"acceptsLights":"off"}),
        )
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/rotationY","value":90})).unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/rotation","value":rotation})).unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/scale","value":scale})).unwrap();
    session.state.snapping = false;
    session.history.clear();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(move |_| EffectcraftApp::new(session));
    h.state_mut().ui.tool = Tool::Selection;
    settle_native(&mut h);
    assert!(h.state().gpu_adapter().is_some() && h.state().viewer_on_gpu(), "edge-on controls require actual native GPU rendering");
    let _ = visible_body(&mut h);
    (h, layer)
}
fn body_pick_and_move(h: &mut Harness<'_, EffectcraftApp>, layer: u64) {
    let (_, body) = visible_body(h);
    h.state_mut().session.execute_checked("edit.deselectAll", json!({})).unwrap();
    h.run_steps(3);
    click(h, body);
    assert_eq!(h.state().session.state.selected_layers, vec![LayerId(layer)]);
    h.state_mut().session.history.clear();
    let before = position(h, layer);
    drag(h, body, body + vec2(20.0, 10.0));
    let after = position(h, layer);
    assert!(after[0] > before[0] && after[1] > before[1]);
    assert_eq!(after[2], before[2]);
    assert_eq!(h.state().session.history.undo.len(), 1);
    assert!(h.state_mut().session.undo());
    assert_eq!(position(h, layer), before);
}
#[test]
fn a_visible_edge_on_cube_keeps_its_anchor_without_invalid_scale_handles() {
    let (h, layer) = edge_on_cube(0.0, [100.0; 3]);
    assert!(h.state().auto.find(&format!("viewer.anchor.{layer}")).is_some(), "the real projected anchor remains visible");
    for handle in 0..8 {
        assert!(
            h.state().auto.find(&format!("viewer.handle.{layer}.{handle}")).is_none(),
            "a singular layer plane must not expose the invalid Scale handle {handle}"
        );
    }
}
#[test]
fn a_tilted_edge_on_cube_supports_body_pick_move_and_undo() {
    let (mut h, layer) = edge_on_cube(25.0, [100.0; 3]);
    body_pick_and_move(&mut h, layer);
}
#[test]
fn a_mirrored_edge_on_cube_supports_body_pick_move_and_undo() {
    let (mut h, layer) = edge_on_cube(0.0, [-100.0, 125.0, 100.0]);
    body_pick_and_move(&mut h, layer);
    assert_eq!(
        h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/scale").unwrap().value.as_vec3(),
        [-100.0, 125.0, 100.0]
    );
}
#[test]
fn an_edge_on_cube_rotates_around_its_original_projected_anchor() {
    let (mut h, layer) = edge_on_cube(0.0, [100.0; 3]);
    h.state_mut().ui.tool = Tool::Rotate;
    h.run_steps(2);
    let center = rect(&h, &format!("viewer.anchor.{layer}")).center();
    let (painted, _) = visible_body(&mut h);
    let from = center + vec2(20.0, 0.0);
    let to = center + vec2(14.0, 14.0);
    assert!(painted.contains(from) && painted.contains(to), "the Rotate gesture remains inside the visibly painted source");
    drag(&mut h, from, to);
    let rotation = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/rotation").unwrap().value.as_f64();
    assert!(rotation > 20.0 && rotation < 70.0, "Rotate must use the preserved original anchor: {rotation}");
    assert_eq!(h.state().session.history.undo.len(), 1);
    assert!(h.state_mut().session.undo());
    assert_eq!(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/rotation").unwrap().value.as_f64(), 0.0);
}

#[test]
fn pan_behind_does_not_edit_an_edge_on_cubes_invalid_local_plane() {
    let (mut h, layer) = edge_on_cube(0.0, [100.0; 3]);
    h.state_mut().ui.tool = Tool::PanBehind;
    h.run_steps(2);
    let (_, body) = visible_body(&mut h);
    let before = position(&h, layer);
    let anchor = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/anchor").unwrap().value.as_vec3();
    drag(&mut h, body, body + vec2(20.0, 10.0));
    assert_eq!(position(&h, layer), before);
    assert_eq!(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/anchor").unwrap().value.as_vec3(), anchor);
    assert!(h.state().session.history.undo.is_empty(), "an invalid local plane must not create a synthetic Anchor edit");
}
