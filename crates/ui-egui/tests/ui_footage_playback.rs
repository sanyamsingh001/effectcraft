//! Footage playback retains subframe time on a 60 Hz display and visits its last frame.

use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind},
    time::{FrameRate, Tick},
};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

fn harness() -> (Harness<'static, EffectcraftApp>, ItemId) {
    let mut session = Session::default();
    let footage = Footage { kind: FootageKind::Sequence, frame_rate: FrameRate::FPS_24, duration: FrameRate::FPS_24.tick_of(3), ..Default::default() };
    let project = std::sync::Arc::make_mut(&mut session.project);
    let first = project.add_item("First", effectcraft_engine::color::Label::Lavender, None, ItemKind::Footage(footage.clone()));
    let second = project.add_item("Second", effectcraft_engine::color::Label::Lavender, None, ItemKind::Footage(footage));
    session.execute("footage.open", json!({"item": first.0})).unwrap();
    let mut app = EffectcraftApp::new(session);
    app.show_panel(PanelKind::Footage);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1000.0)).with_step_dt(1.0 / 60.0).build_eframe(|_| app);
    h.run_steps(4);
    (h, second)
}

fn click_play(h: &mut Harness<'_, EffectcraftApp>) {
    let e = h.state().auto.find("footage.play").unwrap();
    let at = egui::pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
}

fn frame(h: &Harness<'_, EffectcraftApp>) -> i64 {
    FrameRate::FPS_24.frame_at(h.state().session.state.footage_panel.as_ref().unwrap().time)
}

#[test]
fn footage_play_advances_at_media_rate_and_displays_the_final_frame() {
    let (mut h, _) = harness();
    click_play(&mut h);
    h.run_steps(2);
    assert_eq!(frame(&h), 0);
    h.step();
    assert_eq!(frame(&h), 1, "three 60 Hz updates must accumulate past the first 24 fps boundary");
    h.run_steps(3);
    assert_eq!(frame(&h), 2, "the final frame must remain visible before the sequence loops");
    h.run_steps(2);
    assert_eq!(frame(&h), 0, "loop only after the exclusive source end");
}

#[test]
fn playback_clock_resets_after_seek_source_switch_and_pause() {
    let (mut h, second) = harness();
    click_play(&mut h);
    h.run_steps(2);
    h.state_mut().session.execute("footage.setTime", json!({"frame": 1})).unwrap();
    h.step();
    assert_eq!(frame(&h), 1);
    h.run_steps(2);
    assert_eq!(frame(&h), 1, "manual seek starts a fresh phase at the selected frame");
    h.step();
    assert_eq!(frame(&h), 2);
    h.state_mut().session.execute("footage.open", json!({"item": second.0})).unwrap();
    h.step();
    assert_eq!(frame(&h), 0, "opening another source resets playback phase");
    h.run_steps(2);
    assert_eq!(frame(&h), 0);
    h.step();
    assert_eq!(frame(&h), 1);
    click_play(&mut h);
    let paused = frame(&h);
    h.run_steps(6);
    assert_eq!(frame(&h), paused, "Pause holds its selected source frame");
    click_play(&mut h);
    h.run_steps(2);
    assert_eq!(frame(&h), paused, "Resume starts a fresh phase rather than catching up paused time");
    h.step();
    assert_eq!(frame(&h), (paused + 1) % 3);
}

fn assert_sequence_playback_span(duration: Tick, label: &str) {
    let (mut h, _) = harness();
    let source = h.state().session.state.footage_panel.as_ref().unwrap().item;
    let ItemKind::Footage(footage) = &mut std::sync::Arc::make_mut(&mut h.state_mut().session.project).item_mut(source).unwrap().kind else {
        panic!("footage")
    };
    footage.sequence = vec!["frame1.png".into(), "frame2.png".into(), "frame3.png".into()];
    footage.duration = duration;
    footage.loop_count = 2;
    h.run_steps(2);
    click_play(&mut h);
    h.run_steps(3);
    assert_eq!(frame(&h), 1, "{label}: first frame boundary retains its accumulated phase");
    h.run_steps(5);
    assert_eq!(frame(&h), 3, "{label}: playback enters the second loop of the source");
    h.run_steps(5);
    assert_eq!(frame(&h), 5, "{label}: the final logical frame must remain visible");
    h.run_steps(3);
    assert_eq!(frame(&h), 0, "{label}: wrap at the six-frame exclusive end, without holding for stale metadata");
}

#[test]
fn sequence_playback_ruler_clock_uses_the_looped_file_count() {
    assert_sequence_playback_span(FrameRate::FPS_24.tick_of(3), "fresh");
}

#[test]
fn sequence_playback_ruler_clock_ignores_stale_duration() {
    assert_sequence_playback_span(Tick::from_seconds_f64(99.0), "stale");
}
