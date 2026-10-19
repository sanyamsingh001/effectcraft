//! Layer locks protect interactive Timeline, Graph Editor and viewer edits.
use effectcraft_engine::{Session, project::LayerId, time::Tick};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind, state::Tool};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

fn harness() -> (Harness<'static, EffectcraftApp>, LayerId, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Locks","width":640,"height":360,"duration":4,"frameRate":30})).unwrap();
    s.execute("layer.newSolid", json!({"name":"Box","width":80,"height":80,"color":"#e04020"})).unwrap();
    let id = s.active_comp().unwrap().layers[0].id;
    for (time, value) in [(0.0, 0.0), (1.0, 100.0)] {
        s.execute("prop.addKey", json!({"layer":id.0,"path":"transform/opacity","time":time,"value":value})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute("prop.select", json!({"layer":id.0,"prop":uid})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.opacity", json!({})).unwrap();
    h.run_steps(3);
    (h, id, uid)
}
fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}
fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
}
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}
fn lock(h: &mut Harness<'_, EffectcraftApp>, id: LayerId) {
    let at = rect(h, &format!("timeline.layer.{}.lock", id.0)).center();
    click(h, at);
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap().switches.locked);
}
fn graph(h: &mut Harness<'_, EffectcraftApp>) {
    h.state_mut().ui.maximized = Some(PanelKind::Timeline);
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.graph_snap = false;
    h.state_mut().ui.timeline.graph_auto_zoom = false;
    h.state_mut().ui.timeline.graph_range = Some((-40.0, 140.0));
    h.run_steps(3);
}
fn mixed_graph() -> (Harness<'static, EffectcraftApp>, LayerId, u64, LayerId, u64) {
    let (mut h, locked, uid) = harness();
    let editable =
        LayerId(h.state_mut().session.execute("layer.newSolid", json!({"name":"Editable","width":80,"height":80})).unwrap()["layer"].as_u64().unwrap());
    for (time, value) in [(0.0, 25.0), (1.0, 75.0)] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":editable.0,"path":"transform/opacity","time":time,"value":value})).unwrap();
    }
    let eu = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys":[
        {"layer":editable.0,"prop":eu,"time":1},{"layer":locked.0,"prop":uid,"time":0},
        {"layer":editable.0,"prop":eu,"time":0},{"layer":locked.0,"prop":uid,"time":1}
    ],"selectProperties":true}),
        )
        .unwrap();
    graph(&mut h);
    (h, locked, uid, editable, eu)
}
fn key_event(h: &mut Harness<'_, EffectcraftApp>, key: egui::Key, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
    }
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}
fn copied(h: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    h.output().platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None }).collect()
}
#[test]
fn cut_shortcut_preserves_locked_graph_keys_and_clipboard() {
    let (mut h, id, _) = harness();
    graph(&mut h);
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    assert_eq!(selected.len(), 2);
    let history = h.state().session.history.undo.len();
    h.event(Event::Cut);
    h.step();
    let notes = copied(&h);
    h.run_steps(2);
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "Cut preserves locked graph keys");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history);
    assert!(h.state().session.state.key_clipboard.is_empty(), "a protected Cut preserves the internal clipboard");
    assert!(notes.is_empty(), "a protected Cut does not overwrite the system clipboard");
}
#[test]
fn mixed_cut_copies_and_deletes_only_editable_graph_keys() {
    let (mut h, id, _, editable, eu) = mixed_graph();
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected: Vec<_> = h.state().session.state.selected_keys.iter().copied().filter(|k| k.layer == id).collect();
    let project = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    h.event(Event::Cut);
    h.step();
    let notes = copied(&h);
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    assert!(comp.layer(editable).unwrap().props.find(eu).unwrap().keys.is_empty(), "mixed Cut deletes editable keys");
    assert!(comp.layer(id).unwrap() == &before, "mixed Cut preserves locked animation");
    assert_eq!(h.state().session.state.selected_keys, selected);
    let clips = &h.state().session.state.key_clipboard;
    assert_eq!(clips.iter().map(|c| c.keys.len()).sum::<usize>(), 2, "only deleted keys are copied");
    assert!(clips.iter().all(|c| c.layer == editable), "the clipboard excludes protected layers");
    assert_eq!(notes, ["EffectCraft: 2 keyframes"]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
#[test]
fn hold_shortcut_preserves_locked_graph_keys() {
    let (mut h, id, _) = harness();
    graph(&mut h);
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    key_event(&mut h, egui::Key::H, Modifiers { alt: true, ..Modifiers::COMMAND });
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "Toggle Hold preserves locked keys");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn mixed_hold_shortcut_edits_only_editable_keys_in_original_order() {
    let (mut h, id, _, editable, eu) = mixed_graph();
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    let project = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    key_event(&mut h, egui::Key::H, Modifiers { alt: true, ..Modifiers::COMMAND });
    let comp = h.state().session.active_comp().unwrap();
    assert!(
        comp.layer(editable).unwrap().props.find(eu).unwrap().keys.iter().all(|k| k.out_interp == effectcraft_engine::keyframe::Interp::Hold),
        "unlocked selected keys become Hold"
    );
    assert!(comp.layer(id).unwrap() == &before, "mixed Hold preserves locked animation");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
#[test]
fn timeline_value_scrub_respects_the_layer_lock() {
    let (mut h, id, uid) = harness();
    let field = format!("timeline.prop.{uid}.value");
    let from = rect(&h, &field).center();
    let to = from + vec2(45.0, 0.0);
    let before = h.state().session.project.clone();
    drag(&mut h, from, to);
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked value can be scrubbed");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    let from = rect(&h, &field).center();
    drag(&mut h, from, from + vec2(45.0, 0.0));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "scrubbing a locked Timeline value preserves the property");
    assert_eq!(h.state().session.history.undo.len(), history);
}
fn assert_graph_owner_lock(handle: bool) {
    let (mut h, id, uid, editable, _) = mixed_graph();
    if handle {
        h.state_mut().session.execute("keys.easyEase", json!({})).unwrap();
        h.run_steps(3);
    }
    let widget = if handle { format!("timeline.graph.handle.{uid}.0.0.out") } else { format!("timeline.graph.key.{uid}.0.1") };
    let from = rect(&h, &widget).center();
    let to = from + vec2(40.0, -25.0);
    let before = h.state().session.project.clone();
    drag(&mut h, from, to);
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked owner initiates a group edit");
    assert!(h.state_mut().session.undo());
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys":[
        {"layer":editable.0,"path":"transform/opacity","time":1},{"layer":id.0,"prop":uid,"time":0},
        {"layer":editable.0,"path":"transform/opacity","time":0},{"layer":id.0,"prop":uid,"time":1}
    ],"selectProperties":true}),
        )
        .unwrap();
    h.run_steps(3);
    lock(&mut h, id);
    let before = h.state().session.project.clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    let from = rect(&h, &widget).center();
    drag(&mut h, from, from + vec2(40.0, -25.0));
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &before), "a locked Graph owner cannot begin a group edit");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn graph_key_drag_cannot_start_on_a_locked_owner() {
    assert_graph_owner_lock(false);
}
#[test]
fn graph_handle_drag_cannot_start_on_a_locked_owner() {
    assert_graph_owner_lock(true);
}

