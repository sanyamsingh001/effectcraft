//! Finite source boundaries remain visible when a layer strip extends beyond them.
use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, PointerButton, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness() -> (Harness<'static, EffectcraftApp>, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Source","width":320,"height":180,"frameRate":24,"duration":2})).unwrap();
    s.execute("layer.newSolid", json!({"color":"#406080"})).unwrap();
    s.execute("comp.new", json!({"name":"Outer","width":320,"height":180,"frameRate":30,"duration":10})).unwrap();
    let layer = s.execute("layer.addItem", json!({"item":"Source"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, layer)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

#[test]
fn extended_source_tint_starts_at_the_original_end_after_offset_and_stretch() {
    let (mut h, layer) = harness();
    let tail = format!("timeline.layer.{layer}.source.tail");
    assert!(h.state().auto.find(&tail).is_none(), "an ordinary strip needs no held-tail tint");
    h.state_mut().session.execute("layer.timeStretch", json!({"layers":[layer],"percent":200})).unwrap();
    h.state_mut().session.execute("layer.timing", json!({"layers":[layer],"delta":1,"out":7})).unwrap();
    h.run_steps(3);
    let bar = rect(&h, &format!("timeline.layer.{layer}.bar"));
    let held = rect(&h, &tail);
    assert!((held.min.x - (bar.min.x + bar.width() * 4.0 / 6.0)).abs() < 0.1, "source ends at comp time 5, not extended out 7: {held:?}");
    assert!((held.max.x - bar.max.x).abs() < 0.1);
    assert_eq!(held.height(), bar.height());
    if let Ok(dir) = std::env::var("TIMELINE_SOURCE_SNAPSHOTS") {
        h.render().unwrap().save(format!("{dir}/source-tail.png")).unwrap();
    }
}

#[test]
fn reversed_source_boundary_uses_the_original_span_and_follows_offsets() {
    let (mut h, layer) = harness();
    h.state_mut().session.execute("layer.timeStretch", json!({"layers":[layer],"percent":200})).unwrap();
    h.state_mut().session.execute("layer.timeReverse", json!({"layers":[layer]})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.layer.{layer}.source.tail")).is_none());
    h.state_mut().session.execute("layer.timing", json!({"layers":[layer],"delta":1,"out":7})).unwrap();
    h.run_steps(3);
    let bar = rect(&h, &format!("timeline.layer.{layer}.bar"));
    let tail = rect(&h, &format!("timeline.layer.{layer}.source.tail"));
    assert!((tail.min.x - (bar.min.x + bar.width() * 4.0 / 6.0)).abs() < 0.1);
    assert!(h.state().auto.find(&format!("timeline.layer.{layer}.source.head")).is_none());
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.layer.{layer}.source.tail")).is_none());
}

#[test]
fn locked_source_ghosts_preserve_the_trimmed_layers_source_offset() {
    for side in ["l", "r"] {
        let (mut h, layer) = harness();
        h.state_mut().session.execute("layer.timing", json!({"layers":[layer],"in":0.5,"out":1.5})).unwrap();
        h.state_mut().session.execute("layer.setSwitch", json!({"layers":[layer],"switch":"locked","value":true})).unwrap();
        let before = h.state().session.active_comp().unwrap().layer(effectcraft_engine::project::LayerId(layer)).unwrap().clone();
        h.run_steps(3);
        let from = rect(&h, &format!("timeline.layer.{layer}.source{side}")).center();
        let to = from + vec2(20.0, 0.0);
        h.event(Event::PointerMoved(from));
        h.step();
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerMoved(to));
        h.step();
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert!(
            h.state().session.active_comp().unwrap().layer(effectcraft_engine::project::LayerId(layer)).unwrap() == &before,
            "a locked source ghost {side} must preserve source timing"
        );
    }
}
