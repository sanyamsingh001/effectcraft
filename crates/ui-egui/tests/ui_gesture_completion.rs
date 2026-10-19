//! Pointer completion, undo boundaries, and live layer-lock regressions.
//! Procedural 4-second comps, two keys and at most two layers; no external assets.
use std::sync::Arc;

use effectcraft_engine::{Session, project::LayerId, time::Tick};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

type H = Harness<'static, EffectcraftApp>;

fn harness() -> (H, LayerId, u64) {
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"name":"R1", "width":640, "height":360, "duration":4, "frameRate":30})).unwrap();
    s.execute_checked("layer.newSolid", json!({"name":"Box", "width":80, "height":80, "color":"#e04020"})).unwrap();
    let id = s.active_comp().unwrap().layers[0].id;
    for (time, value) in [(0, 0), (1, 100)] {
        s.execute_checked("prop.addKey", json!({"layer":id.0, "path":"transform/opacity", "time":time, "value":value})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute_checked("layer.select", json!({"layers":[id.0]})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.state_mut().ui.maximized = Some(PanelKind::Timeline);
    h.run_steps(3);
    invoke(&mut h, "timeline.reveal.opacity", json!({}));
    h.run_steps(3);
    (h, id, uid)
}

fn invoke(h: &mut H, command: &str, params: serde_json::Value) {
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, command, params).unwrap();
}

fn rect(h: &H, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing automation widget {id}"));
    let r = Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]));
    assert!(r.is_finite() && r.is_positive(), "invalid widget {id}: {r:?}");
    r
}

fn points(h: &H, uid: u64, graph: bool) -> [Pos2; 2] {
    if graph {
        return [rect(h, &format!("timeline.graph.key.{uid}.0.0")).center(), rect(h, &format!("timeline.graph.key.{uid}.0.1")).center()];
    }
    let mut keys: Vec<_> =
        h.state().auto.query(&format!("timeline.key.{uid}.")).into_iter().map(|e| pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)).collect();
    keys.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(keys.len(), 2, "the two keys are visible");
    [keys[0], keys[1]]
}

fn press(h: &mut H, p: Pos2, modifiers: Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers });
    h.step();
}

fn move_to(h: &mut H, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
}

fn release(h: &mut H, p: Pos2, modifiers: Modifiers) {
    // Deliberately no PointerMoved before this release. Its event position is the final motion.
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn drag(h: &mut H, from: Pos2, to: Pos2) {
    press(h, from, Modifiers::NONE);
    for i in 1..=8 {
        move_to(h, from + (to - from) * (i as f32 / 8.0));
    }
    release(h, to, Modifiers::NONE);
}

fn click(h: &mut H, p: Pos2) {
    press(h, p, Modifiers::NONE);
    release(h, p, Modifiers::NONE);
}

fn frames(h: &H, id: LayerId) -> Vec<i64> {
    let c = h.state().session.active_comp().unwrap();
    c.layer(id).unwrap().props.prop("transform/opacity").unwrap().keys.iter().map(|k| c.frame_rate.frame_at(k.time)).collect()
}

fn work_area(h: &H) -> (i64, i64) {
    let c = h.state().session.active_comp().unwrap();
    (c.frame_rate.frame_at(c.work_area.0), c.frame_rate.frame_at(c.work_area.1))
}

fn ui_lock(h: &mut H, id: LayerId) {
    invoke(h, "layer.setSwitch", json!({"layers":[id.0], "switch":"lock", "value":true}));
    assert!(h.state().session.active_comp().unwrap().layer(id).unwrap().switches.locked);
}

fn graph(h: &mut H) {
    let tl = &mut h.state_mut().ui.timeline;
    tl.graph_editor = true;
    tl.graph_mode = "value".into();
    tl.graph_snap = false;
    tl.graph_auto_zoom = false;
    tl.graph_range = Some((-40.0, 140.0));
    h.run_steps(3);
}

fn mixed(graph_on: bool) -> (H, LayerId, u64, LayerId) {
    let (mut h, owner, uid) = harness();
    h.state_mut().session.execute_checked("layer.newSolid", json!({"name":"Second", "width":80, "height":80, "color":"#20a0e0"})).unwrap();
    let second = h.state().session.active_comp().unwrap().layers.iter().find(|l| l.name == "Second").unwrap().id;
    for (time, value) in [(0, 25), (1, 75)] {
        h.state_mut().session.execute_checked("prop.addKey", json!({"layer":second.0, "path":"transform/opacity", "time":time, "value":value})).unwrap();
    }
    // Reveal only the new layer; switch kinds first to avoid the global T toggle / double press.
    h.state_mut().session.execute_checked("layer.select", json!({"layers":[second.0]})).unwrap();
    invoke(&mut h, "timeline.reveal.position", json!({}));
    invoke(&mut h, "timeline.reveal.opacity", json!({}));
    select_mixed_last(&mut h, owner, uid, second);
    if graph_on {
        graph(&mut h);
    } else {
        h.run_steps(3);
    }
    let other_uid = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().uid;
    points(&h, uid, graph_on);
    points(&h, other_uid, graph_on);
    (h, owner, uid, second)
}

fn select_mixed_last(h: &mut H, owner: LayerId, uid: u64, second: LayerId) {
    let other_uid = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute_checked(
            "keys.select",
            json!({"keys":[
        {"layer":owner.0, "prop":uid, "time":1},
        {"layer":second.0, "prop":other_uid, "time":1}
    ], "selectProperties":true}),
        )
        .unwrap();
}

#[test]
fn timeline_key_uses_final_release_position() {
    let (mut h, id, uid) = harness();
    let k = points(&h, uid, false);
    let pps = k[1].x - k[0].x;
    let before = Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    press(&mut h, k[1], Modifiers::NONE);
    move_to(&mut h, k[1] + vec2(pps * 0.1, 0.0));
    assert_eq!(frames(&h, id), [0, 33], "the drag was acquired before release");
    release(&mut h, k[1] + vec2(pps * 0.3, 0.0), Modifiers::NONE);
    assert_eq!(frames(&h, id), [0, 39], "the release event carries the final pointer position");
    assert_eq!(h.state().session.history.undo.len(), history + 1, "one gesture");
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before);
}

