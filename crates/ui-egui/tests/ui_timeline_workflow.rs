//! Timeline selection and stopwatch gestures, exercised with real pointer input.
use effectcraft_engine::{Session, project::LayerId};
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

fn harness() -> (Harness<'static, EffectcraftApp>, Vec<u64>) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Main", "width":320,"height":180,"duration":4})).unwrap();
    let mut ids: Vec<_> = ["D", "C", "B", "A"]
        .iter()
        .map(|name| s.execute("layer.newSolid", json!({"name":name,"color":"#406080"})).unwrap()["layer"].as_u64().unwrap())
        .collect();
    ids.reverse();
    s.execute("edit.deselectAll", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, ids)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn selected(h: &Harness<'_, EffectcraftApp>) -> Vec<u64> {
    h.state().session.state.selected_layers.iter().map(|l| l.0).collect()
}

#[test]
fn shift_clicking_duration_strips_selects_a_range_from_either_pane() {
    let (mut h, ids) = harness();
    let click_bar = |h: &mut Harness<'_, EffectcraftApp>, i: usize, modifiers| {
        let at = rect(h, &format!("timeline.layer.{}.bar", ids[i])).center();
        click(h, at, modifiers);
    };
    click_bar(&mut h, 0, Modifiers::NONE);
    click_bar(&mut h, 3, Modifiers::SHIFT);
    assert_eq!(selected(&h), ids, "strip selection includes intervening rows");

    // Repeating Shift keeps the original anchor rather than the last endpoint.
    click_bar(&mut h, 2, Modifiers::SHIFT);
    assert_eq!(selected(&h), ids[..3]);

    // The outline and strips share one anchor, and locked layers are skipped.
    let a = rect(&h, &format!("timeline.layer.{}.row", ids[0]));
    click(&mut h, pos2(a.min.x + 210.0, a.center().y), Modifiers::NONE);
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[ids[1]], "switch":"locked", "value":true})).unwrap();
    h.run_steps(2);
    click_bar(&mut h, 3, Modifiers::SHIFT);
    assert_eq!(selected(&h), [ids[0], ids[2], ids[3]]);

    // Ctrl/Cmd-click toggles a strip already in the selection.
    click_bar(&mut h, 2, Modifiers::COMMAND);
    assert_eq!(selected(&h), [ids[0], ids[3]]);
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, points: &[Pos2]) {
    let first = points[0];
    h.event(Event::PointerMoved(first));
    h.step();
    h.event(Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for &p in points.iter().skip(1) {
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(Event::PointerButton { pos: *points.last().unwrap(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn dragging_stopwatches_sets_one_state_and_undoes_as_one_gesture() {
    let (mut h, ids) = harness();
    let layer = ids[0];
    h.state_mut().ui.timeline.open_layers.insert(layer);
    let transform = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().transform().unwrap().uid;
    h.state_mut().ui.timeline.open_groups.insert(transform);
    let paths = ["transform/anchor", "transform/position", "transform/scale", "transform/rotation", "transform/opacity"];
    let uids: Vec<_> = paths.iter().map(|p| h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop(p).unwrap().uid).collect();
    h.state_mut().session.execute("prop.toggleAnimation", json!({"layer":layer,"prop":uids[1]})).unwrap();
    h.run_steps(3);
    let watch = |h: &Harness<'_, EffectcraftApp>, i: usize| rect(h, &format!("timeline.prop.{}.stopwatch", uids[i])).center();
    let animated = |h: &Harness<'_, EffectcraftApp>| -> Vec<bool> {
        let layer = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap();
        uids.iter().map(|u| layer.props.find(*u).unwrap().is_animated()).collect()
    };
    let before = h.state().session.history.undo.len();
    let (a, c, e) = (watch(&h, 0), watch(&h, 2), watch(&h, 4));
    // Two moves cross five properties; returning to the first never toggles it again.
    drag(&mut h, &[a, c, e, a]);
    assert_eq!(animated(&h), [true; 5]);
    assert_eq!(h.state().session.history.undo.len(), before + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(animated(&h), [false, true, false, false, false]);

    let (b, e) = (watch(&h, 1), watch(&h, 4));
    drag(&mut h, &[b, e]);
    assert_eq!(animated(&h), [false; 5], "every crossed watch takes the first watch's off state");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(animated(&h), [false, true, false, false, false]);
    let d = watch(&h, 3);
    click(&mut h, d, Modifiers::NONE);
    assert_eq!(animated(&h), [false, true, false, true, false], "plain click toggles once");
    let a = watch(&h, 0);
    click(&mut h, a, Modifiers::ALT);
    assert!(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(uids[0]).unwrap().expr.is_some());
    assert_eq!(animated(&h), [false, true, false, true, false], "Alt-click still adds an expression");
}

#[test]
fn work_area_context_menu_trims_the_comp_and_undo_restores_it() {
    let (mut h, _) = harness();
    h.state_mut().session.execute("comp.workArea", json!({"start": 0.5, "end": 3.5})).unwrap();
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    let original = (comp.duration, comp.work_area);
    let at = rect(&h, "timeline.workArea.bar").center();
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label("Trim Comp to Work Area").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active_comp().unwrap().duration.seconds(), 3.0);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    let restored = h.state().session.active_comp().unwrap();
    assert_eq!((restored.duration, restored.work_area), original);
}

#[test]
fn stopwatch_gestures_handle_batched_press_and_release_without_prepress_motion() {
    let (mut h, ids) = harness();
    let layer = ids[0];
    let transform = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().transform().unwrap().uid;
    h.state_mut().ui.timeline.open_layers.insert(layer);
    h.state_mut().ui.timeline.open_groups.insert(transform);
    let uids: Vec<_> = ["anchor", "position", "scale", "rotation", "opacity"]
        .iter()
        .map(|p| h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop(&format!("transform/{p}")).unwrap().uid)
        .collect();
    h.run_steps(3);
    let watch = |h: &Harness<'_, EffectcraftApp>, i: usize| rect(h, &format!("timeline.prop.{}.stopwatch", uids[i])).center();
    let states = |h: &Harness<'_, EffectcraftApp>| -> Vec<bool> {
        let l = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap();
        uids.iter().map(|u| l.props.find(*u).unwrap().is_animated()).collect()
    };
    let (a, e) = (watch(&h, 0), watch(&h, 4));
    h.event(Event::PointerMoved(e));
    h.step();
    h.event(Event::PointerMoved(a));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
    assert_eq!(states(&h), [true, false, false, false, false], "a quick click affects only the pressed watch");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);

    h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(e));
    h.event(Event::PointerButton { pos: e, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(states(&h), [true; 5], "the release frame's final movement visits every crossed watch");
}

#[test]
fn shift_dragging_a_strip_moves_the_selected_range_consistently() {
    let (mut h, ids) = harness();
    let a = rect(&h, &format!("timeline.layer.{}.bar", ids[0])).center();
    click(&mut h, a, Modifiers::NONE);
    let d = rect(&h, &format!("timeline.layer.{}.bar", ids[3])).center();
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(Event::PointerMoved(d));
    h.step();
    h.event(Event::PointerButton { pos: d, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT });
    h.step();
    for offset in [20.0, 40.0, 60.0] {
        h.event(Event::PointerMoved(d + vec2(offset, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: d + vec2(60.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
    assert_eq!(selected(&h), ids);
    let c = h.state().session.active_comp().unwrap();
    let starts: Vec<_> = ids.iter().map(|id| c.layer(LayerId(*id)).unwrap().start_time).collect();
    assert!(starts[0].seconds() > 0.0);
    assert!(starts.iter().all(|t| *t == starts[0]), "every frame of the drag moves the same range: {starts:?}");
}

fn expression_harness() -> (Harness<'static, EffectcraftApp>, u64, u64, u64) {
    let (mut h, ids) = harness();
    let layer = ids[0];
    h.state_mut().session.execute("prop.setExpression", json!({"layer":layer,"path":"transform/rotation", "expression":"value"})).unwrap();
    let l = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap();
    let transform = l.transform().unwrap().uid;
    let rotation = l.props.prop("transform/rotation").unwrap().uid;
    let opacity = l.props.prop("transform/opacity").unwrap().uid;
    h.state_mut().ui.timeline.open_layers.insert(layer);
    h.state_mut().ui.timeline.open_groups.insert(transform);
    h.run_steps(3);
    let at = rect(&h, &format!("timeline.prop.{rotation}.expression")).center();
    click(&mut h, at, Modifiers::NONE);
    let edit_id = egui::Id::new(("expr-edit", rotation));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, edit_id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(5))));
    state.store(&h.ctx, edit_id);
    h.event(Event::Text(" + 2".into()));
    h.run_steps(2);
    (h, layer, rotation, opacity)
}

#[test]
fn expression_pick_whip_replaces_the_active_selection_and_undo_restores_the_expression() {
    let (mut h, layer, rotation, opacity) = expression_harness();
    let edit_id = egui::Id::new(("expr-edit", rotation));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, edit_id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(8), egui::text::CCursor::new(9))));
    state.store(&h.ctx, edit_id);
    let before = h.state().session.history.undo.len();
    let from = rect(&h, &format!("timeline.prop.{rotation}.pickWhip")).center();
    let to = rect(&h, &format!("timeline.prop.{opacity}.name")).center();
    drag(&mut h, &[from, from + vec2(15.0, 0.0), to]);
    let expression = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(rotation).unwrap().expr.as_ref().unwrap().text.clone();
    assert!(expression.starts_with("value + "), "the uncommitted editor text is kept: {expression}");
    assert!(expression.contains("opacity"), "{expression}");
    assert!(!expression.ends_with('2'), "the selection is replaced: {expression}");
    assert_eq!(h.state().session.history.undo.len(), before + 1, "the draft and insertion commit together");
    let at = rect(&h, &format!("timeline.layer.{layer}.bar")).center();
    click(&mut h, at, Modifiers::NONE);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(rotation).unwrap().expr.as_ref().unwrap().text, "value");
}

#[test]
fn canceling_expression_pick_whip_keeps_the_draft_and_escape_cancels_the_edit() {
    let (mut h, layer, rotation, _) = expression_harness();
    let before = h.state().session.history.undo.len();
    let from = rect(&h, &format!("timeline.prop.{rotation}.pickWhip")).center();
    // The empty graph rectangle also contains property rows; release below every row.
    let empty = rect(&h, "timeline.empty");
    let to = pos2(empty.center().x, empty.max.y - 10.0);
    drag(&mut h, &[from, from + vec2(15.0, 0.0), to]);
    assert_eq!(h.ctx.data(|d| d.get_temp::<String>(egui::Id::new(("expr-buf", rotation)))).as_deref(), Some("value + 2"));
    assert_eq!(h.state().session.history.undo.len(), before);
    for pressed in [true, false] {
        h.event(Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed, repeat: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert!(h.ctx.data(|d| d.get_temp::<String>(egui::Id::new(("expr-buf", rotation)))).is_none());
    assert_eq!(h.state().session.history.undo.len(), before);
    assert_eq!(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(rotation).unwrap().expr.as_ref().unwrap().text, "value");
}

#[test]
fn stopwatch_sweeps_leave_expanded_locked_layers_unchanged() {
    let (mut h, ids) = harness();
    h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
    let (editable, locked) = (LayerId(ids[0]), LayerId(ids[1]));
    for layer in [editable, locked] {
        let group = h.state().session.active_comp().unwrap().layer(layer).unwrap().transform().unwrap().uid;
        h.state_mut().ui.timeline.open_layers.insert(layer.0);
        h.state_mut().ui.timeline.open_groups.insert(group);
    }
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[locked.0],"switch":"locked","value":true})).unwrap();
    let editable_watch = h.state().session.active_comp().unwrap().layer(editable).unwrap().props.prop("transform/opacity").unwrap().uid;
    let locked_watch = h.state().session.active_comp().unwrap().layer(locked).unwrap().props.prop("transform/opacity").unwrap().uid;
    let before = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    h.run_steps(3);
    let from = rect(&h, &format!("timeline.prop.{editable_watch}.stopwatch")).center();
    let to = rect(&h, &format!("timeline.prop.{locked_watch}.stopwatch")).center();
    drag(&mut h, &[from, to]);
    assert!(h.state().session.active_comp().unwrap().layer(editable).unwrap().props.find(editable_watch).unwrap().is_animated());
    assert_eq!(h.state().session.active_comp().unwrap().layer(locked).unwrap(), &before, "crossing a locked layer's stopwatches must preserve its properties");
}

#[test]
fn locked_duration_strips_and_trim_handles_preserve_timing() {
    for kind in ["bar", "bar.in", "bar.out"] {
        let (mut h, ids) = harness();
        let layer = LayerId(ids[0]);
        h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer.0],"switch":"locked","value":true})).unwrap();
        let before = h.state().session.active_comp().unwrap().layer(layer).unwrap().clone();
        h.run_steps(3);
        let bar = rect(&h, &format!("timeline.layer.{}.bar", layer.0));
        let from = rect(&h, &format!("timeline.layer.{}.{kind}", layer.0)).center();
        let distance = bar.width() * if kind == "bar.out" { -0.1 } else { 0.1 };
        drag(&mut h, &[from, from + vec2(distance, 0.0)]);
        assert!(h.state().session.active_comp().unwrap().layer(layer).unwrap() == &before, "a locked layer's {kind} gesture must preserve its timing");
    }
}

