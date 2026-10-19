//! Properties animation controls act on selected editable layers and undo together.
use effectcraft_engine::{Session, project::LayerId, time::Tick};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

type H = Harness<'static, EffectcraftApp>;

fn invoke(h: &mut H, id: &str, params: serde_json::Value) {
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, id, params).unwrap();
}

fn rect(h: &H, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing automation widget {id}"));
    let r = Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]));
    assert!(r.is_finite() && r.is_positive(), "invalid automation rectangle {id}");
    r
}

fn click(h: &mut H, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
}

fn scene(count: usize) -> (H, Vec<LayerId>) {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Bulk", "width":640, "height":360, "duration":4, "frameRate":30})).unwrap();
    let mut ids = vec![];
    for (i, x) in [128.0, 320.0, 512.0].into_iter().take(count).enumerate() {
        let reply = session
            .execute_checked(
                "layer.newShape",
                json!({
                    "kind":"rect", "name":format!("Square {}",i+1), "size":[56,56],
                    "position":[x,180], "fill":"#e04020"
                }),
            )
            .unwrap();
        ids.push(LayerId(reply["layer"].as_u64().unwrap()));
    }
    session.execute_checked("edit.deselectAll", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
    h.state_mut().ui.focused = PanelKind::Viewer(0);
    // Public dispatcher path used by Ctrl+A outside the Project panel.
    invoke(&mut h, "edit.selectAll", json!({}));
    assert_eq!(h.state().session.state.selected_layers.len(), count);
    assert!(ids.iter().all(|id| h.state().session.state.selected_layers.contains(id)));
    h.state_mut().show_panel(PanelKind::Properties);
    h.state_mut().ui.maximized = Some(PanelKind::Properties);
    h.run_steps(3);
    (h, ids)
}

fn position_uid(h: &H, id: LayerId) -> u64 {
    h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap().uid
}

fn position_keys(h: &H, id: LayerId) -> Vec<Tick> {
    h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/position").unwrap().keys.iter().map(|k| k.time).collect()
}

fn initial_positions(h: &H, ids: &[LayerId]) -> Vec<Vec<f64>> {
    ids.iter().map(|id| h.state().session.active_comp().unwrap().layer(*id).unwrap().props.prop("transform/position").unwrap().value.components()).collect()
}

fn clock(h: &mut H) {
    let primary = h.state().session.state.selected_layers[0];
    let uid = position_uid(h, primary);
    let p = rect(h, &format!("properties.prop.{uid}.stopwatch")).center();
    click(h, p);
}

fn viewer_move(h: &mut H, ids: &[LayerId]) {
    h.state_mut().session.execute_checked("time.set", json!({"time":1})).unwrap();
    h.state_mut().ui.maximized = Some(PanelKind::Viewer(0));
    h.state_mut().show_panel(PanelKind::Viewer(0));
    h.run_steps(4);
    let old = initial_positions(h, ids);
    let comp = rect(h, "viewer.comp");
    let scale = comp.width() / 640.0;
    // The square interior stays away from its eight scale handles and path vertices.
    let from = comp.min + vec2((old[0][0] as f32 + 9.0) * scale, (old[0][1] as f32 + 7.0) * scale);
    let to = from + vec2(40.0 * scale, 20.0 * scale);
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().session.state.selected_layers.len(), ids.len(), "viewer move preserves the group selection");
    let c = h.state().session.active_comp().unwrap();
    let cid = h.state().session.active_comp_id().unwrap();
    let ectx = effectcraft_engine::render::EvalCtx {
        project: &h.state().session.project,
        comp_id: cid,
        comp: c,
        time: h.state().session.time(),
        expr: h.state().session.expr.as_deref(),
        footage: None,
    };
    for (id, old) in ids.iter().zip(old) {
        let layer = c.layer(*id).unwrap();
        let p = ectx.value(layer, layer.props.prop("transform/position").unwrap()).components();
        assert!(p[0] > old[0] + 20.0 && p[1] > old[1] + 5.0, "viewer moved selected square {id:?}: {old:?}->{p:?}");
    }
}

#[test]
fn single_properties_stopwatch_then_viewer_move_autokeys() {
    let (mut h, ids) = scene(1);
    assert_eq!(h.state().auto.find("properties.transform").unwrap().label, "Layer Transform");
    clock(&mut h);
    assert_eq!(position_keys(&h, ids[0]), [Tick(0)]);
    viewer_move(&mut h, &ids);
    assert_eq!(position_keys(&h, ids[0]), [Tick(0), Tick::from_seconds_f64(1.0)]);
}

#[test]
fn existing_bulk_position_shortcut_then_viewer_move_autokeys() {
    let (mut h, ids) = scene(3);
    invoke(&mut h, "timeline.keyAt.position", json!({}));
    assert!(ids.iter().all(|id| position_keys(&h, *id) == [Tick(0)]), "existing bulk keyframe command animates all three");
    viewer_move(&mut h, &ids);
    assert!(ids.iter().all(|id| position_keys(&h, *id) == [Tick(0), Tick::from_seconds_f64(1.0)]));
}