#[test]
fn alt_key_scale_uses_final_release_position() {
    // The shared graph_tools AltScale path is initiated by the Timeline's Alt endpoint drag.
    let (mut h, id, uid) = harness();
    h.state_mut().session.execute_checked("prop.select", json!({"layer":id.0, "prop":uid, "selectKeys":true})).unwrap();
    h.run_steps(2);
    let k = points(&h, uid, false);
    let pps = k[1].x - k[0].x;
    let before = Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    let mods = Modifiers { alt: true, ..Modifiers::NONE };
    press(&mut h, k[1], mods);
    move_to(&mut h, k[1] + vec2(pps * 0.1, 0.0));
    assert_eq!(frames(&h, id), [0, 33], "the Alt scale was acquired before release");
    release(&mut h, k[1] + vec2(pps * 0.3, 0.0), mods);
    assert_eq!(frames(&h, id), [0, 39]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before);
}

#[test]
fn work_area_bar_and_handles_use_final_release_position() {
    let mut observations = vec![];
    for (kind, expected) in [("bar", (24, 99)), ("begin", (24, 90)), ("end", (15, 99))] {
        let (mut h, _, uid) = harness();
        h.state_mut().session.execute_checked("comp.workArea", json!({"start":0.5, "end":3})).unwrap();
        h.run_steps(3);
        let k = points(&h, uid, false);
        let pps = k[1].x - k[0].x;
        let p = rect(&h, &format!("timeline.workArea.{kind}")).center();
        press(&mut h, p, Modifiers::NONE);
        move_to(&mut h, p + vec2(pps * 0.1, 0.0));
        assert_ne!(work_area(&h), (15, 90), "{kind} drag was acquired");
        release(&mut h, p + vec2(pps * 0.3, 0.0), Modifiers::NONE);
        observations.push((kind, work_area(&h), expected));
    }
    assert!(observations.iter().all(|(_, actual, expected)| actual == expected), "{observations:?}");
}

