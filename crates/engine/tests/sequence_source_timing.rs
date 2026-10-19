//! Sequence source-panel bounds follow filenames rather than stale persisted duration.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemKind},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn sequence_source_seek_and_marks_use_the_authoritative_looped_frame_count() {
    let mut session = Session::default();
    let footage = Footage {
        kind: FootageKind::Sequence,
        frame_rate: FrameRate::FPS_24,
        duration: Tick::from_seconds_f64(99.0),
        loop_count: 2,
        sequence: vec!["frame1.png".into(), "frame2.png".into(), "frame3.png".into()],
        width: 4,
        height: 4,
        has_video: true,
        ..Default::default()
    };
    let item = Arc::make_mut(&mut session.project).add_item("Sequence", Default::default(), None, ItemKind::Footage(footage));
    let rate = FrameRate::FPS_24;
    assert_eq!(session.project.item(item).unwrap().source_span(), Some((rate.tick_of(6), rate.tick_of(5))));
    session.execute("footage.open", json!({"item":item.0})).unwrap();
    session.execute("footage.setTime", json!({"frame":999})).unwrap();
    let info = session.execute("footage.info", json!({})).unwrap();
    eprintln!("sequence source panel readout: {info}");
    assert_eq!(session.state.footage_panel.as_ref().unwrap().time, rate.tick_of(5), "source seek must clamp to the final logical frame5, matching source_span");
    assert_eq!(info["frame"], 5);
    assert_eq!(info["duration"], 0.25);
    session.execute("footage.setOut", json!({})).unwrap();
    assert_eq!(session.state.footage_panel.as_ref().unwrap().out_point, Some(rate.tick_of(6)));
    session.execute("footage.setTime", json!({"frame":2})).unwrap();
    assert_eq!(session.state.footage_panel.as_ref().unwrap().time, rate.tick_of(2));
    session.execute("footage.setIn", json!({})).unwrap();
    assert_eq!(session.state.footage_panel.as_ref().unwrap().in_point, Some(rate.tick_of(2)));
}