#[test]
fn bulk_properties_stopwatch_then_viewer_move_autokeys_all_selected() {
    let (mut h, ids) = scene(3);
    assert_eq!(h.state().auto.find("properties.transform").unwrap().label, "3 Selected Objects");
    let selection = h.state().session.state.selected_layers.clone();
    let initial = std::sync::Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    clock(&mut h);
    let start_keys: Vec<_> = ids.iter().map(|id| position_keys(&h, *id)).collect();
    assert_eq!(h.state().session.state.selected_layers, selection, "clock preserves the group selection");
    assert_eq!(h.state().session.history.undo.len(), history + 1, "one stopwatch click is one edit");
    let animated = std::sync::Arc::clone(&h.state().session.project);
    viewer_move(&mut h, &ids);
    let final_keys: Vec<_> = ids.iter().map(|id| position_keys(&h, *id)).collect();
    assert_eq!(
        (start_keys, final_keys),
        (vec![vec![Tick(0)]; 3], vec![vec![Tick(0), Tick::from_seconds_f64(1.0)]; 3]),
        "Properties stopwatch must animate every selected square before the existing viewer move"
    );
    assert_eq!(h.state().session.history.undo.len(), history + 2, "one clock and one viewer gesture");
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, animated, "viewer group drag undoes together");
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, initial, "bulk stopwatch undoes together");
}

#[test]
fn bulk_properties_clock_and_diamond_keep_locked_counterpart_unchanged() {
    let (mut h, ids) = scene(3);
    let locked = ids[1];
    invoke(&mut h, "layer.setSwitch", json!({"layers":[locked.0],"switch":"lock","value":true}));
    let protected = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    let selection = h.state().session.state.selected_layers.clone();
    let before = std::sync::Arc::clone(&h.state().session.project);
    let history = h.state().session.history.undo.len();
    clock(&mut h);
    assert_eq!(h.state().session.state.selected_layers, selection);
    assert_eq!(h.state().session.active_comp().unwrap().layer(locked).unwrap(), &protected);
    assert!(ids.iter().filter(|id| **id != locked).all(|id| position_keys(&h, *id) == [Tick(0)]));
    assert_eq!(h.state().session.history.undo.len(), history + 1, "bulk clock is one undo step");
    let animated = std::sync::Arc::clone(&h.state().session.project);
    viewer_move_editable_only(&mut h, &ids, locked);
    assert_eq!(h.state().session.active_comp().unwrap().layer(locked).unwrap(), &protected);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, animated, "one undo reverts the complete group movement");
    // At one second, the same visible key diamond writes all editable counterparts.
    h.state_mut().ui.maximized = Some(PanelKind::Properties);
    h.state_mut().show_panel(PanelKind::Properties);
    h.run_steps(3);
    let primary = h.state().session.state.selected_layers[0];
    let uid = position_uid(&h, primary);
    let key = rect(&h, &format!("properties.prop.{uid}.keyKey")).center();
    click(&mut h, key);
    assert_eq!(h.state().session.history.undo.len(), history + 2, "bulk diamond is one undo step");
    assert!(ids.iter().filter(|id| **id != locked).all(|id| position_keys(&h, *id) == [Tick(0), Tick::from_seconds_f64(1.0)]));
    assert_eq!(h.state().session.active_comp().unwrap().layer(locked).unwrap(), &protected);
    assert_eq!(h.state().session.state.selected_layers, selection);
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, animated, "one undo reverts the complete bulk diamond");
    h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project, before, "one undo reverts the bulk stopwatch");
}

fn viewer_move_editable_only(h: &mut H, ids: &[LayerId], locked: LayerId) {
    h.state_mut().session.execute_checked("time.set", json!({"time":1})).unwrap();
    h.state_mut().ui.maximized = Some(PanelKind::Viewer(0));
    h.state_mut().show_panel(PanelKind::Viewer(0));
    h.run_steps(4);
    let old = initial_positions(h, ids);
    let comp = rect(h, "viewer.comp");
    let scale = comp.width() / 640.0;
    let from = comp.min + vec2((old[0][0] as f32 + 9.0) * scale, (old[0][1] as f32 + 7.0) * scale);
    let to = from + vec2(40.0 * scale, 20.0 * scale);
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let c = h.state().session.active_comp().unwrap();
    let ectx = effectcraft_engine::render::EvalCtx {
        project: &h.state().session.project,
        comp_id: h.state().session.active_comp_id().unwrap(),
        comp: c,
        time: h.state().session.time(),
        expr: h.state().session.expr.as_deref(),
        footage: None,
    };
    for (id, old) in ids.iter().zip(old) {
        let layer = c.layer(*id).unwrap();
        let value = ectx.value(layer, layer.props.prop("transform/position").unwrap()).components();
        if *id == locked {
            assert_eq!(value, old, "locked position stays fixed")
        } else {
            assert!(value[0] > old[0] + 20.0 && value[1] > old[1] + 5.0, "editable counterpart moved")
        }
    }
}
