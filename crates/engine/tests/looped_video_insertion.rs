//! Default layer and composition lengths include interpreted Video loops.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;
fn add(s: &mut Session, kind: FootageKind, loops: u32) -> ItemId {
    Arc::make_mut(&mut s.project).add_item(
        "Footage",
        Default::default(),
        None,
        ItemKind::Footage(Footage {
            kind,
            frame_rate: FrameRate::FPS_30,
            duration: FrameRate::FPS_30.tick_of(60),
            loop_count: loops,
            width: 4,
            height: 4,
            has_video: kind == FootageKind::Video,
            has_audio: kind == FootageKind::Audio,
            ..Default::default()
        }),
    )
}
#[test]
fn default_looped_video_layer_uses_its_complete_source_span() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":10})).unwrap();
    let item = add(&mut s, FootageKind::Video, 2);
    let span = s.project.item(item).unwrap().source_span().unwrap().0;
    assert_eq!(span, Tick::from_seconds_f64(4.0));
    let layer = LayerId(s.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(s.active_comp().unwrap().layer(layer).unwrap().out_point, span);
    s.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":15})).unwrap();
    assert_eq!(s.active_comp().unwrap().layer(layer).unwrap().out_point, Tick::from_seconds_f64(8.0));
}
#[test]
fn new_comp_from_looped_video_uses_its_complete_source_span() {
    let mut s = Session::default();
    let item = add(&mut s, FootageKind::Video, 2);
    s.state.project_selection = vec![item];
    let undo = s.history.undo.len();
    let result = s.execute("file.newCompFromSelection", json!({})).unwrap();
    let comp = ItemId(result["comps"][0].as_u64().unwrap());
    let c = s.project.comp(comp).unwrap();
    assert_eq!(c.duration, Tick::from_seconds_f64(4.0));
    assert_eq!(c.layers[0].out_point, c.duration);
    assert_eq!(s.history.undo.len(), undo + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.comp(comp).is_none());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().duration, Tick::from_seconds_f64(4.0));
}
#[test]
fn unlooped_video_and_looped_audio_keep_their_existing_lengths() {
    for (kind, loops) in [(FootageKind::Video, 1), (FootageKind::Audio, 2)] {
        let mut s = Session::default();
        s.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":10})).unwrap();
        let item = add(&mut s, kind, loops);
        let layer = LayerId(s.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(s.active_comp().unwrap().layer(layer).unwrap().out_point, Tick::from_seconds_f64(2.0));
        s.state.project_selection = vec![item];
        let result = s.execute("file.newCompFromSelection", json!({})).unwrap();
        let comp = ItemId(result["comps"][0].as_u64().unwrap());
        assert_eq!(s.project.comp(comp).unwrap().duration, Tick::from_seconds_f64(2.0));
    }
}