#[test]
fn distinct_work_area_gestures_have_distinct_undo_steps() {
    let (mut h, _, uid) = harness();
    h.state_mut().session.execute_checked("comp.workArea", json!({"start":0.5, "end":3})).unwrap();
    h.state_mut().session.history.undo.clear();
    h.state_mut().session.history.redo.clear();
    h.run_steps(3);
    let k = points(&h, uid, false);
    let pps = k[1].x - k[0].x;
    let begin = rect(&h, "timeline.workArea.begin").center();
    drag(&mut h, begin, begin + vec2(pps * 0.3, 0.0));
    assert_eq!(work_area(&h), (24, 90), "first gesture changed only begin");
    let end = rect(&h, "timeline.workArea.end").center();
    drag(&mut h, end, end - vec2(pps * 0.3, 0.0));
    assert_eq!(work_area(&h), (24, 81), "second gesture changed only end");
    let count = h.state().session.history.undo.len();
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!((count, work_area(&h)), (2, (24, 90)), "Undo removes only the second gesture");
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(work_area(&h), (15, 90));
}

#[test]
fn locked_key_owner_cannot_start_a_mixed_group_drag() {
    let mut observations = vec![];
    for graph_on in [false, true] {
        let (mut h, owner, uid, second) = mixed(graph_on);
        let p = points(&h, uid, graph_on)[1];
        drag(&mut h, p, p + vec2(40.0, 0.0));
        assert_ne!(frames(&h, second), [0, 30], "unlocked mixed drag positive control ({graph_on})");
        h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
        select_mixed_last(&mut h, owner, uid, second);
        h.run_steps(2);
        ui_lock(&mut h, owner);
        h.run_steps(2);
        let before = Arc::clone(&h.state().session.project);
        let selection = h.state().session.state.selected_keys.clone();
        let history = h.state().session.history.undo.len();
        let p = points(&h, uid, graph_on)[1];
        drag(&mut h, p, p + vec2(40.0, 0.0));
        observations.push((
            graph_on,
            h.state().session.project == before,
            h.state().session.state.selected_keys == selection,
            h.state().session.history.undo.len() == history,
            frames(&h, second),
        ));
    }
    assert!(observations.iter().all(|(_, project, selection, history, _)| *project && *selection && *history), "{observations:?}");
}

#[test]
fn captured_key_drag_stops_when_its_owner_becomes_locked() {
    let mut observations = vec![];
    for graph_on in [false, true] {
        let (mut h, owner, uid, second) = mixed(graph_on);
        let k = points(&h, uid, graph_on);
        let pps = k[1].x - k[0].x;
        press(&mut h, k[1], Modifiers::NONE);
        move_to(&mut h, k[1] + vec2(pps * 0.1, 0.0));
        assert_eq!(frames(&h, owner), [0, 33], "owner gesture acquired ({graph_on})");
        assert_eq!(frames(&h, second), [0, 33], "editable follower moved ({graph_on})");
        ui_lock(&mut h, owner);
        // Snapshot after the lock command so its own undo entry is excluded from the assertion.
        let before = Arc::clone(&h.state().session.project);
        let selection = h.state().session.state.selected_keys.clone();
        let history = h.state().session.history.undo.len();
        move_to(&mut h, k[1] + vec2(pps * 0.3, 0.0));
        release(&mut h, k[1] + vec2(pps * 0.4, 0.0), Modifiers::NONE);
        observations.push((
            graph_on,
            h.state().session.project == before,
            h.state().session.state.selected_keys == selection,
            h.state().session.history.undo.len() == history,
            frames(&h, second),
        ));
    }
    assert!(observations.iter().all(|(_, project, selection, history, _)| *project && *selection && *history), "{observations:?}");
}

#[test]
fn cached_bar_followers_observe_locks_changed_mid_drag() {
    let (mut h, owner, uid) = harness();
    h.state_mut().session.execute_checked("layer.newSolid", json!({"name":"Follower", "width":80, "height":80, "color":"#20a0e0"})).unwrap();
    let follower = h.state().session.active_comp().unwrap().layers.iter().find(|l| l.name == "Follower").unwrap().id;
    h.state_mut().session.execute_checked("layer.select", json!({"layers":[owner.0, follower.0]})).unwrap();
    h.run_steps(3);
    let k = points(&h, uid, false);
    let pps = k[1].x - k[0].x;
    let p = rect(&h, &format!("timeline.layer.{}.bar", owner.0)).center();
    press(&mut h, p, Modifiers::NONE);
    move_to(&mut h, p + vec2(pps * 0.3, 0.0));
    let c = h.state().session.active_comp().unwrap();
    let first = c.layer(owner).unwrap().start_time;
    assert!(first > Tick(0), "owner bar gesture acquired");
    assert_eq!(c.layer(follower).unwrap().start_time, first, "the editable follower moved");
    ui_lock(&mut h, follower);
    let locked = h.state().session.active_comp().unwrap().layer(follower).unwrap().clone();
    let selection = h.state().session.state.selected_layers.clone();
    move_to(&mut h, p + vec2(pps * 0.6, 0.0));
    release(&mut h, p + vec2(pps * 0.6, 0.0), Modifiers::NONE);
    let c = h.state().session.active_comp().unwrap();
    assert!(c.layer(owner).unwrap().start_time > first, "owner remains editable and continues");
    assert_eq!(c.layer(follower).unwrap(), &locked, "the cached follower is now protected");
    assert_eq!(h.state().session.state.selected_layers, selection);
}

