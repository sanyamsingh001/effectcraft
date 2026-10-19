//! Pointer regressions for the Ease Presets influence slider and its existing Apply command.

use effectcraft_engine::Session;
use effectcraft_engine::keyframe::{EaseCurve, Interp};
use effectcraft_engine::project::LayerId;
use effectcraft_engine::time::Tick;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::PanelKind;
use egui::{Event, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;

fn harness() -> (Harness<'static, EffectcraftApp>, LayerId) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Ease", "width": 64, "height": 64, "frameRate": 30, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Box", "color": "#e04020", "width": 32, "height": 32})).unwrap();
    let id = s.active_comp().unwrap().layers[0].id;
    for (path, keys) in [("transform/opacity", vec![(0.0, 0.0), (1.0, 100.0), (3.0, 20.0)]), ("transform/rotation", vec![(0.0, 0.0), (2.0, 90.0)])] {
        for (time, value) in keys {
            s.execute("prop.addKey", json!({"layer": id.0, "path": path, "time": time, "value": value})).unwrap();
        }
    }
    s.execute("prop.select", json!({"layer": id.0, "path": "transform/opacity"})).unwrap();
    s.execute("prop.select", json!({"layer": id.0, "path": "transform/rotation", "add": true})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.state_mut().show_panel(PanelKind::EasePresets);
    h.state_mut().ui.maximized = Some(PanelKind::EasePresets);
    h.run_steps(4);
    (h, id)
}

fn at(h: &Harness<'_, EffectcraftApp>, id: &str, fraction: f32) -> Pos2 {
    let r = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}")).rect;
    pos2(r[0] + r[2] * fraction, r[1] + r[3] * 0.5)
}

fn click(h: &mut Harness<'_, EffectcraftApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    h.run_steps(2);
}

#[test]
fn influence_slider_applies_every_selected_pair_in_one_undo_step() {
    let (mut h, id) = harness();
    assert_eq!(effectcraft_engine::ease_presets::selected_pairs(&h.state().session).len(), 3);
    let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
    let opacity = layer.props.prop("transform/opacity").unwrap().keys.clone();
    let rotation = layer.props.prop("transform/rotation").unwrap().keys.clone();
    let steps = h.state().session.history.undo.len();
    // Starting from a custom curve proves the slider creates a symmetric working curve.
    h.state_mut().ui.ease_presets.curve = EaseCurve { out_influence: 80.0, out_speed: 2.0, in_influence: 20.0, in_speed: -1.0 };
    for (amount, quarter) in [(0.0, 0.25), (50.0, 0.10589254302501768), (100.0, 0.0297246055119252)] {
        h.run_steps(2);
        let p = at(&h, "easePresets.influence", amount as f32 / 100.0);
        click(&mut h, p);
        let curve = h.state().ui.ease_presets.curve;
        if amount == 0.0 {
            assert_eq!(curve, EaseCurve::LINEAR, "zero restores the exact Linear curve");
        } else {
            assert!((curve.out_influence - amount).abs() < 1e-9 && (curve.in_influence - amount).abs() < 1e-9, "{curve:?}");
            assert_eq!((curve.out_speed, curve.in_speed), (0.0, 0.0));
        }
        assert_eq!(h.state().session.history.undo.len(), steps, "adjusting the working curve does not edit keys");
        let p = at(&h, "easePresets.apply", 0.5);
        click(&mut h, p);
        assert_eq!(h.state().session.history.undo.len(), steps + 1, "one undo applies all three selected pairs");
        let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
        let op = layer.props.prop("transform/opacity").unwrap();
        let rot = layer.props.prop("transform/rotation").unwrap();
        assert!((op.value_at(Tick::from_seconds_f64(0.25)).as_f64() - 100.0 * quarter).abs() < 1e-7, "first opacity segment");
        assert!((op.value_at(Tick::from_seconds_f64(1.5)).as_f64() - (100.0 - 80.0 * quarter)).abs() < 1e-7, "descending opacity segment");
        assert!((rot.value_at(Tick::from_seconds_f64(0.5)).as_f64() - 90.0 * quarter).abs() < 1e-7, "rotation segment");
        assert_eq!((op.keys[0].in_interp, op.keys[2].out_interp), (Interp::Linear, Interp::Linear), "unpaired sides stay unchanged");
        h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        let layer = h.state().session.active_comp().unwrap().layer(id).unwrap();
        assert_eq!(layer.props.prop("transform/opacity").unwrap().keys, opacity);
        assert_eq!(layer.props.prop("transform/rotation").unwrap().keys, rotation);
        assert_eq!(h.state().session.history.undo.len(), steps);
    }
}

#[test]
fn dragging_influence_beyond_the_rail_clamps_at_zero_and_one_hundred() {
    let (mut h, _) = harness();
    let steps = h.state().session.history.undo.len();
    let r = h.state().auto.find("easePresets.influence").unwrap().rect;
    let left = pos2(r[0], r[1] + r[3] * 0.5);
    for (from, to, expected) in [(left, left + egui::vec2(r[2] + 24.0, 0.0), 100.0), (left + egui::vec2(r[2], 0.0), left - egui::vec2(24.0, 0.0), 0.0)] {
        h.input_mut().events.push(Event::PointerMoved(from));
        h.step();
        h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for i in 1..=12 {
            h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 12.0)));
            h.step();
        }
        h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
        h.run_steps(2);
        let c = h.state().ui.ease_presets.curve;
        if expected == 0.0 {
            assert_eq!(c, EaseCurve::LINEAR);
        } else {
            assert_eq!((c.out_influence, c.in_influence, c.out_speed, c.in_speed), (expected, expected, 0.0, 0.0));
        }
    }
    assert_eq!(h.state().session.history.undo.len(), steps, "slider drags only change the working curve");
}

#[test]
fn slider_edits_preserve_the_selected_user_preset_like_graph_handle_edits() {
    let (mut h, _) = harness();
    let curve = EaseCurve { out_influence: 80.0, out_speed: 0.0, in_influence: 20.0, in_speed: 2.0 };
    h.state_mut().session.execute("keys.easePreset.save", json!({"name": "Asymmetric", "curve": curve})).unwrap();
    h.run_steps(2);
    let tile = h.state().auto.query("easePresets.preset.").into_iter().find(|e| e.label == "Asymmetric").expect("saved preset").id.clone();
    let p = at(&h, &tile, 0.5);
    click(&mut h, p);
    h.run_steps(2);
    assert_eq!(h.state().ui.ease_presets.curve, curve, "showing the slider keeps the custom curve");
    assert_eq!(h.state().ui.ease_presets.selected.as_deref(), Some("Asymmetric"));
    let steps = h.state().session.history.undo.len();
    let p = at(&h, "easePresets.influence", 0.5);
    click(&mut h, p);
    assert_eq!(h.state().ui.ease_presets.selected.as_deref(), Some("Asymmetric"), "Rename/Delete still act on the last clicked user preset");
    assert_eq!(h.state().session.history.undo.len(), steps);
    let p = at(&h, "easePresets.delete", 0.5);
    click(&mut h, p);
    assert!(h.state().session.ease_presets.iter().all(|p| p.name != "Asymmetric"), "Delete remains available after editing the working curve");
}