fn viewer_motion() -> (Harness<'static, EffectcraftApp>, LayerId) {
    let (mut h, id, _) = harness();
    for (time, value) in [(0.0, [100, 100, 0]), (2.0, [500, 100, 0])] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":id.0,"path":"transform/position","time":time,"value":value})).unwrap();
    }
    h.state_mut().session.execute("view.snapping", json!({"value":false})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers":[id.0]})).unwrap();
    h.state_mut().session.set_time(Tick::from_seconds_f64(1.0));
    h.state_mut().ui.tool = Tool::Selection;
    h.run_steps(3);
    (h, id)
}
fn assert_viewer_motion_lock(tangent: bool) {
    let (mut h, id) = viewer_motion();
    if tangent {
        h.state_mut()
            .session
            .execute("keys.setSpatialTangents", json!({"layer": id.0, "path": "transform/position", "time": 2.0, "in": [-70.0, 0.0, 0.0], "break": true}))
            .unwrap();
        h.run_steps(3);
    }
    let widget = if tangent { format!("viewer.motionPath.{}.1.in", id.0) } else { format!("viewer.motionPath.{}.1", id.0) };
    let from = rect(&h, &widget).center();
    let before = h.state().session.project.clone();
    drag(&mut h, from, from + vec2(0.0, 70.0));
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked viewer handle edits");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    lock(&mut h, id);
    assert_eq!(h.state().session.state.selected_layers, [id], "locking retains the selected layer");
    let from = rect(&h, &widget).center();
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    drag(&mut h, from, from + vec2(0.0, 70.0));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "the viewer gesture preserves its locked owner");
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn viewer_motion_key_drag_respects_the_layer_lock() {
    assert_viewer_motion_lock(false);
}
#[test]
fn viewer_spatial_tangent_drag_respects_the_layer_lock() {
    assert_viewer_motion_lock(true);
}
#[test]
fn viewer_resize_handle_respects_the_layer_lock() {
    let (mut h, id, _) = harness();
    h.state_mut().session.execute("view.snapping", json!({"value":false})).unwrap();
    h.run_steps(3);
    let widget = format!("viewer.handle.{}.2", id.0);
    let from = rect(&h, &widget).center();
    let before = h.state().session.project.clone();
    drag(&mut h, from, from + vec2(45.0, 45.0));
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked resize handle edits");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    lock(&mut h, id);
    let from = rect(&h, &widget).center();
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    drag(&mut h, from, from + vec2(45.0, 45.0));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "the viewer resize preserves its locked owner");
    assert_eq!(h.state().session.history.undo.len(), history);
}
fn viewer_mask() -> (Harness<'static, EffectcraftApp>, LayerId, u64) {
    let (mut h, id, _) = harness();
    let mask =
        h.state_mut().session.execute("mask.new", json!({"layer":id.0,"vertices":[[20,20],[60,20],[50,60]],"closed":true})).unwrap()["mask"].as_u64().unwrap();
    h.state_mut().session.execute("view.snapping", json!({"value":false})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers":[id.0]})).unwrap();
    h.state_mut().ui.tool = Tool::Selection;
    h.run_steps(3);
    (h, id, mask)
}
#[test]
fn viewer_mask_vertex_drag_respects_the_layer_lock() {
    let (mut h, id, mask) = viewer_mask();
    let widget = format!("viewer.mask.{mask}.vertex.0");
    let from = rect(&h, &widget).center();
    let before = h.state().session.project.clone();
    drag(&mut h, from, from + vec2(20.0, 30.0));
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked path vertex edits");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    lock(&mut h, id);
    let from = rect(&h, &widget).center();
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    drag(&mut h, from, from + vec2(20.0, 30.0));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "the locked path vertex stays unchanged");
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn pen_add_vertex_respects_the_layer_lock() {
    let (mut h, id, mask) = viewer_mask();
    h.state_mut().ui.tool = Tool::PenAdd;
    h.run_steps(3);
    let a = rect(&h, &format!("viewer.mask.{mask}.vertex.0")).center();
    let b = rect(&h, &format!("viewer.mask.{mask}.vertex.1")).center();
    let at = a + (b - a) * 0.5;
    let before = h.state().session.project.clone();
    click(&mut h, at);
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &before), "the unlocked path accepts a vertex");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    lock(&mut h, id);
    let a = rect(&h, &format!("viewer.mask.{mask}.vertex.0")).center();
    let b = rect(&h, &format!("viewer.mask.{mask}.vertex.1")).center();
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    click(&mut h, a + (b - a) * 0.5);
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "the locked path does not gain a vertex");
    assert_eq!(h.state().session.history.undo.len(), history);
}

