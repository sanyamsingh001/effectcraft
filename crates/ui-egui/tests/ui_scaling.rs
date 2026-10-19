//! Global interface scale and Settings geometry (#284).
use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::control::{self, ControlRequest, Outcome};
use egui::{Event, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width": 32, "height": 32, "duration": 2})).unwrap();
    build(session)
}

fn build(session: Session) -> Harness<'static, EffectcraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
    h.run_steps(3);
    h
}

fn send(h: &mut Harness<'_, EffectcraftApp>, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (request, _) = ControlRequest::new(method, params);
    let Outcome::Done(reply) = control::handle(h.state_mut(), &ctx, &request) else { panic!("immediate reply to {method}") };
    assert_eq!(reply["ok"], true, "{reply}");
    h.run_steps(3);
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let r = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}")).rect;
    Rect::from_min_size(pos2(r[0], r[1]), vec2(r[2], r[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

#[test]
fn saved_scale_applies_when_the_editor_reopens() {
    let store = std::sync::Arc::new(effectcraft_engine::config::MemoryConfig::default());
    let mut session = Session { config: Some(store.clone()), ..Default::default() };
    session.execute("prefs.set", json!({"key": "appearance.uiScale", "value": 1.5})).unwrap();
    let mut reopened = Session { config: Some(store), ..Default::default() };
    reopened.load_settings();
    let h = build(reopened);
    assert_eq!(h.ctx.zoom_factor(), 1.5);
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 1.5);
}

#[test]
fn scale_applies_over_control_and_settings_cancel_restores_it() {
    let mut h = harness();
    send(&mut h, "ui.menu.invoke", json!({"id": "app.settings", "params": {"page": "appearance"}}));
    assert!(h.state().auto.find("settings.appearance.uiScale").is_some());
    send(&mut h, "engine.execute", json!({"command": "prefs.set", "params": {"key": "appearance.uiScale", "value": 1.5}}));
    assert_eq!(h.ctx.zoom_factor(), 1.5);
    let viewport = rect(&h, "settings.viewport");
    assert!(h.ctx.content_rect().contains_rect(viewport), "scaled Settings viewport: {viewport:?}");
    let cancel = rect(&h, "settings.cancel").center();
    click(&mut h, cancel);
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 1.0);
    assert_eq!(h.ctx.zoom_factor(), 1.0);
}

#[test]
fn native_zoom_updates_the_control_and_survives_other_settings_changes() {
    let mut h = harness();
    h.ctx.set_zoom_factor(1.25);
    h.run_steps(3);
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 1.25);
    send(&mut h, "engine.execute", json!({"command": "prefs.set", "params": {"key": "appearance.brightness", "value": 0.1}}));
    assert_eq!(h.ctx.zoom_factor(), 1.25);
}

#[test]
fn scaled_popup_scrolls_its_entries_into_the_window() {
    let mut h = harness();
    send(&mut h, "engine.execute", json!({"command": "prefs.set", "params": {"key": "appearance.uiScale", "value": 2.0}}));
    assert_eq!(h.ctx.zoom_factor(), 2.0);
    let edit = rect(&h, "menu.Edit").center();
    click(&mut h, edit);
    let first = h.query_by_label_contains("Quick Apply").unwrap().rect().center();
    h.event(Event::PointerMoved(first));
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -2000.0), modifiers: Default::default(), phase: egui::TouchPhase::Move });
    h.run_steps(30);
    let last = h.query_by_label_contains("Keyboard Shortcuts").unwrap().rect();
    assert!(h.ctx.content_rect().contains_rect(last), "scaled menu scrolls its last entry into view: {last:?}");
    click(&mut h, last.center());
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::Shortcuts));
}

/// Fixed-width columns used to overflow the default dialog by a small amount.
#[test]
fn settings_columns_fit_the_default_dialog() {
    let mut h = harness();
    send(&mut h, "ui.menu.invoke", json!({"id": "app.settings", "params": {"page": "general"}}));
    let (content, viewport) = (rect(&h, "settings.content"), rect(&h, "settings.viewport"));
    assert!(content.width() <= viewport.width() + 0.5, "unnecessary horizontal scrolling: content {} > viewport {}", content.width(), viewport.width());
}

/// `cargo test -p effectcraft-ui-egui --test ui_scaling -- --ignored` saves visual checks.
#[test]
#[ignore]
fn scaled_settings_snapshot() {
    let mut h = harness();
    send(&mut h, "engine.execute", json!({"command": "prefs.set", "params": {"key": "appearance.uiScale", "value": 1.5}}));
    send(&mut h, "ui.menu.invoke", json!({"id": "app.settings", "params": {"page": "appearance"}}));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test-out");
    std::fs::create_dir_all(dir).unwrap();
    h.render().unwrap().save(format!("{dir}/scaled-settings.png")).unwrap();
}

fn native_zoom_during_unconfirmed_settings(via_escape: bool) {
    let store = std::sync::Arc::new(effectcraft_engine::config::MemoryConfig::default());
    let mut session = Session { config: Some(store.clone()), ..Default::default() };
    session.execute("prefs.set", json!({"key": "appearance.uiScale", "value": 1.0})).unwrap();
    session.execute("comp.new", json!({"width": 32, "height": 32, "duration": 2})).unwrap();
    let (original_scale, original_brightness) = (session.prefs.appearance.ui_scale, session.prefs.appearance.brightness);
    let mut h = build(session);
    send(&mut h, "ui.menu.invoke", json!({"id": "app.settings", "params": {"page": "appearance"}}));
    let brightness = rect(&h, "settings.appearance.brightness");
    click(&mut h, brightness.center() + vec2(brightness.width() * 0.25, 0.0));
    let edited_brightness = h.state().session.prefs.appearance.brightness;
    assert!((edited_brightness - original_brightness).abs() > 0.01, "the Settings slider changed its live brightness");
    let mut saved = Session { config: Some(store.clone()), ..Default::default() };
    saved.load_settings();
    assert_eq!(saved.prefs.appearance.brightness, original_brightness, "the live brightness edit has not been confirmed");
    h.ctx.set_zoom_factor(1.25);
    h.run_steps(3);
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 1.25);
    assert_eq!(h.state().session.prefs.appearance.brightness, edited_brightness);
    if via_escape {
        for pressed in [true, false] {
            h.event(Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed, repeat: false, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    } else {
        let cancel = rect(&h, "settings.cancel").center();
        click(&mut h, cancel);
    }
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.prefs.appearance.ui_scale, original_scale);
    assert_eq!(h.state().session.prefs.appearance.brightness, original_brightness);
    assert_eq!(h.ctx.zoom_factor(), original_scale as f32);
    let mut reopened = Session { config: Some(store), ..Default::default() };
    reopened.load_settings();
    assert_eq!(reopened.prefs.appearance.ui_scale, original_scale, "cancelling Settings keeps its original saved scale on restart");
    assert_eq!(reopened.prefs.appearance.brightness, original_brightness, "native zoom does not save an unconfirmed brightness edit");
}

#[test]
fn native_zoom_cancel_does_not_save_unconfirmed_settings() {
    native_zoom_during_unconfirmed_settings(false);
}

#[test]
fn native_zoom_escape_does_not_save_unconfirmed_settings() {
    native_zoom_during_unconfirmed_settings(true);
}
