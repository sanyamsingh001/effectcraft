//! Applying a remembered conform preserves source frames and existing layer edits.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

fn setup() -> Session {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":10})).unwrap();
    session
}
fn footage(session: &mut Session, name: &str, rate: FrameRate, frames: i64) -> ItemId {
    let clip =
        Footage { kind: FootageKind::Video, frame_rate: rate, duration: rate.tick_of(frames), width: 4, height: 4, has_video: true, ..Default::default() };
    Arc::make_mut(&mut session.project).add_item(name, Default::default(), None, ItemKind::Footage(clip))
}
fn add_layer(session: &mut Session, item: ItemId) -> LayerId {
    LayerId(session.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap())
}
fn get_footage(session: &Session, item: ItemId) -> &Footage {
    let ItemKind::Footage(footage) = &session.project.item(item).unwrap().kind else { panic!("footage") };
    footage
}
fn remember_rate(session: &mut Session, item: ItemId, rate: f64) {
    session.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":rate})).unwrap();
    session.execute("file.rememberInterpretation", json!({"items":[item.0]})).unwrap();
}

#[test]
fn applying_conformed_video_rate_preserves_frames_and_full_length_layers() {
    let mut session = setup();
    let source = footage(&mut session, "Source", FrameRate::FPS_30, 30);
    let target = footage(&mut session, "Target", FrameRate::FPS_30, 60);
    let full = add_layer(&mut session, target);
    let trim = add_layer(&mut session, target);
    let stretch = add_layer(&mut session, target);
    let comp = session.active_comp_id().unwrap();
    {
        let layers = Arc::make_mut(&mut session.project).comp_mut(comp).unwrap();
        let full = layers.layer_mut(full).unwrap();
        full.start_time = Tick::from_seconds_f64(1.0);
        full.out_point = Tick::from_seconds_f64(3.0);
        layers.layer_mut(trim).unwrap().out_point = Tick::from_seconds_f64(0.5);
        let stretch = layers.layer_mut(stretch).unwrap();
        stretch.stretch = 200.0;
        stretch.out_point = Tick::from_seconds_f64(4.0);
    }
    session.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":10})).unwrap();
    let other_comp = session.active_comp_id().unwrap();
    let other_full = add_layer(&mut session, target);
    remember_rate(&mut session, source, 15.0);
    session.execute("file.applyInterpretation", json!({"items":[target.0]})).unwrap();
    let clip = get_footage(&session, target);
    assert_eq!(clip.frame_rate, FrameRate::new(15, 1));
    assert_eq!(clip.native_rate, Some(FrameRate::FPS_30));
    assert_eq!(clip.duration, Tick::from_seconds_f64(4.0));
    assert_eq!(session.project.item(target).unwrap().source_span(), Some((clip.frame_rate.tick_of(60), clip.frame_rate.tick_of(59))));
    let layers = session.project.comp(comp).unwrap();
    assert_eq!(layers.layer(full).unwrap().out_point, Tick::from_seconds_f64(5.0));
    assert_eq!(layers.layer(trim).unwrap().out_point, Tick::from_seconds_f64(0.5));
    assert_eq!(layers.layer(stretch).unwrap().out_point, Tick::from_seconds_f64(4.0));
    assert_eq!(session.project.comp(other_comp).unwrap().layer(other_full).unwrap().out_point, Tick::from_seconds_f64(4.0));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(get_footage(&session, target).duration, Tick::from_seconds_f64(2.0));
    assert_eq!(get_footage(&session, target).native_rate, None);
    assert_eq!(session.project.comp(comp).unwrap().layer(full).unwrap().out_point, Tick::from_seconds_f64(3.0));
    assert_eq!(session.project.comp(other_comp).unwrap().layer(other_full).unwrap().out_point, Tick::from_seconds_f64(2.0));
}

#[test]
fn applying_another_conform_keeps_the_targets_own_native_rate_for_restore() {
    let mut session = setup();
    let source = footage(&mut session, "Source", FrameRate::FPS_30, 30);
    let target = footage(&mut session, "Target", FrameRate::FPS_24, 48);
    let layer = add_layer(&mut session, target);
    session.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":12})).unwrap();
    remember_rate(&mut session, source, 15.0);
    session.execute("file.applyInterpretation", json!({"items":[target.0]})).unwrap();
    let clip = get_footage(&session, target);
    assert_eq!(clip.native_rate, Some(FrameRate::FPS_24));
    assert_eq!(clip.duration, FrameRate::new(15, 1).tick_of(48));
    assert_eq!(session.active_comp().unwrap().layer(layer).unwrap().out_point, clip.duration);
    session.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":"file"})).unwrap();
    assert_eq!(get_footage(&session, target).frame_rate, FrameRate::FPS_24);
    assert_eq!(get_footage(&session, target).native_rate, None);
    assert_eq!(get_footage(&session, target).duration, Tick::from_seconds_f64(2.0));
    assert_eq!(session.active_comp().unwrap().layer(layer).unwrap().out_point, Tick::from_seconds_f64(2.0));
}

#[test]
fn remembered_unconformed_footage_does_not_replace_a_targets_rate() {
    let mut session = setup();
    let source = footage(&mut session, "Source", FrameRate::FPS_24, 24);
    let target = footage(&mut session, "Target", FrameRate::FPS_30, 60);
    session.execute("file.interpretFootage", json!({"items":[source.0],"alpha":"ignore"})).unwrap();
    session.execute("file.rememberInterpretation", json!({"items":[source.0]})).unwrap();
    session.execute("file.applyInterpretation", json!({"items":[target.0]})).unwrap();
    let clip = get_footage(&session, target);
    assert_eq!(clip.frame_rate, FrameRate::FPS_30);
    assert_eq!(clip.duration, Tick::from_seconds_f64(2.0));
    assert_eq!(clip.native_rate, None);
    assert_eq!(clip.alpha, effectcraft_engine::project::AlphaMode::Ignore);
}
