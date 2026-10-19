//! Sequence layer and composition bounds follow filenames rather than stale duration.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;
fn setup(stale: bool) -> (Session, ItemId, Tick) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":24,"duration":10})).unwrap();
    let rate = FrameRate::FPS_24;
    let f = Footage {
        kind: FootageKind::Sequence,
        width: 4,
        height: 4,
        has_video: true,
        frame_rate: rate,
        duration: if stale { Tick::from_seconds_f64(99.0) } else { rate.tick_of(3) },
        loop_count: if stale { 1 } else { 2 },
        sequence: (0..3).map(|i| format!("frame{i:04}.png")).collect(),
        ..Default::default()
    };
    let item = Arc::make_mut(&mut s.project).add_item("Sequence", Default::default(), None, ItemKind::Footage(f));
    let expected = rate.tick_of(if stale { 3 } else { 6 });
    assert_eq!(s.project.item(item).unwrap().source_span().unwrap().0, expected);
    (s, item, expected)
}
fn layer_case(stale: bool) {
    let (mut s, item, expected) = setup(stale);
    let layer = LayerId(s.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(s.active_comp().unwrap().layer(layer).unwrap().out_point, expected);
}
fn comp_case(stale: bool) {
    let (mut s, item, expected) = setup(stale);
    s.state.project_selection = vec![item];
    let undo = s.history.undo.len();
    let result = s.execute("file.newCompFromSelection", json!({})).unwrap();
    let cid = ItemId(result["comps"][0].as_u64().unwrap());
    let comp = s.project.comp(cid).unwrap();
    assert_eq!(comp.duration, expected);
    assert_eq!(comp.layers[0].out_point, expected);
    assert_eq!(s.history.undo.len(), undo + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.comp(cid).is_none());
}
#[test]
fn looped_sequence_layer_uses_authoritative_frame_count() {
    layer_case(false);
}
#[test]
fn stale_sequence_layer_uses_authoritative_frame_count() {
    layer_case(true);
}
#[test]
fn looped_sequence_new_comp_uses_authoritative_frame_count() {
    comp_case(false);
}
#[test]
fn stale_sequence_new_comp_uses_authoritative_frame_count() {
    comp_case(true);
}
