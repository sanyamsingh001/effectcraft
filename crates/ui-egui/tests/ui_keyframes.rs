//! Headless checks for Timeline keyframes as a person uses them: dragging keys over many frames
//! in one gesture (the drag used to end after the first frame), Shift-snapping, Ctrl+C /
//! Ctrl+V, which the windowing layer delivers as clipboard events rather than key presses, and
//! easing selected keys from the Ease Presets panel.

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;

/// A 4 s, 30 fps comp with a Box layer whose Opacity has keys at 0 s and 1 s, revealed in the
/// Timeline; returns the harness, the layer and the property uid.
fn harness() -> (Harness<'static, EffectcraftApp>, LayerId, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Keys", "width": 320, "height": 180, "frameRate": 30, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Box", "color": "#e04020", "width": 80, "height": 80})).unwrap();
    let id = s.active_comp().unwrap().layers[0].id;
    for (t, v) in [(0.0, 0.0), (1.0, 100.0)] {
        s.execute("prop.addKey", json!({"layer": id.0, "path": "transform/opacity", "time": t, "value": v})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute("layer.select", json!({"layers": [id.0]})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.opacity", json!({})).unwrap();
    h.run_steps(4);
    (h, id, uid)
}

fn key_times(h: &Harness<'_, EffectcraftApp>, id: LayerId) -> Vec<f64> {
    let l = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    l.props.prop("transform/opacity").unwrap().keys.iter().map(|k| (k.time.seconds() * 30.0).round() / 30.0).collect()
}

/// Screen centres of the property's keys, left to right.
fn keys(h: &Harness<'_, EffectcraftApp>, uid: u64) -> Vec<Pos2> {
    let mut ks: Vec<Pos2> =
        h.state().auto.query(&format!("timeline.key.{uid}.")).iter().map(|e| pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)).collect();
    ks.sort_by(|a, b| a.x.total_cmp(&b.x));
    ks
}

/// Press at `from`, move to `to` in small steps (a real drag), release.
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2, modifiers: egui::Modifiers) {
    drag_with_status(h, from, to, modifiers);
}

fn drag_with_status(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2, modifiers: egui::Modifiers) -> String {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.step();
    for i in 1..=24 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 24.0)));
        h.step();
    }
    let status = h.state().ui.status.clone();
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
    status
}

#[test]
fn a_key_drag_follows_the_pointer_for_its_whole_length() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    assert_eq!(ks.len(), 2);
    let px_per_s = ks[1].x - ks[0].x;
    // Drag the 1 s key to where 2.5 s is: many frames, one gesture.
    let to = pos2(ks[1].x + px_per_s * 1.5, ks[1].y);
    drag(&mut h, ks[1], to, Default::default());
    let ts = key_times(&h, id);
    assert!((ts[1] - 2.5).abs() <= 2.0 / 30.0, "the key follows the whole drag: {ts:?}");
    let steps = h.state().session.history.undo.iter().filter(|(l, _)| l == "Move Keyframes").count();
    assert_eq!(steps, 1, "one undo step per drag");
    // Shift snaps it to the current time indicator (at 0.5 s), from a few pixels away.
    h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(0.5));
    h.run_steps(2);
    let ks = keys(&h, uid);
    let near = pos2(ks[0].x + px_per_s * 0.5 + 4.0, ks[1].y);
    drag(&mut h, ks[1], near, egui::Modifiers { shift: true, ..Default::default() });
    assert_eq!(key_times(&h, id)[1], 0.5, "snapped to the current time");
}

#[test]
fn ctrl_c_and_ctrl_v_copy_and_paste_keys_at_the_current_time() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    // Click the 1 s key to select it, Ctrl+C (a clipboard event, not a key press).
    h.input_mut().events.push(Event::PointerMoved(ks[1]));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: ks[1], button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: ks[1], button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert_eq!(h.state().session.state.selected_keys.len(), 1);
    h.input_mut().events.push(Event::Copy);
    h.step();
    let copied: Vec<String> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, ["EffectCraft: 1 keyframe"], "the system clipboard is filled, so Ctrl+V sends a paste event");
    assert!(h.state().session.state.clip_is_keys);
    // Move to 3 s, Ctrl+V: the key lands there.
    h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(3.0));
    h.input_mut().events.push(Event::Paste("EffectCraft: 1 keyframe".into()));
    h.run_steps(2);
    assert_eq!(key_times(&h, id), vec![0.0, 1.0, 3.0]);
}

fn click_with(h: &mut Harness<'_, EffectcraftApp>, p: Pos2, modifiers: egui::Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
}