#[test]
fn locked_keyframe_diamonds_preserve_existing_and_interpolated_animation() {
    let mut unchanged = Vec::new();
    for time in [0.0, 0.5] {
        let (mut h, ids) = harness();
        let layer = ids[0];
        h.state_mut().ui.maximized = Some(effectcraft_ui_egui::dock::PanelKind::Timeline);
        h.state_mut().ui.timeline.open_layers.insert(layer);
        let transform = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().transform().unwrap().uid;
        h.state_mut().ui.timeline.open_groups.insert(transform);
        let uid = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/opacity").unwrap().uid;
        for (time, value) in [(0.0, 0.0), (1.0, 100.0)] {
            h.state_mut().session.execute("prop.addKey", json!({"layer":layer,"prop":uid,"time":time,"value":value})).unwrap();
        }
        h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"locked","value":true})).unwrap();
        h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(time));
        let before = h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().clone();
        h.run_steps(3);
        let diamond = rect(&h, &format!("timeline.prop.{uid}.addKey")).center();
        click(&mut h, diamond, Modifiers::NONE);
        unchanged.push(h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap() == &before);
    }
    assert_eq!(unchanged, [true, true], "a locked property cannot remove an existing key or add an interpolated key");
}

#[test]
fn editable_group_bar_drags_preserve_layers_locked_from_the_timeline() {
    let (mut h, ids) = harness();
    let editable = LayerId(ids[0]);
    let locked = LayerId(ids[1]);
    h.state_mut().session.execute("layer.select", json!({"layers":[editable.0,locked.0]})).unwrap();
    h.run_steps(2);
    let lock_button = rect(&h, &format!("timeline.layer.{}.lock", locked.0)).center();
    click(&mut h, lock_button, Modifiers::NONE);
    assert!(h.state().session.active_comp().unwrap().layer(locked).unwrap().switches.locked);
    eprintln!("selection after UI lock: {:?}", selected(&h));
    assert_eq!(selected(&h), [editable.0, locked.0], "both rows remain selected after the lock switch");
    let before = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    let project_before = h.state().session.project.clone();
    let history = h.state().session.history.undo.len();
    let editable_start = h.state().session.active_comp().unwrap().layer(editable).unwrap().start_time;
    let bar = rect(&h, &format!("timeline.layer.{}.bar", editable.0));
    let from = bar.center();
    let to = from + vec2(bar.width() * 0.075, 0.0);
    drag(&mut h, &[from, to]);
    let comp = h.state().session.active_comp().unwrap();
    assert!(comp.layer(editable).unwrap().start_time > editable_start, "the editable group's bar actually moved");
    let after = comp.layer(locked).unwrap();
    eprintln!(
        "locked layer timing before={:?}; after={:?}",
        (before.start_time, before.in_point, before.out_point),
        (after.start_time, after.in_point, after.out_point)
    );
    assert!(after == &before, "moving an editable selected bar must preserve the locked selection member");
    assert_eq!(h.state().session.state.selected_layers, [editable, locked]);
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &project_before));
}