#[test]
fn timeline_time_cells_respect_locked_layers() {
    let mut observations = vec![];
    for (kind, dx) in [("in", 40.0), ("out", -40.0)] {
        let (mut h, id, _) = harness();
        h.state_mut().ui.timeline.columns.insert(kind.into());
        h.run_steps(3);
        let widget = format!("timeline.layer.{}.{kind}", id.0);
        let before = Arc::clone(&h.state().session.project);
        let p = rect(&h, &widget).center();
        drag(&mut h, p, p + vec2(dx, 0.0));
        assert_ne!(h.state().session.project, before, "{kind} unlocked control is interactive");
        h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
        h.run_steps(2);
        ui_lock(&mut h, id);
        h.run_steps(2);
        let before = Arc::clone(&h.state().session.project);
        let history = h.state().session.history.undo.len();
        let p = rect(&h, &widget).center();
        drag(&mut h, p, p + vec2(dx, 0.0));
        observations.push((kind, h.state().session.project == before, h.state().session.history.undo.len() == history));
    }
    assert!(observations.iter().all(|(_, project, history)| *project && *history), "{observations:?}");
}

fn group_toggle_harness(kind: &str) -> (H, LayerId, u64) {
    let (mut h, id, _) = harness();
    let s = &mut h.state_mut().session;
    let uid = match kind {
        "effect" => s.execute_checked("effect.apply", json!({"layers":[id.0], "effect":"Gaussian Blur"})).unwrap()["effects"][0].as_u64().unwrap(),
        "style" => s.execute_checked("layer.style.add", json!({"layers":[id.0], "style":"dropShadow"})).unwrap()["styles"][0]["uid"].as_u64().unwrap(),
        "mask" => s.execute_checked("layer.addMask", json!({"layer":id.0, "shape":"rect"})).unwrap()["mask"].as_u64().unwrap(),
        _ => panic!("unknown fixture kind"),
    };
    let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
    let parent = match kind {
        "effect" => layer.props.sub("effects").unwrap().uid,
        "style" => layer.layer_styles().unwrap().uid,
        "mask" => layer.masks().unwrap().uid,
        _ => panic!("unknown fixture kind"),
    };
    let tl = &mut h.state_mut().ui.timeline;
    tl.layer_reveal.remove(&id.0);
    tl.reveal.clear();
    tl.open_layers.insert(id.0);
    tl.open_groups.insert(parent);
    h.run_steps(3);
    (h, id, uid)
}

fn toggle_point(h: &H, uid: u64, kind: &str) -> Pos2 {
    match kind {
        "effect" => rect(h, &format!("timeline.group.{uid}.fx")).center(),
        "style" => rect(h, &format!("timeline.group.{uid}.eye")).center(),
        "mask" => {
            // The mask inversion checkbox currently lacks an automation id. Its centre is
            // cw.switches+100: mode starts at +4 (width80), gap8, checkbox width16.
            let row = rect(h, &format!("timeline.group.{uid}.name"));
            pos2(row.max.x + 100.0, row.center().y)
        }
        _ => panic!("unknown fixture kind"),
    }
}