#[test]
fn shift_click_toggles_and_ctrl_click_switches_interpolation() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    let shift = egui::Modifiers { shift: true, ..Default::default() };
    click_with(&mut h, ks[0], Default::default());
    click_with(&mut h, ks[1], shift);
    assert_eq!(h.state().session.state.selected_keys.len(), 2, "Shift+click adds");
    click_with(&mut h, ks[1], shift);
    assert_eq!(h.state().session.state.selected_keys.len(), 1, "and takes out again");
    let key =
        |h: &Harness<'_, EffectcraftApp>| h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys[0].clone();
    // Ctrl+click: Linear → Auto Bezier → Linear.
    click_with(&mut h, ks[0], egui::Modifiers::COMMAND);
    assert!(key(&h).auto_bezier, "Auto Bezier");
    click_with(&mut h, ks[0], egui::Modifiers::COMMAND);
    assert_eq!(key(&h).out_interp, effectcraft_engine::keyframe::Interp::Linear);
    // Ctrl+Alt+click: Hold.
    click_with(&mut h, ks[0], egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND });
    assert_eq!(key(&h).out_interp, effectcraft_engine::keyframe::Interp::Hold);
}

/// Clicking a key in the Timeline selects its property and layer too, as in After Effects, so
/// the Graph Editor shows the key's curve (#252).
#[test]
fn a_key_clicked_in_the_timeline_shows_in_the_graph_editor() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("edit.deselectAll", json!({})).unwrap();
    h.run_steps(2);
    let ks = keys(&h, uid);
    click_with(&mut h, ks[1], Default::default());
    let st = &h.state().session.state;
    assert_eq!(st.selected_keys.len(), 1);
    assert_eq!((st.selected_props.clone(), st.selected_layers.clone()), (vec![(id, uid)], vec![id]), "the key's property and layer");
    h.state_mut().ui.timeline.graph_editor = true;
    h.run_steps(4);
    assert!(h.state().auto.find(&format!("timeline.graph.key.{uid}.0.1")).is_some(), "the Graph Editor shows the key");
}

#[test]
fn graph_editor_drags_move_every_selected_key_and_shift_keeps_an_axis() {
    let (mut h, id, uid) = harness();
    let s = &mut h.state_mut().session;
    s.execute("prop.select", json!({"layer": id.0, "prop": uid})).unwrap();
    s.execute("keys.selectAll", json!({})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.run_steps(4);
    let at = |h: &Harness<'_, EffectcraftApp>, i: usize| {
        let e = h.state().auto.find(&format!("timeline.graph.key.{uid}.0.{i}")).unwrap_or_else(|| panic!("no graph key {i}")).clone();
        pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
    };
    let (k0, k1) = (at(&h, 0), at(&h, 1));
    let px_per_s = k1.x - k0.x;
    let values = |h: &Harness<'_, EffectcraftApp>| -> Vec<f64> {
        h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys.iter().map(|k| k.value.as_f64()).collect()
    };
    // Shift: only along time, both keys, by 0.5 s.
    drag(&mut h, k1, pos2(k1.x + px_per_s * 0.5, k1.y - 6.0), egui::Modifiers { shift: true, ..Default::default() });
    assert_eq!(key_times(&h, id), vec![0.5, 1.5], "both selected keys moved");
    assert_eq!(values(&h), vec![0.0, 100.0], "Shift kept the values");
    let undo = h.state().session.history.undo.iter().filter(|(l, _)| l == "Transform Keyframes").count();
    assert_eq!(undo, 1, "one undo step per drag");
    // Without Shift the values move too (both keys, by the same amount).
    h.run_steps(2);
    let k1 = at(&h, 1);
    drag(&mut h, k1, pos2(k1.x, k1.y + 20.0), Default::default());
    let v = values(&h);
    assert!(v[0] < 0.0 && (v[1] - 100.0 - v[0]).abs() < 1e-6, "{v:?}");
}

