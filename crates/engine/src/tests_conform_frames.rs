use crate::{
    Session,
    project::{FootageKind, ItemId, ItemKind},
    time::{FrameRate, TICKS_PER_SECOND, Tick},
};
use serde_json::json;
use std::sync::Arc;

fn footage(s: &Session, id: ItemId) -> &crate::project::Footage {
    match &s.project.item(id).unwrap().kind {
        ItemKind::Footage(f) => f,
        _ => panic!("footage fixture"),
    }
}
fn fixture(rate: FrameRate, duration: Tick) -> (Session, ItemId) {
    let mut s = Session::default();
    let id = ItemId(
        s.execute("file.importPlaceholder", json!({"name":"Video", "width":4,"height":4,"frameRate":30,"duration":1})).unwrap()["item"].as_u64().unwrap(),
    );
    match &mut Arc::make_mut(&mut s.project).item_mut(id).unwrap().kind {
        ItemKind::Footage(f) => {
            f.frame_rate = rate;
            f.duration = duration;
            f.kind = FootageKind::Video;
        }
        _ => panic!("footage fixture"),
    }
    s.history.clear();
    (s, id)
}
#[test]
fn conform_frame_count_matches_exact_source_span_at_long_half_frame_boundary() {
    let rate = FrameRate::FPS_30;
    let frames = 1_000_000_i64;
    let duration = Tick(((2 * i128::from(frames) + 1) * i128::from(TICKS_PER_SECOND) / 60 - 1) as i64);
    let (mut s, id) = fixture(rate, duration);
    let span = s.project.item(id).unwrap().source_span().unwrap();
    assert_eq!(rate.frame_at(span.0), frames, "public source bounds use the exact rational count");
    let before = s.project.clone();
    s.execute_checked("file.interpretFootage", json!({"items":[id.0],"frameRate":12})).unwrap();
    let f = footage(&s, id);
    assert_eq!(f.duration, f.frame_rate.tick_of(frames), "conforming must keep the same exact source frame count");
    assert_eq!(f.native_rate, Some(rate));
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn apply_interpretation_preserves_exact_target_count_at_long_half_frame_boundary() {
    let frames = 1_000_000_i64;
    let duration = Tick(((2 * i128::from(frames) + 1) * i128::from(TICKS_PER_SECOND) / 60 - 1) as i64);
    let (mut s, id) = fixture(FrameRate::FPS_30, duration);
    let src = ItemId(
        s.execute("file.importPlaceholder", json!({"name":"Reference","width":4,"height":4,"frameRate":30,"duration":1})).unwrap()["item"].as_u64().unwrap(),
    );
    s.execute_checked("file.interpretFootage", json!({"items":[src.0],"frameRate":12})).unwrap();
    s.execute_checked("file.rememberInterpretation", json!({"items":[src.0]})).unwrap();
    s.history.clear();
    let before = s.project.clone();
    s.execute_checked("file.applyInterpretation", json!({"items":[id.0]})).unwrap();
    let f = footage(&s, id);
    assert_eq!(f.duration, f.frame_rate.tick_of(frames), "Apply Interpretation uses the same rational count as source bounds");
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn conform_round_trip_preserves_fractional_rate_frame_count() {
    let original = FrameRate::from_f64(12.345);
    let (mut s, id) = fixture(original, original.tick_of(17));
    let duration = footage(&s, id).duration;
    s.execute_checked("file.interpretFootage", json!({"items":[id.0],"frameRate":30})).unwrap();
    assert_eq!(footage(&s, id).duration, FrameRate::FPS_30.tick_of(17));
    s.execute_checked("file.interpretFootage", json!({"items":[id.0],"frameRate":"file"})).unwrap();
    assert_eq!(footage(&s, id).duration, duration);
    assert_eq!(footage(&s, id).native_rate, None);
}

#[test]
fn conform_sequence_uses_actual_file_count_and_preserves_single_undo() {
    let (mut s, id) = fixture(FrameRate::FPS_30, FrameRate::FPS_30.tick_of(30));
    match &mut Arc::make_mut(&mut s.project).item_mut(id).unwrap().kind {
        ItemKind::Footage(f) => {
            f.kind = FootageKind::Sequence;
            f.sequence = vec!["1.png".into(), "2.png".into(), "3.png".into()];
        }
        _ => panic!("footage fixture"),
    }
    assert_eq!(FrameRate::FPS_30.frame_at(s.project.item(id).unwrap().source_span().unwrap().0), 3);
    let before = s.project.clone();
    s.execute_checked("file.interpretFootage", json!({"items": [id.0], "frameRate": 12})).unwrap();
    let f = footage(&s, id);
    assert_eq!(f.duration, f.frame_rate.tick_of(3));
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
