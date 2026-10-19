//! Loop duration arithmetic must stay bounded before slip offsets are clipped.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemKind, LayerId},
    time::{FrameRate, Tick},
};
use serde_json::json;

fn setup(duration: f64, loops: u32, comp_duration: f64) -> (Session, LayerId) {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width": 4, "height": 4, "frameRate": 24, "duration": comp_duration})).unwrap();
    let footage = Footage {
        kind: FootageKind::Video,
        width: 4,
        height: 4,
        frame_rate: FrameRate::FPS_24,
        duration: Tick::from_seconds_f64(duration),
        loop_count: loops,
        has_video: true,
        ..Default::default()
    };
    let item = std::sync::Arc::make_mut(&mut session.project).add_item("Looped source", Default::default(), None, ItemKind::Footage(footage));
    let layer = LayerId(session.execute("layer.addItem", json!({"item": item.0})).unwrap()["layer"].as_u64().unwrap());
    (session, layer)
}

#[test]
fn long_looped_footage_slips_without_duration_overflow_or_decoding() {
    let (mut session, layer) = setup(7200.0, 9999, 10.0);
    session.execute("layer.timing", json!({"layers": [layer.0], "in": 2, "out": 8})).unwrap();
    let before = session.active_comp().unwrap().layer(layer).unwrap().clone();
    session.execute("layer.slip", json!({"layers": [layer.0], "delta": 10})).unwrap();
    let after = session.active_comp().unwrap().layer(layer).unwrap();
    assert_eq!(after.start_time, Tick::from_seconds_f64(2.0), "offset still clips at the first source frame");
    assert_eq!((after.in_point, after.out_point), (before.in_point, before.out_point));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active_comp().unwrap().layer(layer).unwrap(), before);
}

#[test]
fn ordinary_looped_sources_keep_both_slip_limits() {
    let (mut session, layer) = setup(5.0, 2, 20.0);
    session.execute("layer.timing", json!({"layers": [layer.0], "in": 3, "out": 7})).unwrap();
    let before = session.active_comp().unwrap().layer(layer).unwrap().clone();
    for (delta, start) in [(10, 3.0), (-10, -3.0)] {
        session.execute("layer.slip", json!({"layers": [layer.0], "delta": delta})).unwrap();
        let after = session.active_comp().unwrap().layer(layer).unwrap();
        assert_eq!(after.start_time, Tick::from_seconds_f64(start));
        assert_eq!((after.in_point, after.out_point), (before.in_point, before.out_point));
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active_comp().unwrap().layer(layer).unwrap(), before);
    }
}
