//! Pointer regressions for viewer resize cursors and existing transform gestures.

use effectcraft_engine::{Session, project::LayerId};
use effectcraft_ui_egui::{EffectcraftApp, state::Tool};
use egui::{CursorIcon, Event, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let item = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(item.rect[0], item.rect[1]), vec2(item.rect[2], item.rect[3]))
}
fn hover(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) -> CursorIcon {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    h.output().platform_output.cursor_icon
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
fn value(h: &Harness<'_, EffectcraftApp>, layer: u64, property: &str) -> effectcraft_engine::keyframe::Value {
    h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop(property).unwrap().value.clone()
}
fn card(three_d: bool, rotation: f64) -> (Harness<'static, EffectcraftApp>, u64) {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Transform controls","width":640,"height":360,"duration":2,"frameRate":30})).unwrap();
    let layer = session.execute_checked("layer.newSolid", json!({"name":"Card","width":100,"height":80,"color":"#c05030"})).unwrap()["layer"].as_u64().unwrap();
    if three_d {
        session.execute_checked("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
    }
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/rotation","value":rotation})).unwrap();
    session.state.snapping = false;
    session.history.clear();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(session));
    h.run_steps(4);
    assert!(h.state().auto.find(&format!("viewer.handle.{layer}.0")).is_some(), "a visible card provides the control case");
    (h, layer)
}
fn screen(h: &Harness<'_, EffectcraftApp>, point: [f32; 2]) -> Pos2 {
    let frame = rect(h, "viewer.comp");
    let comp = h.state().session.active_comp().unwrap();
    frame.min + vec2(point[0] * frame.width() / comp.width as f32, point[1] * frame.height() / comp.height as f32)
}
#[test]
fn viewer_resize_handles_use_their_horizontal_vertical_and_diagonal_directions() {
    let (mut h, layer) = card(false, 0.0);
    let expected = [
        CursorIcon::ResizeNwSe,
        CursorIcon::ResizeNeSw,
        CursorIcon::ResizeNwSe,
        CursorIcon::ResizeNeSw,
        CursorIcon::ResizeVertical,
        CursorIcon::ResizeHorizontal,
        CursorIcon::ResizeVertical,
        CursorIcon::ResizeHorizontal,
    ];
    let mut measured = Vec::new();
    for (handle, expected) in expected.into_iter().enumerate() {
        let at = rect(&h, &format!("viewer.handle.{layer}.{handle}")).center();
        measured.push((handle, hover(&mut h, at), expected));
    }
    eprintln!("handle cursor measurements: {measured:?}");
    assert_eq!(measured[0].1, measured[0].2, "the existing northwest diagonal is a positive control");
    assert_eq!(measured[2].1, measured[2].2, "the existing southeast diagonal is a positive control");
    let wrong: Vec<_> = measured.into_iter().filter(|(_, actual, expected)| actual != expected).collect();
    assert!(wrong.is_empty(), "each side/corner resize cursor must reflect its actual drag direction: {wrong:?}");
}
#[test]
fn rotated_viewer_resize_handles_follow_their_screen_directions() {
    let (mut h, layer) = card(false, 90.0);
    let expected = [
        CursorIcon::ResizeNeSw,
        CursorIcon::ResizeNwSe,
        CursorIcon::ResizeNeSw,
        CursorIcon::ResizeNwSe,
        CursorIcon::ResizeHorizontal,
        CursorIcon::ResizeVertical,
        CursorIcon::ResizeHorizontal,
        CursorIcon::ResizeVertical,
    ];
    let mut wrong = Vec::new();
    for (handle, expected) in expected.into_iter().enumerate() {
        let at = rect(&h, &format!("viewer.handle.{layer}.{handle}")).center();
        let actual = hover(&mut h, at);
        if actual != expected {
            wrong.push((handle, actual, expected));
        }
    }
    assert!(wrong.is_empty(), "rotation changes the screen direction of a handle: {wrong:?}");
}
#[test]
fn selection_tool_already_moves_an_unlocked_3d_card_in_the_view_plane() {
    let (mut h, layer) = card(true, 0.0);
    h.state_mut().ui.tool = Tool::Selection;
    let before = value(&h, layer, "transform/position").as_vec3();
    let from = screen(&h, [320.0, 180.0]);
    let to = from + vec2(24.0, 12.0);
    drag(&mut h, from, to);
    let moved = value(&h, layer, "transform/position").as_vec3();
    assert!(moved[0] > before[0] && moved[1] > before[1], "existing 3D card movement is supported: {before:?} -> {moved:?}");
    assert_eq!(moved[2], before[2]);
    assert_eq!(h.state().session.history.undo.len(), 1);
}
#[test]
fn rotation_tool_already_rotates_an_unlocked_3d_card() {
    let (mut h, layer) = card(true, 0.0);
    h.state_mut().ui.tool = Tool::Rotate;
    h.run_steps(2);
    let center = rect(&h, &format!("viewer.anchor.{layer}")).center();
    let from = center + vec2(20.0, 0.0);
    let to = center + vec2(14.0, 14.0);
    drag(&mut h, from, to);
    let rotation = value(&h, layer, "transform/rotation").as_f64();
    assert!(rotation > 20.0 && rotation < 70.0, "the existing W Rotation Tool rotates a 3D card: {rotation}");
    assert_eq!(h.state().session.history.undo.len(), 1);
}
#[test]
fn selection_handle_already_scales_an_unlocked_3d_card_on_one_axis() {
    let (mut h, layer) = card(true, 0.0);
    h.state_mut().ui.tool = Tool::Selection;
    let from = rect(&h, &format!("viewer.handle.{layer}.5")).center();
    drag(&mut h, from, from + vec2(24.0, 0.0));
    let scale = value(&h, layer, "transform/scale").as_vec3();
    assert!(scale[0] > 100.0, "existing 3D card horizontal handle changes X: {scale:?}");
    assert!((scale[1] - 100.0).abs() < 1e-8);
    assert_eq!(scale[2], 100.0);
    assert_eq!(h.state().session.history.undo.len(), 1);
}
fn native_cube() -> (Harness<'static, EffectcraftApp>, u64) {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Native cube controls","width":320,"height":240,"duration":0.04,"frameRate":25})).unwrap();
    let layer = session
        .execute_checked(
            "layer.newPrimitive",
            json!({"kind":"cube","width":80,"height":80,"depth":80,"position":[160,120,0],"baseColor":[1,0.1,0.05],"emissive":[1,0,0],"acceptsLights":"off"}),
        )
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    session.state.snapping = false;
    session.history.clear();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(move |_| EffectcraftApp::new(session));
    settle_native(&mut h);
    assert!(h.state().gpu_adapter().is_some(), "native proving run requires a usable compositor GPU");
    assert!(h.state().viewer_on_gpu(), "the visible cube must use the actual native GPU viewer path");
    // Inspect actual viewer pixels before any expected interaction failure, excluding UI overlays.
    let _ = visible_cube_bounds(&mut h);
    let owner = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap();
    assert!(owner.source.is_model() && owner.is_3d());
    assert_eq!(h.state().session.state.selected_layers, vec![LayerId(layer)]);
    eprintln!("native proving adapter: {:?}", h.state().gpu_adapter());
    (h, layer)
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
    panic!("the native cube's actual viewer frame did not finish");
}
fn visible_cube_bounds(h: &mut Harness<'_, EffectcraftApp>) -> Rect {
    let frame = rect(h, "viewer.comp");
    let pixels = h.state_mut().viewer_pixels().expect("actual native cube viewer pixels");
    let mut lo = [usize::MAX; 2];
    let mut hi = [0; 2];
    let mut count = 0;
    for (i, color) in pixels.pixels.iter().enumerate() {
        if color.r() > 40 && color.r() > color.g().saturating_add(20) && color.r() > color.b().saturating_add(20) {
            let point = [i % pixels.size[0], i / pixels.size[0]];
            for axis in 0..2 {
                lo[axis] = lo[axis].min(point[axis]);
                hi[axis] = hi[axis].max(point[axis]);
            }
            count += 1;
        }
    }
    assert!(count > 100 && hi[0] > lo[0] && hi[1] > lo[1], "the fixture must actually render a nontrivial red cube before checking its controls");
    let at = |point: [usize; 2]| {
        frame.min + vec2((point[0] as f32 + 0.5) * frame.width() / pixels.size[0] as f32, (point[1] as f32 + 0.5) * frame.height() / pixels.size[1] as f32)
    };
    Rect::from_min_max(at(lo), at(hi))
}
#[test]
fn a_visible_native_3d_cube_has_the_existing_selection_resize_handles() {
    let (h, layer) = native_cube();
    for handle in 0..8 {
        assert!(
            h.state().auto.find(&format!("viewer.handle.{layer}.{handle}")).is_some(),
            "a visibly rendered selected 3D cube needs its existing resize handle {handle}"
        );
    }
}
#[test]
fn selection_tool_can_move_a_visible_native_3d_cube_by_its_body() {
    let (mut h, layer) = native_cube();
    h.state_mut().ui.tool = Tool::Selection;
    let from = visible_cube_bounds(&mut h).center();
    let before = value(&h, layer, "transform/position").as_vec3();
    drag(&mut h, from, from + vec2(20.0, 10.0));
    let moved = value(&h, layer, "transform/position").as_vec3();
    assert!(moved[0] > before[0] && moved[1] > before[1], "existing Selection body drag must move a visibly rendered cube: {before:?} -> {moved:?}");
    assert_eq!(h.state().session.history.undo.len(), 1);
    assert!(h.state_mut().session.undo());
    assert_eq!(value(&h, layer, "transform/position").as_vec3(), before);
}
#[test]
fn rotation_tool_can_rotate_a_visible_native_3d_cube_by_its_body() {
    let (mut h, layer) = native_cube();
    h.state_mut().ui.tool = Tool::Rotate;
    h.run_steps(2);
    let center = visible_cube_bounds(&mut h).center();
    drag(&mut h, center + vec2(12.0, 0.0), center + vec2(9.0, 9.0));
    let rotation = value(&h, layer, "transform/rotation").as_f64();
    assert!(rotation > 15.0 && rotation < 75.0, "existing Rotation body drag must rotate a visibly rendered cube: {rotation}");
    assert_eq!(h.state().session.history.undo.len(), 1);
}
#[test]
fn selection_tool_can_scale_a_visible_native_3d_cube_on_one_axis() {
    let (mut h, layer) = native_cube();
    h.state_mut().ui.tool = Tool::Selection;
    let painted = visible_cube_bounds(&mut h);
    let from = h
        .state()
        .auto
        .find(&format!("viewer.handle.{layer}.5"))
        .map(|handle| pos2(handle.rect[0] + handle.rect[2] / 2.0, handle.rect[1] + handle.rect[3] / 2.0))
        .unwrap_or_else(|| painted.right_center());
    drag(&mut h, from, from + vec2(24.0, 0.0));
    let scale = value(&h, layer, "transform/scale").as_vec3();
    assert!(scale[0] > 100.0, "existing X resize interaction must scale the visible cube: {scale:?}");
    assert!((scale[1] - 100.0).abs() < 1e-8);
    assert_eq!(scale[2], 100.0);
    assert_eq!(h.state().session.history.undo.len(), 1);
}

struct ModelSource(std::sync::Arc<effectcraft_model::Model>);
impl effectcraft_engine::render::FootageSource for ModelSource {
    fn frame(
        &self,
        _: effectcraft_engine::project::ItemId,
        _: &effectcraft_engine::project::Footage,
        _: effectcraft_engine::time::Tick,
    ) -> Option<std::sync::Arc<effectcraft_engine::render::Image>> {
        None
    }
    fn model(&self, _: effectcraft_engine::project::ItemId, _: &effectcraft_engine::project::Footage) -> Option<std::sync::Arc<effectcraft_model::Model>> {
        Some(self.0.clone())
    }
}
fn native_model(animated: bool) -> (Harness<'static, EffectcraftApp>, u64) {
    use effectcraft_engine::project::{Footage, FootageKind, ItemKind};
    let mut material = effectcraft_model::Material::plain([1.0, 0.1, 0.05, 1.0], 0.0, 0.5);
    material.emissive = [1.0, 0.0, 0.0];
    let mut model = effectcraft_model::prim::model(
        effectcraft_model::prim::PrimitiveKind::Cube,
        &effectcraft_model::prim::PrimParams { size: [40.0; 3], ..Default::default() },
        material,
    );
    model.nodes[0].translation = [20.0, 10.0, 5.0];
    if animated {
        model.animations.push(effectcraft_model::Animation {
            name: "Translate".into(),
            channels: vec![effectcraft_model::Channel {
                node: 0,
                path: effectcraft_model::ChannelPath::Translation,
                interp: effectcraft_model::Interp::Linear,
                times: vec![0.0, 1.0],
                values: vec![20.0, 10.0, 5.0, 40.0, 10.0, 5.0],
            }],
        });
    }
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Model controls","width":320,"height":240,"duration":2,"frameRate":25})).unwrap();
    let item = std::sync::Arc::make_mut(&mut session.project).add_item(
        "Synthetic parsed model",
        Default::default(),
        None,
        ItemKind::Footage(Footage { kind: FootageKind::Model, has_video: true, ..Default::default() }),
    );
    session.footage = std::sync::Arc::new(ModelSource(std::sync::Arc::new(model)));
    let layer = session.execute_checked("layer.newModel", json!({"item":item.0,"position":[160,120,0]})).unwrap()["layer"].as_u64().unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"geometryOptions/unitScale","value":2})).unwrap();
    if animated {
        session.execute_checked("prop.set", json!({"layer":layer,"path":"geometryOptions/animation","value":1})).unwrap();
    }
    session.state.snapping = false;
    session.history.clear();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(move |_| EffectcraftApp::new(session));
    settle_native(&mut h);
    assert!(h.state().gpu_adapter().is_some() && h.state().viewer_on_gpu(), "model controls require actual native GPU rendering");
    let _ = visible_cube_bounds(&mut h);
    (h, layer)
}
fn model_box(h: &Harness<'_, EffectcraftApp>, layer: u64) -> Rect {
    Rect::from_min_max(rect(h, &format!("viewer.handle.{layer}.0")).center(), rect(h, &format!("viewer.handle.{layer}.2")).center())
}
#[test]
fn native_imported_model_handles_include_node_transforms_unit_scale_and_depth() {
    let (mut h, layer) = native_model(false);
    let painted = visible_cube_bounds(&mut h);
    let controls = model_box(&h, layer);
    assert!(
        controls.expand(2.0).contains_rect(painted),
        "the projected controls must contain the actual transformed model pixels: {controls:?} vs {painted:?}"
    );
    let center = screen(&h, [160.0, 120.0]);
    assert!(
        controls.center().x > center.x + 20.0 && controls.center().y < center.y - 10.0,
        "Y-up node coordinates and unitScale move the box away from the layer origin"
    );
    let before = value(&h, layer, "transform/position").as_vec3();
    drag(&mut h, painted.center(), painted.center() + vec2(20.0, 10.0));
    let moved = value(&h, layer, "transform/position").as_vec3();
    assert!(moved[0] > before[0] && moved[1] > before[1]);
    assert_eq!(h.state().session.history.undo.len(), 1);
    assert!(h.state_mut().session.undo());
    assert_eq!(value(&h, layer, "transform/position").as_vec3(), before);
}
#[test]
fn native_imported_model_handles_follow_the_current_animation_pose() {
    let (mut h, layer) = native_model(true);
    let before = model_box(&h, layer);
    h.state_mut().session.execute_checked("time.set", json!({"time":0.48})).unwrap();
    settle_native(&mut h);
    let painted = visible_cube_bounds(&mut h);
    let current = model_box(&h, layer);
    assert!(current.center().x > before.center().x + 10.0, "the pose cache must follow the clip time: {before:?} -> {current:?}");
    assert!(current.expand(2.0).contains_rect(painted), "animated handles must contain current rendered pixels: {current:?} vs {painted:?}");
    h.state_mut().session.execute_checked("time.set", json!({"time":0.0})).unwrap();
    settle_native(&mut h);
    let restored = model_box(&h, layer);
    assert!((restored.center().x - before.center().x).abs() < 1e-4, "a previously cached pose remains correct");
    assert_eq!(h.state().session.history.undo.len(), 0);
}

#[test]
fn rotation_tool_uses_the_projected_anchor_of_an_off_center_3d_card() {
    let (mut h, layer) = card(true, 0.0);
    h.state_mut().session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[420,210,400]})).unwrap();
    h.state_mut().session.history.clear();
    h.state_mut().ui.tool = Tool::Rotate;
    h.run_steps(4);
    let center = rect(&h, &format!("viewer.anchor.{layer}")).center();
    let raw = screen(&h, [420.0, 210.0]);
    assert!(center.distance(raw) > 20.0, "the fixture must distinguish actual projected anchor from raw local position: {center:?} vs {raw:?}");
    drag(&mut h, center + vec2(20.0, 0.0), center + vec2(14.0, 14.0));
    let rotation = value(&h, layer, "transform/rotation").as_f64();
    assert!(rotation > 20.0 && rotation < 70.0, "a body drag around the projected anchor must rotate a 3D card around that same anchor: {rotation}");
    assert_eq!(h.state().session.history.undo.len(), 1);
}
