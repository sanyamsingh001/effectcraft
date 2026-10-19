//! Middle-button panel navigation leaves editing and selection unchanged (#284).
use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::PanelKind;
use egui::{Event, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness(panel: PanelKind, height: f32) -> Harness<'static, EffectcraftApp> {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width": 16, "height": 16, "duration": 30})).unwrap();
    session.execute("layer.newSolid", json!({"name": "Plate", "width": 16, "height": 16})).unwrap();
    let mut app = EffectcraftApp::new(session);
    app.show_panel(panel);
    app.ui.maximized = Some(panel);
    let mut h = Harness::builder().with_size(vec2(1200.0, height)).build_eframe(|_| app);
    h.run_steps(3);
    h
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let r = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}")).rect;
    Rect::from_min_size(pos2(r[0], r[1]), vec2(r[2], r[3]))
}

fn middle_drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Middle, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Middle, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

fn offset(h: &Harness<'_, EffectcraftApp>, id: &str) -> f32 {
    h.state().auto.elements.iter().chain(h.state().auto.previous.iter()).find(|e| e.id == id).unwrap().label.split_once('/').unwrap().0.parse().unwrap()
}

#[test]
fn middle_drag_scrolls_effect_controls_without_editing_values() {
    let mut h = harness(PanelKind::EffectControls, 340.0);
    let lid = h.state().session.active_comp().unwrap().layers[0].id;
    for _ in 0..12 {
        h.state_mut().session.execute("effect.apply", json!({"effect": "ec.control.slider", "layers": [lid.0]})).unwrap();
    }
    h.run_steps(3);
    let selected = h.state().session.state.selected_layers.clone();
    let revision = h.state().session.revision;
    let value_uid = h.state().session.active_comp().unwrap().layer(lid).unwrap().props.prop("effects/#1/slider").unwrap().uid;
    let from = rect(&h, &format!("effectControls.prop.{value_uid}.value")).center();
    let initial = offset(&h, "effectControls.scroll");
    middle_drag(&mut h, from, from - vec2(20.0, 60.0));
    assert!(offset(&h, "effectControls.scroll") > initial, "the middle drag scrolls the rows");
    assert_eq!(h.state().session.revision, revision, "the value under the pointer was not scrubbed");
    assert_eq!(h.state().session.state.selected_layers, selected);
}

#[test]
fn middle_drag_scrolls_project_without_selecting_or_moving_items() {
    let mut h = harness(PanelKind::Project, 420.0);
    for i in 0..100 {
        h.state_mut().session.execute("project.newFolder", json!({"name": format!("Folder {i}")})).unwrap();
    }
    h.state_mut().session.execute("project.select", json!({"items": []})).unwrap();
    h.run_steps(3);
    h.state_mut().ui.project_scroll = 0.0;
    h.run_steps(2);
    let selected = h.state().session.state.project_selection.clone();
    let revision = h.state().session.revision;
    let from = rect(&h, "project.empty").center();
    middle_drag(&mut h, from, from - vec2(0.0, 60.0));
    assert!(h.state().ui.project_scroll > 0.0, "the middle drag scrolls Project rows");
    assert_eq!(h.state().session.revision, revision, "folders stay in place");
    assert_eq!(h.state().session.state.project_selection, selected);
}

#[test]
fn middle_drag_navigates_timeline_without_editing_layers() {
    let mut h = harness(PanelKind::Timeline, 420.0);
    for i in 0..40 {
        h.state_mut().session.execute("layer.newSolid", json!({"name": format!("Layer {i}"), "width": 16, "height": 16})).unwrap();
    }
    h.state_mut().ui.timeline.pps = Some(200.0);
    h.state_mut().ui.timeline.start = 5.0;
    h.run_steps(3);
    let selected = h.state().session.state.selected_layers.clone();
    let revision = h.state().session.revision;
    let time = h.state().session.time();
    let row = rect(&h, &format!("timeline.layer.{}.row", selected[0].0));
    let from = pos2(rect(&h, "timeline.ruler").center().x, row.center().y);
    middle_drag(&mut h, from, from - vec2(80.0, 60.0));
    assert!(h.state().ui.timeline.start > 5.0, "the time graph pans horizontally");
    assert!(h.state().ui.timeline.scroll_y > 0.0, "the layer rows scroll vertically");
    assert_eq!(h.state().session.revision, revision, "layer timing and order stay unchanged");
    assert_eq!(h.state().session.time(), time);
    assert_eq!(h.state().session.state.selected_layers, selected);
}

#[test]
fn middle_drag_scrolls_settings_pages_without_changing_the_page() {
    let mut h = harness(PanelKind::Project, 520.0);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "app.settings", json!({"page": "general"})).unwrap();
    h.run_steps(3);
    let from = rect(&h, "settings.page.general").center();
    let last_id = format!("settings.page.{}", effectcraft_engine::prefs::pages().last().unwrap().id);
    let before = rect(&h, &last_id).min.y;
    middle_drag(&mut h, from, from - vec2(0.0, 120.0));
    assert!(rect(&h, &last_id).min.y < before, "the middle drag scrolls the page list");
    assert!(h.state().auto.find("settings.general.undoLevels").is_some(), "the General page stays open");
}

