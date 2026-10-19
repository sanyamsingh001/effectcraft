//! Project Settings color-engine choices stay valid when switched through the dialog.

use effectcraft_engine::Session;
use effectcraft_engine::project::{BitDepth, ColorEngine, ColorSpace, HdrMode};
use effectcraft_ui_egui::{Dialog, EffectcraftApp};
use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

fn harness(mut session: Session) -> Harness<'static, EffectcraftApp> {
    session.prefs.appearance.ui_scale = 1.0;
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
    h.run_steps(3);
    h
}

fn center(h: &Harness<'_, EffectcraftApp>, id: &str) -> Pos2 {
    let element = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    pos2(element.rect[0] + element.rect[2] / 2.0, element.rect[1] + element.rect[3] / 2.0)
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn open(h: &mut Harness<'_, EffectcraftApp>) {
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.projectSettings", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::Form));
}

fn choose(h: &mut Harness<'_, EffectcraftApp>, key: &str, label: &str) {
    let at = center(h, &format!("form.field.{key}"));
    click(h, at);
    let at = h.query_by_label(label).unwrap_or_else(|| panic!("missing option {label}")).rect().center();
    click(h, at);
    assert!(h.query_by_value(label).is_some(), "{key} selects {label}");
}

fn modal_rect(h: &Harness<'_, EffectcraftApp>) -> egui::Rect {
    h.ctx.memory(|m| m.area_rect(egui::Id::new(("modal", "Project Settings")))).expect("Project Settings modal")
}

fn submit(h: &mut Harness<'_, EffectcraftApp>) {
    let button = h.get_by_label("   OK   ").rect();
    let modal = modal_rect(h);
    assert!(modal.shrink(18.0).contains_rect(button), "OK is visible without scrolling: button={button:?}, modal={modal:?}");
    click(h, button.center());
    assert!(h.state().dialog.is_none(), "OK submits: button={button:?}, modal={modal:?}, status={}", h.state().ui.status);
}

#[test]
fn project_settings_fields_and_ok_are_visible_without_horizontal_scrolling() {
    let mut h = harness(Session::default());
    open(&mut h);
    let modal = modal_rect(&h);
    let visible = modal.shrink(18.0);
    for id in ["form.field.bitDepth", "form.field.colorEngine", "form.field.workingSpace", "form.field.hdr", "form.field.renderer", "form.ok"] {
        let e = h.state().auto.find(id).unwrap();
        let r = egui::Rect::from_min_size(pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3]));
        assert!(visible.contains_rect(r), "{id}: content={r:?}, modal={modal:?}, visible={visible:?}");
    }
}

#[test]
fn ocio_engine_switch_updates_visible_defaults_and_submits_in_one_undo_step() {
    let mut h = harness(Session::default());
    let history = h.state().session.history.undo.len();
    open(&mut h);
    choose(&mut h, "colorEngine", "OCIO (built-in ACES config)");
    assert!(h.query_by_value("ACEScg").is_some(), "the working-space choice reflects the OCIO default before OK");
    assert!(h.query_by_value("Tone map").is_some(), "the HDR choice reflects the OCIO default before OK");
    submit(&mut h);
    let settings = &h.state().session.project.settings;
    assert_eq!((settings.color_engine, settings.working_space, settings.hdr), (ColorEngine::Ocio, Some(ColorSpace::AcesCg), HdrMode::ToneMap));
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state().dialog.is_none());
    open(&mut h);
    assert!(h.query_by_value("ACEScg").is_some());
    assert!(h.query_by_value("Tone map").is_some());
    // Explicit choices remain untouched on unrelated edits while OCIO is already selected.
    choose(&mut h, "workingSpace", "ACES2065-1");
    choose(&mut h, "hdr", "Clip");
    choose(&mut h, "bitDepth", "32 bits per channel (float)");
    assert!(h.query_by_value("ACES2065-1").is_some());
    assert!(h.query_by_value("Clip").is_some());
    submit(&mut h);
    let settings = &h.state().session.project.settings;
    assert_eq!((settings.working_space, settings.hdr, settings.bit_depth), (Some(ColorSpace::Aces2065), HdrMode::Clip, BitDepth::Bpc32));
    assert_eq!(h.state().session.history.undo.len(), history + 2);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project.settings.working_space, Some(ColorSpace::AcesCg));
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project.settings.color_engine, ColorEngine::Adobe);
    assert_eq!(h.state().session.project.settings.working_space, None);
}

#[test]
fn ocio_engine_switch_preserves_existing_aces_and_hdr_choices() {
    let mut session = Session::default();
    session.execute("file.projectSettings", json!({"workingSpace": "aces2065", "hdr": "compand"})).unwrap();
    let mut h = harness(session);
    let history = h.state().session.history.undo.len();
    open(&mut h);
    choose(&mut h, "colorEngine", "OCIO (built-in ACES config)");
    assert!(h.query_by_value("ACES2065-1").is_some());
    assert!(h.query_by_value("Compand").is_some());
    submit(&mut h);
    let settings = &h.state().session.project.settings;
    assert_eq!(
        (settings.color_engine, settings.working_space, settings.hdr),
        (ColorEngine::Ocio, Some(ColorSpace::Aces2065), HdrMode::Compand),
        "status={}",
        h.state().ui.status
    );
    assert_eq!(h.state().session.history.undo.len(), history + 1);
}
