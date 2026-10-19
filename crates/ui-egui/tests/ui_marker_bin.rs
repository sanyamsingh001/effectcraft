//! Composition marker creation from the Timeline's marker bin (#275).
use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Markers", "width":32, "height":32, "duration":4, "frameRate":25})).unwrap();
    s.execute("layer.newSolid", json!({"name":"Selected layer"})).unwrap();
    s.execute("time.set", json!({"time":1})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn center(h: &Harness<'_, EffectcraftApp>, id: &str) -> Pos2 {
    let r = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}")).rect;
    egui::pos2(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn pointer(h: &mut Harness<'_, EffectcraftApp>, p: Pos2, pressed: bool) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

#[test]
fn clicking_the_bin_adds_a_composition_marker_and_undo_removes_it() {
    let mut h = harness();
    let p = center(&h, "timeline.markerBin");
    pointer(&mut h, p, true);
    pointer(&mut h, p, false);
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    assert_eq!(comp.markers.len(), 1);
    assert_eq!(comp.markers[0].time.seconds(), 1.0);
    assert!(comp.layers.iter().all(|l| l.markers.is_empty()), "selection does not create a layer marker");
    h.state_mut().session.undo();
    assert!(h.state().session.active_comp().unwrap().markers.is_empty());
}

#[test]
fn dragging_the_bin_places_a_marker_at_the_drop_time() {
    let mut h = harness();
    let p = center(&h, "timeline.markerBin");
    let destination = center(&h, "timeline.cti");
    pointer(&mut h, p, true);
    h.input_mut().events.push(Event::PointerMoved(destination));
    h.run_steps(2);
    assert!(h.state().session.active_comp().unwrap().markers.is_empty(), "drag preview does not edit the project");
    pointer(&mut h, destination, false);
    h.run_steps(2);
    let comp = h.state().session.active_comp().unwrap();
    assert_eq!(comp.markers.len(), 1);
    assert_eq!(comp.markers[0].time.seconds(), 1.0);
}

#[test]
fn dropping_the_bin_outside_the_time_graph_leaves_no_marker() {
    let mut h = harness();
    let p = center(&h, "timeline.markerBin");
    let destination = egui::pos2(5.0, 5.0);
    pointer(&mut h, p, true);
    h.input_mut().events.push(Event::PointerMoved(destination));
    h.run_steps(2);
    pointer(&mut h, destination, false);
    h.run_steps(2);
    assert!(h.state().session.active_comp().unwrap().markers.is_empty());
}
