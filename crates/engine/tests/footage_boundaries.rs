//! Source seek and Out use integer rational frame boundaries through the public API.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

fn source(kind: FootageKind, rate: FrameRate, frames: i64, loops: u32) -> (Session, ItemId) {
    let mut session = Session::default();
    let footage = Footage {
        kind,
        frame_rate: rate,
        duration: rate.tick_of(frames),
        loop_count: loops,
        sequence: if kind == FootageKind::Sequence { (0..frames).map(|n| format!("frame{n:04}.png")).collect() } else { vec![] },
        ..Default::default()
    };
    let item = Arc::make_mut(&mut session.project).add_item("Source", Default::default(), None, ItemKind::Footage(footage));
    session.execute("footage.open", json!({"item":item.0})).unwrap();
    (session, item)
}

#[test]
fn fractional_sequence_overshoot_seek_clamps_to_the_canonical_boundary() {
    let rate = FrameRate::from_f64(12.345);
    let (mut session, item) = source(FootageKind::Sequence, rate, 4, 1);
    assert_eq!(session.project.item(item).unwrap().source_span(), Some((rate.tick_of(4), rate.tick_of(3))));
    for frame in [3, 999] {
        session.execute("footage.setTime", json!({"frame":frame})).unwrap();
        let info = session.execute("footage.info", json!({})).unwrap();
        eprintln!("seek {frame}: {info}; tick={:?}, expected={:?}", session.state.footage_panel.as_ref().unwrap().time, rate.tick_of(3));
        assert_eq!(
            session.state.footage_panel.as_ref().unwrap().time,
            rate.tick_of(3),
            "overshoot seek must use the same canonical last-frame tick as an in-range frame seek"
        );
        assert_eq!(info["frame"], 3);
    }
}

#[test]
fn fractional_sequence_out_ends_on_the_exact_next_boundary() {
    let rate = FrameRate::from_f64(12.345);
    let (mut session, _) = source(FootageKind::Sequence, rate, 4, 1);
    session.execute("footage.setTime", json!({"frame":1})).unwrap();
    assert_eq!(session.state.footage_panel.as_ref().unwrap().time, rate.tick_of(1));
    session.execute("footage.setOut", json!({})).unwrap();
    eprintln!("Out={:?}, expected={:?}", session.state.footage_panel.as_ref().unwrap().out_point, rate.tick_of(2));
    assert_eq!(session.state.footage_panel.as_ref().unwrap().out_point, Some(rate.tick_of(2)), "Out must end at the exact next frame boundary");
}

#[test]
fn rational_source_seek_keeps_in_range_and_broadcast_video_behavior() {
    let rate = FrameRate::from_f64(12.345);
    let (mut session, _) = source(FootageKind::Sequence, rate, 4, 1);
    for frame in [0, 1, 2] {
        session.execute("footage.setTime", json!({"frame":frame})).unwrap();
        assert_eq!(session.state.footage_panel.as_ref().unwrap().time, rate.tick_of(frame));
    }
    let (mut video, _) = source(FootageKind::Video, FrameRate::FPS_30, 60, 2);
    video.execute("footage.setTime", json!({"frame":999})).unwrap();
    assert_eq!(video.state.footage_panel.as_ref().unwrap().time, FrameRate::FPS_30.tick_of(59));
    assert_eq!(video.execute("footage.info", json!({})).unwrap()["duration"], 2.0, "Video source-panel duration keeps its native non-looped semantics");
    video.execute("footage.setOut", json!({})).unwrap();
    assert_eq!(video.state.footage_panel.as_ref().unwrap().out_point, Some(Tick::from_seconds_f64(2.0)));
}

#[test]
fn partial_video_native_panel_behavior_stays_unchanged() {
    use effectcraft_engine::{
        Session,
        project::{Footage, FootageKind, ItemKind},
        time::{FrameRate, Tick},
    };
    let mut s = Session::default();
    let duration = Tick::from_seconds_f64(0.04);
    let rate = FrameRate::FPS_30;
    let item = std::sync::Arc::make_mut(&mut s.project).add_item(
        "Partial Video",
        Default::default(),
        None,
        ItemKind::Footage(Footage { kind: FootageKind::Video, frame_rate: rate, duration, ..Default::default() }),
    );
    assert_eq!(s.project.item(item).unwrap().source_span().unwrap().1, Tick::ZERO);
    s.execute("footage.open", serde_json::json!({"item":item.0})).unwrap();
    s.execute("footage.setTime", serde_json::json!({"frame":999})).unwrap();
    assert_eq!(s.execute("footage.info", serde_json::json!({})).unwrap()["frame"], 0);
    assert_eq!(s.state.footage_panel.as_ref().unwrap().time, duration - rate.frame_duration());
    s.execute("footage.setOut", serde_json::json!({})).unwrap();
    assert_eq!(s.state.footage_panel.as_ref().unwrap().out_point, Some(duration));
}
