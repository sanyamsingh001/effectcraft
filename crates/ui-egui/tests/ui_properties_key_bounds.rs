//! Native Properties key diamonds use the exact distance to imported keyframe times.
use effectcraft_engine::{
    Session,
    project::{LayerId, Project},
    time::{TICKS_PER_SECOND, Tick},
};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;
use std::sync::Arc;

type H = Harness<'static, EffectcraftApp>;

#[derive(Debug)]
struct Diamond {
    rect: [f32; 4],
    pixels: [[u8; 4]; 9],
}

fn rectangle(h: &H, id: &str) -> Rect {
    let entry = h.state().auto.find(id).unwrap_or_else(|| panic!("missing visible widget {id}"));
    let r = Rect::from_min_size(pos2(entry.rect[0], entry.rect[1]), vec2(entry.rect[2], entry.rect[3]));
    assert!(r.is_finite() && r.is_positive());
    let panel = h.state().auto.find("panel.Properties").unwrap().rect;
    let panel = Rect::from_min_size(pos2(panel[0], panel[1]), vec2(panel[2], panel[3]));
    assert!(panel.contains_rect(r), "the actual diamond is visible in Properties: {r:?}");
    r
}

fn click(h: &mut H, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

fn key_times(h: &H, layer: LayerId) -> Vec<Tick> {
    h.state().session.active_comp().unwrap().layer(layer).unwrap().props.prop("transform/opacity").unwrap().keys.iter().map(|key| key.time).collect()
}

fn sample(raw_time: i64, add_current_key: bool) -> Diamond {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"width":64,"height":64,"duration":2,"frameRate":30})).unwrap();
    let layer = LayerId(
        session.execute_checked("layer.newSolid", json!({"name":"Diamond","width":16,"height":8,"color":"#00ff00"})).unwrap()["layer"].as_u64().unwrap(),
    );
    // The ordinary public command constructs the same scalar/value track in every case.
    session.execute_checked("prop.addKey", json!({"layer":layer.0,"path":"transform/opacity","time":-1,"value":40})).unwrap();
    let cid = session.active_comp_id().unwrap();
    let mut imported = session.project.as_ref().clone();
    imported.comp_mut(cid).unwrap().layer_mut(layer).unwrap().props.prop_mut("transform/opacity").unwrap().keys[0].time = Tick(raw_time);
    // File-format import retains exact i64 ticks. Tick::MIN is i64::MIN/4 and is a
    // separate, safe value; this case intentionally imports the full raw minimum.
    session.project = Arc::new(Project::from_json(&imported.to_json()).unwrap());
    session.execute_checked("time.set", json!({"time":0})).unwrap();
    let selection = session.state.selected_layers.clone();
    assert_eq!(selection, [layer]);
    assert_eq!(session.time(), Tick::ZERO);
    let opacity = session.active_comp().unwrap().layer(layer).unwrap().props.prop("transform/opacity").unwrap();
    assert_eq!(opacity.keys.len(), 1);
    assert_eq!(opacity.keys[0].time, Tick(raw_time));
    let uid = opacity.uid;

    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| EffectcraftApp::new(session));
    h.state_mut().show_panel(PanelKind::Properties);
    h.state_mut().ui.maximized = Some(PanelKind::Properties);
    h.event(Event::PointerMoved(pos2(700.0, 100.0)));
    h.run_steps(4);
    assert_eq!(
        h.state_mut().session.execute_checked("prop.get", json!({"layer":layer.0,"path":"transform/opacity"})).unwrap()["value"],
        json!(40.0),
        "key-time changes preserve the displayed scalar value",
    );
    let r = rectangle(&h, &format!("properties.prop.{uid}.keyKey"));
    let image = h.render().expect("actual native UI pixels are required; this diagnostic must not skip");
    let x = r.center().x.round() as u32;
    let y = r.center().y.round() as u32;
    assert!(x > 0 && y > 0 && x + 1 < image.width() && y + 1 < image.height());
    let mut pixels = [[0; 4]; 9];
    for row in 0..3 {
        for col in 0..3 {
            pixels[row * 3 + col] = image.get_pixel(x + col as u32 - 1, y + row as u32 - 1).0;
        }
    }
    let observed = Diamond { rect: [r.min.x, r.min.y, r.width(), r.height()], pixels };

    if add_current_key {
        let before = Arc::clone(&h.state().session.project);
        let history = h.state().session.history.undo.len();
        click(&mut h, r.center());
        assert_eq!(h.state().session.state.selected_layers, selection);
        assert_eq!(h.state().session.time(), Tick::ZERO);
        assert_eq!(key_times(&h, layer), [Tick(raw_time), Tick::ZERO], "the real diamond adds a current-time key");
        assert_eq!(h.state().session.history.undo.len(), history + 1, "one diamond click is one Undo");
        h.state_mut().session.execute_checked("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.project, before, "Undo restores the exact imported project");
        assert_eq!(h.state().session.state.selected_layers, selection);
    }
    observed
}

#[test]
fn properties_imported_minimum_key_is_hollow_at_zero_and_click_adds_one_key() {
    let far = sample(-TICKS_PER_SECOND, true);
    let current = sample(0, false);
    assert_eq!(current.rect, far.rect, "both positive controls use the identical visible diamond layout");
    let center_distance: u32 = far.pixels[4][..3].iter().zip(&current.pixels[4][..3]).map(|(a, b)| u32::from(a.abs_diff(*b))).sum();
    assert!(center_distance > 20, "ordinary offscreen and CTI keys have distinct hollow/filled center colors: {far:?} / {current:?}");

    let imported_min = sample(i64::MIN, false);
    assert_eq!(imported_min.rect, far.rect, "the imported boundary does not change value/layout");
    println!("DIAMOND_NATIVE_CONTROL far={far:?} current={current:?} imported_min={imported_min:?}");
    assert_eq!(imported_min.pixels, far.pixels, "PROPERTIES_RAW_MIN_DIAMOND: an imported offscreen key must have the same hollow diamond as -1s at CTI0",);
    // The imported boundary click is a desired behavior, not a baseline control.
    // Verify it only after the narrow rendered-pixel oracle has passed.
    sample(i64::MIN, true);
}