#[test]
fn timeline_effect_style_and_mask_toggles_respect_locked_layers() {
    let mut observations = vec![];
    for kind in ["effect", "style", "mask"] {
        let (mut h, id, uid) = group_toggle_harness(kind);
        let before = Arc::clone(&h.state().session.project);
        let p = toggle_point(&h, uid, kind);
        click(&mut h, p);
        assert_ne!(h.state().session.project, before, "{kind} unlocked toggle control");
        h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
        h.run_steps(2);
        ui_lock(&mut h, id);
        h.run_steps(2);
        let before = Arc::clone(&h.state().session.project);
        let history = h.state().session.history.undo.len();
        let p = toggle_point(&h, uid, kind);
        click(&mut h, p);
        observations.push((kind, h.state().session.project == before, h.state().session.history.undo.len() == history));
    }
    assert!(observations.iter().all(|(_, project, history)| *project && *history), "{observations:?}");
}

#[test]
fn time_reverse_context_menu_preserves_locked_mixed_keys() {
    let (mut h, owner, uid, second) = mixed(false);
    let other_uid = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute_checked(
            "keys.select",
            json!({"keys":[
        {"layer":owner.0, "prop":uid, "time":0},
        {"layer":second.0, "prop":other_uid, "time":0},
        {"layer":owner.0, "prop":uid, "time":1},
        {"layer":second.0, "prop":other_uid, "time":1}
    ], "selectProperties":true}),
        )
        .unwrap();
    ui_lock(&mut h, owner);
    h.run_steps(3);
    let before = Arc::clone(&h.state().session.project);
    let locked = h.state().session.active_comp().unwrap().layer(owner).unwrap().clone();
    let unlocked = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().keys.clone();
    let history = h.state().session.history.undo.len();
    // Open the menu on an editable selected key, so guarding a locked hit remains compatible.
    let p = points(&h, other_uid, false)[1];
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    h.get_by_label("Time-Reverse Keyframes").click();
    h.run_steps(3);
    let c = h.state().session.active_comp().unwrap();
    assert_ne!(c.layer(second).unwrap().props.prop("transform/opacity").unwrap().keys, unlocked, "editable keys are reversed");
    assert_eq!(c.layer(owner).unwrap(), &locked, "locked selected keys are preserved");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before);
}

#[test]
fn time_reverse_preserves_mixed_selection_identity_and_one_undo() {
    let (mut h, owner, uid, second) = mixed(false);
    let other_uid = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut()
        .session
        .execute_checked(
            "keys.select",
            json!({"keys":[
        {"layer":owner.0, "prop":uid, "time":0}, {"layer":second.0, "prop":other_uid, "time":0},
        {"layer":owner.0, "prop":uid, "time":1}, {"layer":second.0, "prop":other_uid, "time":1}
    ], "selectProperties":true}),
        )
        .unwrap();
    ui_lock(&mut h, owner);
    let selection = h.state().session.state.selected_keys.clone();
    let expected: Vec<_> = selection
        .iter()
        .map(|key| effectcraft_engine::KeyRef { time: if key.layer == owner { key.time } else { Tick::from_seconds_f64(1.0) - key.time }, ..*key })
        .collect();
    let before = Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    invoke(&mut h, "keys.timeReverse", json!({}));
    assert_eq!(h.state().session.state.selected_keys, expected, "mixed order follows each original editable key through reversal");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before);
}

#[test]
fn alt_scale_stops_when_the_dragged_owner_becomes_locked() {
    let (mut h, owner, uid, second) = mixed(false);
    let other_uid = h.state().session.active_comp().unwrap().layer(second).unwrap().props.prop("transform/opacity").unwrap().uid;
    let select_group = |h: &mut H| {
        h.state_mut()
            .session
            .execute_checked(
                "keys.select",
                json!({"keys":[
        {"layer":owner.0,"prop":uid,"time":0}, {"layer":second.0,"prop":other_uid,"time":0},
        {"layer":owner.0,"prop":uid,"time":1}, {"layer":second.0,"prop":other_uid,"time":1}
    ],"selectProperties":true}),
            )
            .unwrap()
    };
    select_group(&mut h);
    h.run_steps(2);
    let k = points(&h, uid, false);
    let pps = k[1].x - k[0].x;
    let mods = Modifiers { alt: true, ..Modifiers::NONE };
    let original = Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    press(&mut h, k[1], mods);
    move_to(&mut h, k[1] + vec2(pps * 0.1, 0.0));
    assert_eq!((frames(&h, owner), frames(&h, second)), (vec![0, 33], vec![0, 33]), "unlocked Alt scale was acquired");
    move_to(&mut h, k[1] + vec2(pps * 0.3, 0.0));
    release(&mut h, k[1] + vec2(pps * 0.3, 0.0), mods);
    assert_eq!((frames(&h, owner), frames(&h, second)), (vec![0, 39], vec![0, 39]), "unlocked control follows held motion and completes");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, original);
    select_group(&mut h);
    h.run_steps(2);
    let k = points(&h, uid, false);
    press(&mut h, k[1], mods);
    move_to(&mut h, k[1] + vec2(pps * 0.1, 0.0));
    assert_eq!((frames(&h, owner), frames(&h, second)), (vec![0, 33], vec![0, 33]), "second Alt gesture acquired");
    ui_lock(&mut h, owner);
    let protected = Arc::clone(&h.state().session.project);
    let selection = h.state().session.state.selected_keys.clone();
    let history = h.state().session.history.undo.len();
    move_to(&mut h, k[1] + vec2(pps * 0.3, 0.0));
    release(&mut h, k[1] + vec2(pps * 0.4, 0.0), mods);
    assert_eq!(
        (
            h.state().session.project == protected,
            h.state().session.state.selected_keys == selection,
            h.state().session.history.undo.len() == history,
            frames(&h, second)
        ),
        (true, true, true, vec![0, 33]),
        "locking the dragged owner ends Alt scale"
    );
}