fn button(h: &mut Harness<'_, EffectcraftApp>, at: Pos2, button: egui::PointerButton, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() });
    h.step();
}

fn move_to(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}

fn primary_drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    move_to(h, from);
    button(h, from, egui::PointerButton::Primary, true);
    for i in 1..=8 {
        move_to(h, from + (to - from) * (i as f32 / 8.0));
    }
    button(h, to, egui::PointerButton::Primary, false);
    h.run_steps(3);
}

fn effect_controls() -> Harness<'static, EffectcraftApp> {
    let mut h = harness(PanelKind::EffectControls, 340.0);
    let lid = h.state().session.active_comp().unwrap().layers[0].id.0;
    for _ in 0..12 {
        h.state_mut().session.execute("effect.apply", json!({"effect": "ec.control.slider", "layers": [lid]})).unwrap();
    }
    h.run_steps(3);
    h
}

fn visible_slider(h: &Harness<'_, EffectcraftApp>) -> (u64, Pos2) {
    let bar = rect(h, "effectControls.scroll");
    let e = h
        .state()
        .auto
        .query("effectControls.prop.")
        .into_iter()
        .find(|e| e.id.ends_with(".value") && e.rect[1] > bar.min.y && e.rect[1] + e.rect[3] < bar.max.y)
        .unwrap();
    let uid = e.id.strip_prefix("effectControls.prop.").unwrap().strip_suffix(".value").unwrap().parse().unwrap();
    (uid, pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0))
}

fn slider(h: &Harness<'_, EffectcraftApp>, uid: u64) -> f64 {
    h.state().session.active_comp().unwrap().layers[0].props.find(uid).unwrap().value.as_f64()
}

#[test]
fn overlapping_buttons_preserve_the_first_gesture_and_release_cleanly() {
    use egui::PointerButton::{Middle, Primary};
    for primary_first in [false, true] {
        for first_released_first in [false, true] {
            let mut h = effect_controls();
            let (uid, from) = visible_slider(&h);
            let revision = h.state().session.revision;
            let (first, second) = if primary_first { (Primary, Middle) } else { (Middle, Primary) };
            move_to(&mut h, from);
            button(&mut h, from, first, true);
            let delta = if primary_first { vec2(20.0, 0.0) } else { vec2(0.0, -20.0) };
            let mut at = from + delta;
            move_to(&mut h, at);
            button(&mut h, at, second, true);
            at += delta;
            move_to(&mut h, at);
            if primary_first {
                assert!(slider(&h, uid) > 0.0, "a primary scrub keeps ownership when Middle is pressed");
                assert_eq!(offset(&h, "effectControls.scroll"), 0.0);
            } else {
                assert!(offset(&h, "effectControls.scroll") >= 39.0, "middle navigation keeps ownership when Primary is pressed");
                assert_eq!(h.state().session.revision, revision);
            }
            let released = if first_released_first { first } else { second };
            let held = if first_released_first { second } else { first };
            button(&mut h, at, released, false);
            let (value_before, scroll_before) = (slider(&h, uid), offset(&h, "effectControls.scroll"));
            at += delta;
            move_to(&mut h, at);
            if first_released_first {
                assert_eq!(slider(&h, uid), value_before, "releasing the original gesture does not start a value edit with the already-held button");
                assert_eq!(offset(&h, "effectControls.scroll"), scroll_before);
            } else if primary_first {
                assert!(slider(&h, uid) > value_before, "the remaining Primary gesture still scrubs");
            } else {
                assert!(offset(&h, "effectControls.scroll") > scroll_before, "the remaining Middle gesture still navigates");
            }
            button(&mut h, at, held, false);
            h.run_steps(3);
            let (next_uid, next_at) = visible_slider(&h);
            let before = slider(&h, next_uid);
            primary_drag(&mut h, next_at, next_at + vec2(30.0, 0.0));
            assert!(slider(&h, next_uid) > before, "the next independent primary edit works after either release order");
        }
    }
}