#[test]
fn double_click_a_key_to_edit_its_value() {
    let (mut h, id, uid) = harness();
    let k1 = keys(&h, uid)[1];
    h.input_mut().events.push(Event::PointerMoved(k1));
    h.step();
    // Both clicks inside egui's double-click time.
    for _ in 0..2 {
        h.input_mut().events.push(Event::PointerButton { pos: k1, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.input_mut().events.push(Event::PointerButton { pos: k1, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    }
    h.step();
    h.run_steps(2);
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::Form), "the value dialog opened");
    h.run_steps(2);
    assert!(h.state().auto.find("form.field.value").is_some(), "one Opacity field");
    // OK keeps the key where it is (the dialog edits the value only).
    let ok = h.state().auto.find("form.ok").expect("form.ok").clone();
    let p = pos2(ok.rect[0] + ok.rect[2] / 2.0, ok.rect[1] + ok.rect[3] / 2.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    let k = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys[1].clone();
    assert_eq!((k.time.seconds(), k.value.as_f64()), (1.0, 100.0));
}

#[test]
fn k_follows_the_revealed_properties_and_auto_select_picks_the_speed_graph_for_position() {
    let (mut h, id, _) = harness();
    // A Rotation key at 2 s on a property that isn't revealed: K skips it.
    h.state_mut().session.execute("prop.addKey", json!({"layer": id.0, "path": "transform/rotation", "time": 2.0, "value": 45})).unwrap();
    h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(1.0));
    h.run_steps(2);
    h.input_mut().events.push(Event::Key { key: egui::Key::K, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.run_steps(2);
    assert!(h.state().session.time().seconds() > 3.9, "past the hidden key to the work area end: {}", h.state().session.time().seconds());
    // Auto-Select Graph Type: Position shows its speed (one curve), Opacity its value.
    let s = &mut h.state_mut().session;
    for (t, v) in [(0.0, [40.0, 90.0]), (1.0, [280.0, 90.0])] {
        s.execute("prop.addKey", json!({"layer": id.0, "path": "transform/position", "time": t, "value": v})).unwrap();
    }
    let pos = s.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap().uid;
    s.execute("prop.select", json!({"layer": id.0, "prop": pos})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.run_steps(4);
    assert_eq!(h.state().ui.timeline.graph_mode, "auto");
    assert!(h.state().auto.find(&format!("timeline.graph.key.{pos}.0.0")).is_some());
    assert!(h.state().auto.find(&format!("timeline.graph.key.{pos}.1.0")).is_none(), "speed graph: one curve");
    assert!(h.state().auto.find("timeline.graph.autoSelectGraphType").is_some());
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.graph.key.{pos}.1.0")).is_some(), "value graph: X and Y");
}

fn center(h: &Harness<'_, EffectcraftApp>, id: &str) -> Pos2 {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
}

#[test]
fn clicking_an_ease_preset_eases_the_selected_keys_and_handles_edit_the_curve() {
    use effectcraft_engine::keyframe::{Ease, Interp};
    use effectcraft_ui_egui::dock::PanelKind;
    let (mut h, id, _) = harness();
    h.state_mut().session.execute("prop.select", json!({"layer": id.0, "path": "transform/opacity"})).unwrap();
    h.state_mut().show_panel(PanelKind::EasePresets);
    h.state_mut().ui.maximized = Some(PanelKind::EasePresets);
    h.run_steps(4);
    let opacity =
        |h: &Harness<'_, EffectcraftApp>| h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().clone();
    let tile = h.state().auto.query("easePresets.preset.").into_iter().find(|e| e.label == "Ease In-Out").expect("the Ease In-Out thumbnail").id.clone();
    let steps = h.state().session.history.undo.len();
    let at = center(&h, &tile);
    click_with(&mut h, at, Default::default());
    let op = opacity(&h);
    let eased = Ease { speed: 0.0, influence: 0.5 };
    assert_eq!((op.keys[0].out_interp, op.keys[0].out_ease.clone()), (Interp::Bezier, vec![eased]));
    assert_eq!((op.keys[1].in_interp, op.keys[1].in_ease.clone()), (Interp::Bezier, vec![eased]));
    assert_eq!(h.state().session.history.undo.len(), steps + 1);
    assert_eq!(h.state().ui.ease_presets.selected.as_deref(), Some("Ease In-Out"));
    // Dragging the out handle to the right lengthens its influence; Apply uses the edited curve.
    let graph = h.state().auto.find("easePresets.graph").unwrap().rect;
    let from = center(&h, "easePresets.handle.out");
    drag(&mut h, from, from + egui::vec2(graph[2] * 0.2, 0.0), Default::default());
    let out = h.state().ui.ease_presets.curve.out_influence;
    assert!(out > 60.0 && out < 75.0, "{out}");
    let apply = center(&h, "easePresets.apply");
    click_with(&mut h, apply, Default::default());
    let applied = opacity(&h).keys[0].out_ease[0].influence;
    assert!((applied * 100.0 - out).abs() < 1e-9, "{applied} vs {out}");
    assert_eq!(h.state().session.history.undo.len(), steps + 2);
}

#[test]
fn dragging_a_graph_handle_edits_all_selected_endpoints_in_one_undo_step() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("prop.select", json!({"layer": id.0, "prop": uid})).unwrap();
    h.state_mut().session.execute("keys.select", json!({"keys": [{"layer": id.0, "prop": uid, "time": 0}]})).unwrap();
    h.state_mut().session.execute("keys.easyEaseOut", json!({})).unwrap();
    h.state_mut().session.execute("keys.selectAll", json!({})).unwrap();
    // Stretched layers still present the handle at its corresponding composition time.
    let cid = h.state().session.active_comp_id().unwrap();
    std::sync::Arc::make_mut(&mut h.state_mut().session.project).comp_mut(cid).unwrap().layer_mut(id).unwrap().stretch = 200.0;
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(4);
    let k0 = center(&h, &format!("timeline.graph.key.{uid}.0.0"));
    let k1 = center(&h, &format!("timeline.graph.key.{uid}.0.1"));
    let handle = center(&h, &format!("timeline.graph.handle.{uid}.0.0.out"));
    assert!(((handle.x - k0.x) / (k1.x - k0.x) - 1.0 / 3.0).abs() < 0.002);
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().clone();
    assert_eq!(before.keys[1].in_interp, effectcraft_engine::keyframe::Interp::Linear);
    let history = h.state().session.history.undo.len();
    drag(&mut h, handle, pos2(handle.x + (k1.x - k0.x) * 0.15, handle.y - 12.0), Default::default());
    let after = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().clone();
    assert!(after.keys[0].out_ease[0].influence > 0.45);
    assert_eq!(after.keys[1].in_interp, effectcraft_engine::keyframe::Interp::Bezier);
    assert_eq!(after.keys[0].out_ease[0], after.keys[1].in_ease[0], "both selected endpoint handles receive the edit");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert_eq!(key_times(&h, id), [0.0, 1.0]);
    assert_eq!(after.keys.iter().map(|k| k.value.as_f64()).collect::<Vec<_>>(), [0.0, 100.0]);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap(), &before);
}

#[test]
fn ctrl_alt_timeline_drag_staggers_in_selection_order_instead_of_scaling() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("prop.addKey", json!({"layer": id.0, "prop": uid, "time": 2, "value": 50})).unwrap();
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys": [
                {"layer": id.0, "prop": uid, "time": 2},
                {"layer": id.0, "prop": uid, "time": 0},
                {"layer": id.0, "prop": uid, "time": 1}
            ]}),
        )
        .unwrap();
    h.run_steps(3);
    let ks = keys(&h, uid);
    let px_per_s = ks[1].x - ks[0].x;
    let history = h.state().session.history.undo.len();
    let drag_status = drag_with_status(&mut h, ks[1], pos2(ks[1].x + px_per_s * 0.3, ks[1].y), egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND });
    assert_eq!(key_times(&h, id), [0.3, 1.6, 2.0]);
    let s = &h.state().session;
    let expected = effectcraft_ui_egui::panels::timecode(s, s.active_comp().unwrap(), effectcraft_engine::time::Tick::from_seconds_f64(1.6));
    assert_eq!(drag_status, format!("Keyframe at {expected}"), "the Info time stays visible throughout the live drag");
    assert_eq!(h.state().ui.status, format!("Keyframe at {expected}"), "the Info time follows the grabbed key's own stagger offset");
    assert_eq!(h.state().session.state.selected_keys.iter().map(|k| (k.time.seconds() * 30.0).round() as i64).collect::<Vec<_>>(), [60, 9, 48]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(key_times(&h, id), [0.0, 1.0, 2.0]);
}