#[test]
fn time_reverse_restores_caller_order_at_imported_tick_extrema() {
    let (mut h, locked, locked_uid, editable) = mixed(false);
    let cid = h.state().session.active_comp_id().unwrap();
    let editable_uid = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/opacity").unwrap().uid;
    // Small imported project: three exact i64 tick times, including off-screen endpoints.
    let mut imported = h.state().session.project.as_ref().clone();
    let prop = imported.comp_mut(cid).unwrap().layer_mut(editable).unwrap().props.find_mut(editable_uid).unwrap();
    let mut lo = prop.keys[0].clone();
    let mut middle = lo.clone();
    let mut hi = prop.keys[1].clone();
    lo.time = Tick(i64::MIN);
    middle.time = Tick(0);
    middle.value = effectcraft_engine::keyframe::Value::Scalar(50.0);
    hi.time = Tick(i64::MAX);
    prop.keys = vec![lo, middle, hi];
    let serialized = serde_json::to_vec(&imported).unwrap();
    let imported = serde_json::from_slice::<effectcraft_engine::project::Project>(&serialized).unwrap();
    h.state_mut().session.project = Arc::new(imported);
    h.state_mut().session.execute_checked("prop.select", json!({"layer":editable.0,"prop":editable_uid,"selectKeys":true})).unwrap();
    h.state_mut()
        .session
        .execute_checked("keys.select", json!({"keys":[{"layer":locked.0,"prop":locked_uid,"time":0}],"add":true,"selectProperties":true}))
        .unwrap();
    let selected = h.state().session.state.selected_keys.clone();
    assert_eq!(
        selected.iter().map(|k| (k.layer, k.time)).collect::<Vec<_>>(),
        [(editable, Tick(i64::MIN)), (editable, Tick(0)), (editable, Tick(i64::MAX)), (locked, Tick(0))],
        "public selection retains exact imported endpoints"
    );
    ui_lock(&mut h, locked);
    h.run_steps(3);
    let before = Arc::clone(&h.state().session.project);
    let protected = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    let history = h.state().session.history.undo.len();
    // The visible selected middle key opens the same real menu as an ordinary project.
    let p = rect(&h, &format!("timeline.key.{editable_uid}.0")).center();
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    h.get_by_label("Time-Reverse Keyframes").click();
    h.run_steps(3);
    let expected = vec![
        effectcraft_engine::KeyRef { layer: editable, prop: editable_uid, time: Tick(i64::MAX) },
        effectcraft_engine::KeyRef { layer: editable, prop: editable_uid, time: Tick(-1) },
        effectcraft_engine::KeyRef { layer: editable, prop: editable_uid, time: Tick(i64::MIN) },
        effectcraft_engine::KeyRef { layer: locked, prop: locked_uid, time: Tick(0) },
    ];
    assert_eq!(h.state().session.state.selected_keys, expected, "the engine mirrors MIN to MAX, zero to -1, and MAX to MIN in caller order");
    assert_eq!(h.state().session.active_comp().unwrap().layer(locked).unwrap(), &protected);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before);
}