#[test]
fn middle_drag_can_leave_reenter_and_release_outside_project() {
    let mut h = harness(PanelKind::Project, 420.0);
    for i in 0..100 {
        h.state_mut().session.execute("project.newFolder", json!({"name": format!("Folder {i}")})).unwrap();
    }
    h.state_mut().session.execute("project.select", json!({"items": []})).unwrap();
    h.run_steps(3);
    h.state_mut().ui.project_scroll = 0.0;
    h.run_steps(2);
    let revision = h.state().session.revision;
    let from = rect(&h, "project.empty").center();
    move_to(&mut h, from);
    button(&mut h, from, egui::PointerButton::Middle, true);
    move_to(&mut h, from - vec2(0.0, 20.0));
    move_to(&mut h, pos2(-40.0, from.y - 40.0));
    move_to(&mut h, from - vec2(0.0, 60.0));
    let outside = pos2(-40.0, from.y - 80.0);
    move_to(&mut h, outside);
    button(&mut h, outside, egui::PointerButton::Middle, false);
    h.run_steps(3);
    assert!(h.state().ui.project_scroll >= 79.0, "navigation continues outside and after re-entry");
    assert_eq!(h.state().session.revision, revision);
    assert!(h.state().session.state.project_selection.is_empty());
    let viewport = rect(&h, "project.empty");
    let e = h
        .state()
        .auto
        .query("project.item.")
        .into_iter()
        .find(|e| e.id.ends_with(".name") && viewport.contains(pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)))
        .unwrap()
        .clone();
    let item: u64 = e.id.strip_prefix("project.item.").unwrap().strip_suffix(".name").unwrap().parse().unwrap();
    let at = pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    move_to(&mut h, at);
    button(&mut h, at, egui::PointerButton::Primary, true);
    button(&mut h, at, egui::PointerButton::Primary, false);
    h.run_steps(3);
    assert_eq!(h.state().session.state.project_selection, vec![effectcraft_engine::project::ItemId(item)], "a primary selection works after release outside");
}

#[test]
fn middle_drag_pans_graph_values_and_time_without_editing_keys() {
    let mut h = harness(PanelKind::Timeline, 600.0);
    let lid = h.state().session.active_comp().unwrap().layers[0].id;
    for (time, value) in [(6.0, 20.0), (7.0, 80.0)] {
        h.state_mut().session.execute("prop.addKey", json!({"layer": lid.0, "path": "transform/opacity", "time": time, "value": value})).unwrap();
    }
    let uid = h.state().session.active_comp().unwrap().layer(lid).unwrap().props.prop("transform/opacity").unwrap().uid;
    h.state_mut().session.execute("prop.select", json!({"layer": lid.0, "prop": uid})).unwrap();
    h.state_mut().session.execute("keys.selectAll", json!({})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.state_mut().ui.timeline.graph_mode = "value".into();
    h.state_mut().ui.timeline.pps = Some(100.0);
    h.state_mut().ui.timeline.start = 5.0;
    h.run_steps(3);
    let revision = h.state().session.revision;
    let selected = h.state().session.state.selected_keys.clone();
    let (lo, hi) = h.state().ui.timeline.graph_range.unwrap();
    let key_id = format!("timeline.graph.key.{uid}.0.0");
    let from = rect(&h, &key_id).center();
    middle_drag(&mut h, from, from - vec2(40.0, 40.0));
    let (panned_lo, panned_hi) = h.state().ui.timeline.graph_range.unwrap();
    assert!(panned_lo < lo && panned_hi < hi, "a middle drag pans the value axis");
    assert!(((panned_hi - panned_lo) - (hi - lo)).abs() < 1e-6, "panning keeps the value scale");
    assert!(!h.state().ui.timeline.graph_auto_zoom, "manual navigation holds its range");
    assert!(h.state().ui.timeline.start > 5.0);
    assert_eq!(h.state().ui.timeline.scroll_y, 0.0, "graph navigation leaves outline scrolling alone");
    assert_eq!(h.state().session.revision, revision, "the key under the pointer stays unchanged");
    assert_eq!(h.state().session.state.selected_keys, selected);
    let at = rect(&h, &key_id).center();
    primary_drag(&mut h, at, at + vec2(20.0, -10.0));
    assert!(h.state().session.revision > revision, "a primary key edit works after navigation");
}

/// Releasing the other button must not end navigation or hand its pointer to a value control.
#[test]
fn releasing_primary_keeps_middle_navigation_until_middle_release() {
    use egui::PointerButton::{Middle, Primary};
    let mut h = effect_controls();
    let (uid, from) = visible_slider(&h);
    let (value, revision) = (slider(&h, uid), h.state().session.revision);
    move_to(&mut h, from);
    button(&mut h, from, Middle, true);
    let mut at = from - vec2(0.0, 20.0);
    move_to(&mut h, at);
    assert!(offset(&h, "effectControls.scroll") >= 19.0);
    button(&mut h, at, Primary, true);
    at -= vec2(0.0, 20.0);
    move_to(&mut h, at);
    assert!(offset(&h, "effectControls.scroll") >= 39.0);
    button(&mut h, at, Primary, false);
    let before = offset(&h, "effectControls.scroll");
    at -= vec2(0.0, 20.0);
    move_to(&mut h, at);
    assert!(offset(&h, "effectControls.scroll") >= before + 19.0, "navigation continues after Primary is released while Middle stays down");
    assert_eq!(slider(&h, uid), value, "navigation leaves the numeric value unchanged");
    assert_eq!(h.state().session.revision, revision, "no numeric edit was committed during either button transition");
    button(&mut h, at, Middle, false);
    h.run_steps(3);
    assert_eq!(h.state().session.revision, revision);
    let (next_uid, next_at) = visible_slider(&h);
    let before = slider(&h, next_uid);
    primary_drag(&mut h, next_at, next_at + vec2(30.0, 0.0));
    assert!(slider(&h, next_uid) > before, "the next independent Primary edit still works");
}
