//! The new transform properties use ordinary Timeline rows, animation and dimension visibility.
use effectcraft_engine::{Session, project::LayerId};
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;
fn harness() -> (Harness<'static, EffectcraftApp>, u64, u64, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":2})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"width":16,"height":8,"color":"#00ff00","name":"Skew"})).unwrap()["layer"].as_u64().unwrap();
    let l = s.active_comp().unwrap().layer(LayerId(layer)).unwrap();
    let tr = l.transform().unwrap();
    let group = tr.uid;
    let skew = tr.get("skew").unwrap().uid;
    let axis = tr.get("skewAxis").unwrap().uid;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.state_mut().ui.timeline.open_layers.insert(layer);
    h.state_mut().ui.timeline.open_groups.insert(group);
    h.run_steps(4);
    (h, layer, skew, axis)
}
fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let entry = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(entry.rect[0], entry.rect[1]), vec2(entry.rect[2], entry.rect[3]))
}
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=12 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 12.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
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
#[test]
fn layer_skew_numeric_and_axis_rows_edit_undo_and_hide_in_three_d() {
    let (mut h, layer, skew, axis) = harness();
    let panel = rect(&h, "panel.Timeline");
    for uid in [skew, axis] {
        let field = rect(&h, &format!("timeline.prop.{uid}.value"));
        assert!(panel.contains(field.min) && panel.contains(field.max), "ordinary numeric row must fit inside Timeline");
        assert!(h.state().auto.find(&format!("timeline.prop.{uid}.stopwatch")).is_some());
    }
    let before = h.state().session.history.undo.len();
    let r = rect(&h, &format!("timeline.prop.{skew}.value"));
    drag(&mut h, r.center(), r.center() + vec2(80.0, 0.0));
    let value = h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skew"})).unwrap()["value"].as_f64().unwrap();
    assert!(value > 20.0 && value < 85.0, "a real scrub changes Skew within its established slider range: {value}");
    assert_eq!(h.state().session.history.undo.len(), before + 1, "one scrub is one undo");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skew"})).unwrap()["value"], json!(0.0));
    let r = rect(&h, &format!("timeline.prop.{axis}.value"));
    drag(&mut h, r.center(), r.center() + vec2(30.0, 0.0));
    assert!(h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skewAxis"})).unwrap()["value"].as_f64().unwrap() > 10.0);
    let watch = rect(&h, &format!("timeline.prop.{skew}.stopwatch")).center();
    click(&mut h, watch);
    assert!(h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skew"})).unwrap()["animated"].as_bool().unwrap());
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"threeD","value":true})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.prop.{skew}.value")).is_none());
    assert!(h.state().auto.find(&format!("timeline.prop.{axis}.value")).is_none());
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"threeD","value":false})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.prop.{skew}.value")).is_some());
    assert!(h.state().auto.find(&format!("timeline.prop.{axis}.value")).is_some());
}
#[test]
#[ignore = "visual proof runs explicitly with the fresh normal UI graph"]
fn snapshot_layer_skew_standard_timeline_rows() {
    let (mut h, layer, _, _) = harness();
    h.state_mut().session.execute("prop.set", json!({"layer":layer,"path":"transform/skew","value":25})).unwrap();
    h.run_steps(3);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/targettest-out");
    std::fs::create_dir_all(&out).unwrap();
    h.render().expect("the native snapshot renderer must be available").save(out.join("layer-skew-timeline.png")).unwrap();
}

#[test]
fn layer_skew_properties_panel_retains_two_d_values_and_hides_them_in_three_d() {
    let (mut h, layer, skew, axis) = harness();
    h.state_mut().show_panel(effectcraft_ui_egui::dock::PanelKind::Properties);
    h.run_steps(4);
    let panel = rect(&h, "panel.Properties");
    for uid in [skew, axis] {
        let field = rect(&h, &format!("properties.prop.{uid}.value"));
        assert!(panel.contains(field.min) && panel.contains(field.max), "the standard transform field must fit in Properties");
        assert!(h.state().auto.find(&format!("properties.prop.{uid}.stopwatch")).is_some());
    }
    assert!(h.state().auto.find(&format!("properties.prop.{axis}.revolutions")).is_some(), "Skew Axis uses the existing angle editor");
    let before = h.state().session.history.undo.len();
    let r = rect(&h, &format!("properties.prop.{skew}.value"));
    drag(&mut h, r.center(), r.center() + vec2(60.0, 0.0));
    let skew_value = h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skew"})).unwrap()["value"].as_f64().unwrap();
    assert!(skew_value > 10.0 && skew_value <= 85.0);
    assert_eq!(h.state().session.history.undo.len(), before + 1);
    let r = rect(&h, &format!("properties.prop.{axis}.value"));
    drag(&mut h, r.center(), r.center() + vec2(30.0, 0.0));
    let axis_value = h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skewAxis"})).unwrap()["value"].as_f64().unwrap();
    assert!(axis_value > 10.0);
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"threeD","value":true})).unwrap();
    h.run_steps(3);
    for uid in [skew, axis] {
        assert!(h.state().auto.find(&format!("properties.prop.{uid}.value")).is_none());
        assert!(h.state().auto.find(&format!("properties.prop.{uid}.stopwatch")).is_none());
    }
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"threeD","value":false})).unwrap();
    h.run_steps(3);
    for uid in [skew, axis] {
        assert!(h.state().auto.find(&format!("properties.prop.{uid}.value")).is_some());
    }
    assert_eq!(h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skew"})).unwrap()["value"], json!(skew_value));
    assert_eq!(h.state_mut().session.execute("prop.get", json!({"layer":layer,"path":"transform/skewAxis"})).unwrap()["value"], json!(axis_value));
}