#[test]
fn ctrl_alt_graph_drag_staggers_keys_and_keeps_their_values() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("prop.select", json!({"layer": id.0, "prop": uid})).unwrap();
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys": [
                {"layer": id.0, "prop": uid, "time": 1}, {"layer": id.0, "prop": uid, "time": 0}
            ]}),
        )
        .unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.graph_snap = false;
    h.run_steps(4);
    let k0 = center(&h, &format!("timeline.graph.key.{uid}.0.0"));
    let k1 = center(&h, &format!("timeline.graph.key.{uid}.0.1"));
    let history = h.state().session.history.undo.len();
    drag(&mut h, k0, pos2(k0.x + (k1.x - k0.x) * 0.3, k0.y - 12.0), egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND });
    assert_eq!(key_times(&h, id), [0.3, 1.0]);
    let values: Vec<_> =
        h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys.iter().map(|k| k.value.as_f64()).collect();
    assert_eq!(values, [0.0, 100.0]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
}

#[test]
fn ctrl_alt_layer_bar_drag_staggers_layers_in_selection_order() {
    let (mut h, id, uid) = harness();
    let id2 = h.state_mut().session.execute("layer.newSolid", json!({"name": "Second", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
    let id3 = h.state_mut().session.execute("layer.newSolid", json!({"name": "Third", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers": [id2, id.0, id3]})).unwrap();
    h.run_steps(4);
    let ks = keys(&h, uid);
    let px_per_s = ks[1].x - ks[0].x;
    let bar = center(&h, &format!("timeline.layer.{}.bar", id.0));
    let history = h.state().session.history.undo.len();
    drag(&mut h, bar, pos2(bar.x + px_per_s * 0.3, bar.y), egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND });
    let starts = |h: &Harness<'_, EffectcraftApp>| {
        [id2, id.0, id3].map(|id| (h.state().session.active_comp().unwrap().layer(LayerId(id)).unwrap().start_time.seconds() * 30.0).round() as i64)
    };
    let after = starts(&h);
    assert_eq!(after, [0, 9, 18]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(starts(&h), [0, 0, 0]);
}

#[test]
fn graph_stagger_drag_recovers_after_the_grabbed_widget_index_disappears() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("prop.addKey", json!({"layer": id.0, "prop": uid, "time": 2, "value": 50})).unwrap();
    h.state_mut().session.execute("prop.select", json!({"layer": id.0, "prop": uid})).unwrap();
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys": [
                {"layer": id.0, "prop": uid, "time": 2},
                {"layer": id.0, "prop": uid, "time": 0},
                {"layer": id.0, "prop": uid, "time": 1}
            ]}),
        )
        .unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.graph_snap = false;
    h.run_steps(4);
    let from = center(&h, &format!("timeline.graph.key.{uid}.0.2"));
    let k0 = center(&h, &format!("timeline.graph.key.{uid}.0.0"));
    let k1 = center(&h, &format!("timeline.graph.key.{uid}.0.1"));
    let pps = k1.x - k0.x;
    let modifiers = egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND };
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().clone();
    let history = h.state().session.history.undo.len();
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.step();
    h.event(Event::PointerMoved(pos2(from.x - pps * 0.5, from.y)));
    h.step();
    assert_eq!(key_times(&h, id), [0.0, 2.0], "the other keys temporarily share frame zero");
    h.event(Event::PointerMoved(pos2(from.x + pps * 0.3, from.y)));
    h.step();
    assert_eq!(key_times(&h, id), [0.3, 1.6, 2.0], "the drag survives the missing widget and restores the original keys");
    h.event(Event::PointerButton { pos: pos2(from.x + pps * 0.3, from.y), button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.event(Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
    assert_eq!(h.state().session.state.selected_keys.iter().map(|k| (k.time.seconds() * 30.0).round() as i64).collect::<Vec<_>>(), [60, 9, 48]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap(), &before);
}

#[test]
fn graph_key_release_keeps_the_drag_scale_and_applies_final_pointer_motion() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("prop.select", json!({"layer": id.0, "prop": uid})).unwrap();
    h.state_mut().session.execute("keys.select", json!({"keys": [{"layer": id.0, "prop": uid, "time": 1}]})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.graph_snap = false;
    h.run_steps(4);
    let from = center(&h, &format!("timeline.graph.key.{uid}.0.1"));
    let k0 = center(&h, &format!("timeline.graph.key.{uid}.0.0"));
    let span = k0.y - from.y;
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(Event::PointerMoved(pos2(from.x, from.y + span * 0.1)));
    h.step();
    let value = |h: &Harness<'_, EffectcraftApp>| {
        h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys[1].value.as_f64()
    };
    assert!((value(&h) - 90.0).abs() < 0.1);
    h.event(Event::PointerButton {
        pos: pos2(from.x, from.y + span * 0.2),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_steps(2);
    assert!((value(&h) - 80.0).abs() < 0.1, "release motion uses the scale held during the drag: {}", value(&h));
}

#[test]
fn reversed_stretched_layers_navigate_to_the_nearest_comp_time_key() {
    use effectcraft_engine::time::Tick;

    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for percent in [50.0, 100.0] {
        let (mut h, id, uid) = harness();
        h.state_mut().session.execute("prop.addKey", json!({"layer": id.0, "prop": uid, "time": 2, "value": 50})).unwrap();
        h.state_mut().session.execute("layer.timeStretch", json!({"layers": [id.0], "percent": percent})).unwrap();
        h.state_mut().session.execute("layer.timeReverse", json!({"layers": [id.0]})).unwrap();
        h.run_steps(3);
        let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
        assert_eq!(layer.stretch, -percent);
        let mut comp_times: Vec<_> = layer.props.find(uid).unwrap().keys.iter().map(|k| layer.comp_time(k.time)).collect();
        comp_times.sort();
        assert_eq!(comp_times.len(), 3);
        assert!(comp_times.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(comp_times[0] >= Tick::ZERO && comp_times[2] < h.state().session.active_comp().unwrap().duration);
        let history = h.state().session.history.undo.len();
        // Previous and Next flank the existing Add Key button, at 16 px either side.
        for (a, b, offset) in [(comp_times[1], comp_times[2], -16.0), (comp_times[0], comp_times[1], 16.0)] {
            h.state_mut().session.set_time(Tick(a.0 + (b.0 - a.0) / 2));
            h.run_steps(2);
            let mid = center(&h, &format!("timeline.prop.{uid}.addKey"));
            click_with(&mut h, pos2(mid.x + offset, mid.y), Default::default());
            actual.push(h.state().session.time());
            expected.push(comp_times[1]);
        }
        assert_eq!(h.state().session.history.undo.len(), history, "navigation leaves the animation unchanged");
    }
    assert_eq!(actual, expected, "both arrow buttons choose the nearest key in composition time");
}

#[test]
fn graph_transform_box_recovers_after_selected_keys_collapse_to_one_frame() {
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute("keys.select", json!({"keys":[{"layer":id.0,"prop":uid,"time":0},{"layer":id.0,"prop":uid,"time":1}]})).unwrap();
    h.state_mut().session.execute("keys.delete", json!({})).unwrap();
    for time in [0.0, 0.1] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":id.0,"prop":uid,"time":time,"value":50})).unwrap();
    }
    h.state_mut().session.execute("prop.select", json!({"layer":id.0,"prop":uid})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.graph_snap = false;
    h.state_mut().ui.timeline.graph_transform_box = true;
    h.run_steps(4);
    let k0 = center(&h, &format!("timeline.graph.key.{uid}.0.0"));
    let k1 = center(&h, &format!("timeline.graph.key.{uid}.0.1"));
    let from = center(&h, "timeline.graph.transformBox.5");
    let pps = (k1.x - k0.x) / 0.1;
    let history = h.state().session.history.undo.len();
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().props.find(uid).unwrap().clone();
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(Event::PointerMoved(pos2(k0.x + pps * 0.01, from.y)));
    h.step();
    assert_eq!(key_times(&h, id), [0.0], "both keys temporarily round onto frame zero");
    assert_eq!(h.state().session.state.selected_keys.len(), 1);
    let expanded = pos2(k0.x + pps * 0.2, from.y);
    h.event(Event::PointerMoved(expanded));
    h.step();
    assert_eq!(key_times(&h, id), [0.0, 0.2], "the same held transform-box gesture restores both original keys");
    h.event(Event::PointerButton { pos: expanded, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(id).unwrap().props.find(uid).unwrap(), &before);
}

#[test]
fn layer_stagger_drag_applies_final_release_motion_in_both_directions() {
    for direction in [1.0_f32, -1.0] {
        let (mut h, id, uid) = harness();
        let second = h.state_mut().session.execute("layer.newSolid", json!({"name": "Second", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        let third = h.state_mut().session.execute("layer.newSolid", json!({"name": "Third", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        h.state_mut().session.execute("layer.select", json!({"layers": [second, id.0, third]})).unwrap();
        h.run_steps(4);
        let ks = keys(&h, uid);
        let pps = ks[1].x - ks[0].x;
        let from = center(&h, &format!("timeline.layer.{}.bar", id.0));
        let to = from + egui::vec2(pps * 0.3 * direction, 0.0);
        let modifiers = egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND };
        let before = h.state().session.project.clone();
        let history = h.state().session.history.undo.len();
        h.event(Event::PointerMoved(from));
        h.step();
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
        h.step();
        h.event(Event::PointerMoved(from + egui::vec2(15.0 * direction, 0.0)));
        h.step();
        h.event(Event::PointerMoved(to));
        h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
        h.step();
        h.event(Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(2);
        let comp = h.state().session.active_comp().unwrap();
        let frames = [second, id.0, third].map(|layer| (comp.layer(LayerId(layer)).unwrap().start_time.seconds() * 30.0).round() as i64);
        assert_eq!(frames, [0, 9 * direction as i64, 18 * direction as i64], "the release frame completes the original stagger gesture");
        assert_eq!(h.state().session.state.selected_layers, [LayerId(second), id, LayerId(third)]);
        assert_eq!(h.state().session.history.undo.len(), history + 1);
        assert!(h.state_mut().session.undo());
        assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &before));
    }
}

#[test]
fn delete_from_graph_editor_preserves_keys_locked_from_the_timeline() {
    let (mut h, id, uid) = harness();
    h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
    h.state_mut().session.execute("prop.select", json!({"layer":id.0,"prop":uid})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(3);
    assert_eq!(h.state().session.state.selected_keys.len(), 2);
    let lock = center(&h, &format!("timeline.layer.{}.lock", id.0));
    click_with(&mut h, lock, Default::default());
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    assert!(before.switches.locked);
    assert_eq!(h.state().session.state.selected_keys.len(), 2, "the existing graph key selection remains after the UI lock");
    assert_eq!(h.state().ui.focused, effectcraft_ui_egui::dock::PanelKind::Timeline);
    let history = h.state().session.history.undo.len();
    let locked_selection = h.state().session.state.selected_keys.clone();
    for pressed in [true, false] {
        h.event(Event::Key { key: egui::Key::Delete, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
    }
    h.run_steps(2);
    let after = h.state().session.active_comp().unwrap().layer(id).unwrap();
    eprintln!("locked graph key count before={}; after={}", before.props.find(uid).unwrap().keys.len(), after.props.find(uid).unwrap().keys.len());
    assert!(after == &before, "Delete in the Graph Editor must preserve its locked key selection");
    assert_eq!(h.state().session.history.undo.len(), history, "a protected Delete does not create an edit");
    assert_eq!(h.state().session.state.selected_keys, locked_selection, "the locked graph selection remains protected");
}

#[test]
fn delete_from_graph_editor_only_removes_unlocked_keys_in_mixed_selection() {
    let (mut h, locked, uid) = harness();
    let editable = LayerId(h.state_mut().session.execute("layer.newSolid", json!({"name":"Editable"})).unwrap()["layer"].as_u64().unwrap());
    for (time, value) in [(0.0, 25.0), (1.0, 75.0)] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":editable.0,"path":"transform/opacity","time":time,"value":value})).unwrap();
    }
    let editable_uid = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys":[
        {"layer":locked.0,"prop":uid,"time":0},{"layer":locked.0,"prop":uid,"time":1},
        {"layer":editable.0,"prop":editable_uid,"time":0},{"layer":editable.0,"prop":editable_uid,"time":1}
    ],"selectProperties":true}),
        )
        .unwrap();
    h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(3);
    let lock = center(&h, &format!("timeline.layer.{}.lock", locked.0));
    click_with(&mut h, lock, Default::default());
    let before = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    assert!(before.switches.locked);
    assert_eq!(h.state().session.state.selected_keys.len(), 4);
    let protected: Vec<_> = h.state().session.state.selected_keys.iter().copied().filter(|key| key.layer == locked).collect();
    assert_eq!(protected.len(), 2);
    assert_eq!(h.state().ui.focused, effectcraft_ui_egui::dock::PanelKind::Timeline);
    let project_before = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    for pressed in [true, false] {
        h.event(Event::Key { key: egui::Key::Delete, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
    }
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    assert!(comp.layer(editable).unwrap().props.find(editable_uid).unwrap().keys.is_empty(), "Delete still removes the editable selected keys");
    let after = comp.layer(locked).unwrap();
    eprintln!("mixed Delete locked count before={}; after={}", before.props.find(uid).unwrap().keys.len(), after.props.find(uid).unwrap().keys.len());
    assert!(after == &before, "mixed Delete must preserve the locked layer's animation");
    assert_eq!(h.state().session.state.selected_keys, protected, "the protected keys stay selected");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project_before), "one Undo restores the editable animation and the locked layer");
    assert_eq!(h.state().session.state.selected_keys, protected);
}

fn assert_locked_graph_ease_shortcut(key_modifiers: egui::Modifiers) {
    let (mut h, id, uid) = harness();
    h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
    h.state_mut().session.execute("prop.select", json!({"layer":id.0,"prop":uid})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(3);
    assert_eq!(h.state().session.state.selected_keys.len(), 2);
    let lock = center(&h, &format!("timeline.layer.{}.lock", id.0));
    click_with(&mut h, lock, Default::default());
    let before = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    assert!(before.switches.locked);
    assert_eq!(h.state().session.state.selected_keys.len(), 2, "the existing graph key selection remains after the UI lock");
    assert_eq!(h.state().ui.focused, effectcraft_ui_egui::dock::PanelKind::Timeline);
    let history = h.state().session.history.undo.len();
    let locked_selection = h.state().session.state.selected_keys.clone();
    for pressed in [true, false] {
        h.event(Event::Key { key: egui::Key::F9, physical_key: None, pressed, repeat: false, modifiers: key_modifiers });
    }
    h.run_steps(2);
    let after = h.state().session.active_comp().unwrap().layer(id).unwrap();
    eprintln!("locked Easy Ease keys unchanged={}", before.props.find(uid).unwrap().keys == after.props.find(uid).unwrap().keys);
    assert!(after == &before, "Easy Ease in the Graph Editor must preserve its locked key selection");
    assert_eq!(h.state().session.history.undo.len(), history, "a protected Easy Ease shortcut does not create an edit");
    assert_eq!(h.state().session.state.selected_keys, locked_selection, "the locked graph selection remains protected");
}

#[test]
fn easy_ease_shortcut_preserves_keys_locked_from_the_timeline() {
    assert_locked_graph_ease_shortcut(egui::Modifiers::NONE);
}

#[test]
fn easy_ease_in_shortcut_preserves_keys_locked_from_the_timeline() {
    assert_locked_graph_ease_shortcut(egui::Modifiers::SHIFT);
}

#[test]
fn easy_ease_out_shortcut_preserves_keys_locked_from_the_timeline() {
    assert_locked_graph_ease_shortcut(egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND });
}

fn assert_mixed_graph_ease_shortcut(key_modifiers: egui::Modifiers) {
    let (mut h, locked, uid) = harness();
    let editable = LayerId(h.state_mut().session.execute("layer.newSolid", json!({"name":"Editable"})).unwrap()["layer"].as_u64().unwrap());
    for (time, value) in [(0.0, 25.0), (1.0, 75.0)] {
        h.state_mut().session.execute("prop.addKey", json!({"layer":editable.0,"path":"transform/opacity","time":time,"value":value})).unwrap();
    }
    let editable_uid = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute(
            "keys.select",
            json!({"keys":[
        {"layer":editable.0,"prop":editable_uid,"time":1},{"layer":locked.0,"prop":uid,"time":0},
        {"layer":editable.0,"prop":editable_uid,"time":0},{"layer":locked.0,"prop":uid,"time":1}
    ],"selectProperties":true}),
        )
        .unwrap();
    h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.run_steps(3);
    let lock = center(&h, &format!("timeline.layer.{}.lock", locked.0));
    click_with(&mut h, lock, Default::default());
    let before = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    assert!(before.switches.locked);
    assert_eq!(h.state().session.state.selected_keys.len(), 4);
    let protected: Vec<_> = h.state().session.state.selected_keys.iter().copied().filter(|key| key.layer == locked).collect();
    assert_eq!(protected.len(), 2);
    assert_eq!(h.state().ui.focused, effectcraft_ui_egui::dock::PanelKind::Timeline);
    let editable_before = h.state().session.active_comp().unwrap().layer(editable).unwrap().clone();
    let selection_before = h.state().session.state.selected_keys.clone();
    let project_before = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    for pressed in [true, false] {
        h.event(Event::Key { key: egui::Key::F9, physical_key: None, pressed, repeat: false, modifiers: key_modifiers });
    }
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    let edited_keys = &comp.layer(editable).unwrap().props.find(editable_uid).unwrap().keys;
    assert_eq!(edited_keys.len(), 2, "Easy Ease keeps the editable selected keys");
    assert_ne!(edited_keys, &editable_before.props.find(editable_uid).unwrap().keys, "the shortcut actually eases the editable selection");
    let after = comp.layer(locked).unwrap();
    eprintln!("mixed Easy Ease locked count before={}; after={}", before.props.find(uid).unwrap().keys.len(), after.props.find(uid).unwrap().keys.len());
    assert!(after == &before, "mixed Easy Ease must preserve the locked layer's animation");
    assert_eq!(h.state().session.state.selected_keys, selection_before, "easing keeps the mixed key selection in its original order");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project_before), "one Undo restores the editable animation and the locked layer");
    assert_eq!(h.state().session.state.selected_keys, selection_before);
}

#[test]
fn easy_ease_shortcut_only_edits_unlocked_keys_and_keeps_the_mixed_graph_selection() {
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT, egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND }] {
        assert_mixed_graph_ease_shortcut(modifiers);
    }
}