#[test]
fn spatial_tangent_drag_keeps_the_roving_key_across_pointer_frames() {
    let (mut h, id, _) = harness();
    for (time, value) in [(0.0, [100, 180, 0]), (1.0, [190, 180, 0]), (2.0, [200, 180, 0])] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":id.0,"path":"transform/position","time":time,"value":value})).unwrap();
    }
    h.state_mut().session.execute("keys.select", json!({"keys":[{"layer":id.0,"path":"transform/position","time":1}]})).unwrap();
    h.state_mut().session.execute("keys.interpolation", json!({"roving":true})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers":[id.0]})).unwrap();
    h.state_mut().session.execute("view.snapping", json!({"value":false})).unwrap();
    h.state_mut().session.set_time(Tick::from_seconds_f64(1.0));
    h.run_steps(3);
    let prop = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap().clone();
    assert!(prop.keys[1].roving);
    assert!(prop.keys[1].time.seconds() > 1.0);
    let last = prop.keys[2].clone();
    let project = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    let from = rect(&h, &format!("viewer.motionPath.{}.1.out", id.0)).center();
    let comp = rect(&h, "viewer.comp");
    let z = comp.width() / 640.0;
    let to = comp.min + vec2(190.0 * z, 10180.0 * z);
    let next = comp.min + vec2(190.0 * z, 11180.0 * z);
    let modifiers = Modifiers::ALT;
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    let first = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap().clone();
    eprintln!("roving tangent first target time {} -> {}", prop.keys[1].time.seconds(), first.keys[1].time.seconds());
    assert!(first.keys[1].time.seconds() < 0.5, "the initial tangent pull crosses the old nearest-key boundary");
    assert_eq!(first.keys[2], last, "the first pointer movement edits the intended middle key");
    h.event(Event::PointerMoved(next));
    h.step();
    h.event(Event::PointerButton { pos: next, button: PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let after = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap();
    eprintln!("roving last tangent before={:?};after={:?}", last.spatial_out, after.keys[2].spatial_out);
    assert_eq!(after.keys[2], last, "later pointer frames keep editing the same roving key");
    assert!((after.keys[1].spatial_out[1] - 11000.0).abs() < 2.0);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}

fn ui_lock(h: &mut Harness<'_, EffectcraftApp>, id: LayerId) {
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.setSwitch", json!({"layers":[id.0],"switch":"lock","value":true})).unwrap();
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap().switches.locked);
}
fn assert_pending_value_lock(enter: bool) {
    let (mut h, id, uid) = harness();
    h.state_mut().ui.maximized = Some(PanelKind::Timeline);
    h.run_steps(3);
    let at = rect(&h, &format!("timeline.prop.{uid}.value")).center();
    click(&mut h, at);
    h.event(Event::Text("35".into()));
    h.step();
    assert_eq!(
        h.state().session.active_comp().unwrap().layer(id).unwrap().props.find(uid).unwrap().keys[0].value.as_f64(),
        0.0,
        "typing stays in the pending inline editor"
    );
    ui_lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    if enter {
        key_event(&mut h, egui::Key::Enter, Modifiers::NONE);
    } else {
        let row = rect(&h, &format!("timeline.layer.{}.row", id.0));
        click(&mut h, pos2(row.min.x + 210.0, row.center().y));
    }
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "an existing inline editor cannot commit after its layer locks");
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn pending_timeline_value_editor_cannot_commit_after_lock_on_enter() {
    assert_pending_value_lock(true);
}
#[test]
fn pending_timeline_value_editor_cannot_commit_after_lock_on_clickaway() {
    assert_pending_value_lock(false);
}
#[test]
fn captured_viewer_motion_key_drag_stops_when_its_owner_locks() {
    let (mut h, id) = viewer_motion();
    let from = rect(&h, &format!("viewer.motionPath.{}.1", id.0)).center();
    let project = h.state().session.project.clone();
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(from + vec2(0.0, 20.0)));
    h.step();
    assert!(!std::sync::Arc::ptr_eq(&h.state().session.project, &project), "the gesture begins by editing the unlocked key");
    ui_lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    let to = from + vec2(0.0, 70.0);
    h.event(Event::PointerMoved(to));
    h.step();
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "captured viewer key drags stop editing a newly locked owner");
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn mixed_mask_vertex_drag_preserves_locked_selected_vertices() {
    let (mut h, locked, mask) = viewer_mask();
    let editable =
        LayerId(h.state_mut().session.execute("layer.newSolid", json!({"name":"Editable","width":80,"height":80})).unwrap()["layer"].as_u64().unwrap());
    let em = h.state_mut().session.execute("mask.new", json!({"layer":editable.0,"vertices":[[20,20],[60,20],[50,60]],"closed":true})).unwrap()["mask"]
        .as_u64()
        .unwrap();
    h.state_mut().session.execute("prop.set", json!({"layer":locked.0,"path":"transform/position","value":[200,180,0]})).unwrap();
    h.state_mut().session.execute("prop.set", json!({"layer":editable.0,"path":"transform/position","value":[440,180,0]})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers":[editable.0,locked.0]})).unwrap();
    h.state_mut()
        .session
        .execute("mask.selectVertices", json!({"vertices":[{"layer":editable.0,"mask":em,"index":0},{"layer":locked.0,"mask":mask,"index":0}]}))
        .unwrap();
    h.run_steps(3);
    lock(&mut h, locked);
    let comp = h.state().session.active_comp().unwrap();
    let before = comp.layer(locked).unwrap().clone();
    let edited = comp.layer(editable).unwrap().clone();
    let selected = h.state().session.state.selected_vertices.clone();
    let project = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    let from = rect(&h, &format!("viewer.mask.{em}.vertex.0")).center();
    drag(&mut h, from, from + vec2(20.0, 30.0));
    let comp = h.state().session.active_comp().unwrap();
    assert!(comp.layer(editable).unwrap() != &edited, "the unlocked selected vertex still moves");
    assert!(comp.layer(locked).unwrap() == &before, "the same drag preserves retained locked selected vertices");
    assert_eq!(h.state().session.state.selected_vertices, selected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
#[test]
fn a_pen_path_in_progress_cannot_continue_on_a_locked_owner() {
    let (mut h, id, _) = harness();
    h.state_mut().session.execute("view.snapping", json!({"value":false})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.run_steps(3);
    let comp = rect(&h, "viewer.comp");
    let z = comp.width() / 640.0;
    click(&mut h, comp.min + vec2(320.0 * z, 180.0 * z));
    let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
    let mask = layer.masks().unwrap().groups().next().unwrap();
    assert_eq!(mask.get("path").unwrap().value.as_path().unwrap().vertices.len(), 1, "the first click begins a path on the selected layer");
    ui_lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let history = h.state().session.history.undo.len();
    click(&mut h, comp.min + vec2(360.0 * z, 220.0 * z));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "the active Pen path cannot append to its newly locked owner");
    assert_eq!(h.state().session.history.undo.len(), history);
}
fn ease_panel(h: &mut Harness<'_, EffectcraftApp>) {
    h.state_mut().show_panel(PanelKind::EasePresets);
    h.state_mut().ui.maximized = Some(PanelKind::EasePresets);
    h.run_steps(3);
    let influence = rect(h, "easePresets.influence");
    let project = h.state().session.project.clone();
    click(h, pos2(influence.min.x + influence.width() * 0.6, influence.center().y));
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project), "the Influence slider only edits the working curve");
    assert!((h.state().ui.ease_presets.curve.out_influence - 60.0).abs() < 1.0);
}
#[test]
fn ease_presets_apply_preserves_retained_locked_pairs() {
    let (mut h, id, _) = harness();
    graph(&mut h);
    lock(&mut h, id);
    ease_panel(&mut h);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    let at = rect(&h, "easePresets.apply").center();
    click(&mut h, at);
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap() == &before, "Ease Presets Apply preserves locked pairs");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history);
}
#[test]
fn ease_presets_apply_edits_only_unlocked_pairs_in_original_selection_order() {
    let (mut h, id, _, editable, eu) = mixed_graph();
    lock(&mut h, id);
    ease_panel(&mut h);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    let project = h.state().session.project.clone();
    let at = rect(&h, "easePresets.apply").center();
    click(&mut h, at);
    let comp = h.state().session.active_comp().unwrap();
    assert_eq!(
        comp.layer(editable).unwrap().props.find(eu).unwrap().keys[0].out_interp,
        effectcraft_engine::keyframe::Interp::Bezier,
        "the editable selected pair is eased"
    );
    assert!(comp.layer(id).unwrap() == &before, "Ease Presets leaves the locked selected pair unchanged");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
#[test]
fn interpolation_dialog_preserves_locked_keys_in_mixed_selection() {
    let (mut h, id, _, editable, eu) = mixed_graph();
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    let project = h.state().session.project.clone();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "keys.interpolation", json!({})).unwrap();
    h.run_steps(3);
    let at = rect(&h, "dialog.interpolation.temporal").center();
    click(&mut h, at);
    h.run_steps(2);
    let hold = h.get_by_label("Hold").rect().center();
    click(&mut h, hold);
    let ok = rect(&h, "dialog.interpolation.ok").center();
    click(&mut h, ok);
    assert!(h.state().dialog.is_none());
    let comp = h.state().session.active_comp().unwrap();
    assert!(
        comp.layer(editable).unwrap().props.find(eu).unwrap().keys.iter().all(|k| k.out_interp == effectcraft_engine::keyframe::Interp::Hold),
        "the dialog edits the unlocked selected keys"
    );
    assert!(comp.layer(id).unwrap() == &before, "the interpolation dialog preserves its locked selected keys");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
#[test]
fn velocity_dialog_preserves_locked_keys_in_mixed_selection() {
    let (mut h, id, _, editable, eu) = mixed_graph();
    lock(&mut h, id);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    let edited = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.find(eu).unwrap().keys.clone();
    let selected = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    let project = h.state().session.project.clone();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "keys.velocity", json!({})).unwrap();
    h.run_steps(3);
    let field = rect(&h, "dialog.velocity.outSpeed.0").center();
    drag(&mut h, field, field + vec2(40.0, 0.0));
    let ok = rect(&h, "dialog.velocity.ok").center();
    click(&mut h, ok);
    assert!(h.state().dialog.is_none());
    let comp = h.state().session.active_comp().unwrap();
    assert!(comp.layer(editable).unwrap().props.find(eu).unwrap().keys != edited, "the dialog edits the unlocked selected keys");
    assert!(comp.layer(id).unwrap() == &before, "the velocity dialog preserves its locked selected keys");
    assert_eq!(h.state().session.state.selected_keys, selected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}

#[test]
fn roving_interpolation_dialog_keeps_mixed_selection_order_after_retiming() {
    let (mut h, locked, locked_uid, editable, _) = mixed_graph();
    for (time, value) in [(0.0, [0, 0, 0]), (1.0, [90, 0, 0]), (2.0, [100, 0, 0])] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":editable.0,"path":"transform/position","time":time,"value":value})).unwrap();
    }
    let uid = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/position").unwrap().uid;
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys":[
                {"layer":editable.0,"prop":uid,"time":1},
                {"layer":locked.0,"prop":locked_uid,"time":0},
                {"layer":editable.0,"prop":uid,"time":2},
                {"layer":locked.0,"prop":locked_uid,"time":1}
            ],"selectProperties":true}),
        )
        .unwrap();
    h.run_steps(3);
    lock(&mut h, locked);
    let protected = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    let selected = h.state().session.state.selected_keys.clone();
    assert_eq!(selected.iter().map(|key| key.layer).collect::<Vec<_>>(), [editable, locked, editable, locked]);
    let project = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "keys.interpolation", json!({})).unwrap();
    h.run_steps(3);
    let roving = rect(&h, "dialog.interpolation.roving").center();
    click(&mut h, roving);
    let across = h.get_by_label("Rove Across Time").rect().center();
    click(&mut h, across);
    let ok = rect(&h, "dialog.interpolation.ok").center();
    click(&mut h, ok);
    assert!(h.state().dialog.is_none());
    let comp = h.state().session.active_comp().unwrap();
    let keys = &comp.layer(editable).unwrap().props.find(uid).unwrap().keys;
    assert_eq!(keys.len(), 3);
    assert!(keys[1].roving, "the real dialog enables roving on the unlocked interior key");
    assert!((keys[1].time.seconds() - 1.8).abs() < 1e-6, "roving moves the interior key according to the unequal path segments");
    assert!(comp.layer(locked).unwrap() == &protected, "roving preserves the protected selected keys");
    let mut expected = selected;
    expected[0].time = keys[1].time;
    assert_eq!(h.state().session.state.selected_keys, expected, "retimed editable refs retain their original interleaved selection positions");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project));
}
