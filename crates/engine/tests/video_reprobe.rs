//! Conforming survives metadata refresh while the file's native count/rate can change.
use effectcraft_engine::{
    Importer, Session,
    project::{Footage, FootageKind, ItemId, ItemKind},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

struct NativeMetadata {
    rate: FrameRate,
    frames: i64,
}
impl Importer for NativeMetadata {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        Ok(Footage {
            path: path.into(),
            kind: FootageKind::Video,
            frame_rate: self.rate,
            duration: self.rate.tick_of(self.frames),
            width: 4,
            height: 4,
            has_video: true,
            ..Default::default()
        })
    }
}
fn setup(native_rate: FrameRate, native_frames: i64, name: &str) -> (Session, ItemId) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/engine/video-reprobe");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(name);
    std::fs::write(&path, b"public Importer supplies this fixture's native metadata").unwrap();
    let mut session = Session { importer: Some(Arc::new(NativeMetadata { rate: native_rate, frames: native_frames })), ..Default::default() };
    session.execute("comp.new", json!({"width":4,"height":4,"duration":10})).unwrap();
    let footage = Footage {
        path: path.to_string_lossy().into(),
        kind: FootageKind::Video,
        frame_rate: FrameRate::FPS_30,
        duration: FrameRate::FPS_30.tick_of(60),
        width: 4,
        height: 4,
        has_video: true,
        ..Default::default()
    };
    let item = Arc::make_mut(&mut session.project).add_item("Video", Default::default(), None, ItemKind::Footage(footage));
    session.execute("layer.addItem", json!({"item":item.0})).unwrap();
    session.execute("file.interpretFootage",json!({"items":[item.0],"frameRate":15,"alpha":"premultiplied","matteColor":[0.2,0.3,0.4],"invertAlpha":true,"fields":"upper","colorProfile":"rec2020","linearLight":true,"pixelAspect":1.3,"loop":2})).unwrap();
    assert_eq!(get(&session, item).duration, Tick::from_seconds_f64(4.0));
    (session, item)
}
fn get(session: &Session, item: ItemId) -> &Footage {
    let ItemKind::Footage(f) = &session.project.item(item).unwrap().kind else { panic!("footage") };
    f
}
fn verify(session: &Session, item: ItemId, before: &Footage, native: FrameRate, frames: i64) {
    let after = get(session, item);
    eprintln!(
        "after native probe: effective={:?} native={:?} duration={:?} invert={} matte={:?}",
        after.frame_rate, after.native_rate, after.duration, after.invert_alpha, after.premul_color
    );
    assert_eq!(after.frame_rate, FrameRate::new(15, 1), "metadata refresh must preserve the15fps conform");
    assert_eq!(after.native_rate, Some(native));
    assert_eq!(after.duration, FrameRate::new(15, 1).tick_of(frames), "use the freshly probed source frame count");
    assert_eq!(
        (after.alpha, after.premul_color, after.invert_alpha, after.fields, after.color_profile, after.linear_light, after.pixel_aspect, after.loop_count),
        (
            before.alpha,
            before.premul_color,
            before.invert_alpha,
            before.fields,
            before.color_profile,
            before.linear_light,
            before.pixel_aspect,
            before.loop_count
        )
    );
}
#[test]
fn video_reload_preserves_interpretation_and_restores_the_files_rate() {
    let (mut session, item) = setup(FrameRate::FPS_30, 90, "reload.mp4");
    let before = get(&session, item).clone();
    session.execute("file.reloadFootage", json!({"items":[item.0]})).unwrap();
    verify(&session, item, &before, FrameRate::FPS_30, 90);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(get(&session, item), &before);
    session.execute("edit.redo", json!({})).unwrap();
    session.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
    assert_eq!(get(&session, item).frame_rate, FrameRate::FPS_30);
    assert_eq!(get(&session, item).native_rate, None);
    assert_eq!(get(&session, item).duration, Tick::from_seconds_f64(3.0));
}
#[test]
fn dimensionless_video_check_preserves_interpretation_without_an_undo_step() {
    let (mut session, item) = setup(FrameRate::FPS_30, 60, "check.mp4");
    let ItemKind::Footage(f) = &mut Arc::make_mut(&mut session.project).item_mut(item).unwrap().kind else { panic!("footage") };
    f.width = 0;
    f.height = 0;
    let before = get(&session, item).clone();
    session.mark_saved();
    let undo = session.history.undo.len();
    let result = session.execute("footage.check", json!({"items":[item.0],"wait":true})).unwrap();
    assert_eq!(result, json!({"checked":1,"missing":0,"probed":1}));
    verify(&session, item, &before, FrameRate::FPS_30, 60);
    assert_eq!((get(&session, item).width, get(&session, item).height), (4, 4));
    assert_eq!(session.history.undo.len(), undo);
    assert!(!session.is_dirty());
    session.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
    assert_eq!(get(&session, item).frame_rate, FrameRate::FPS_30);
    assert_eq!(get(&session, item).duration, Tick::from_seconds_f64(2.0));
}
#[test]
fn reload_uses_changed_native_metadata_without_losing_the_conform() {
    let (mut session, item) = setup(FrameRate::FPS_24, 48, "changed-native.mp4");
    let before = get(&session, item).clone();
    session.execute("file.reloadFootage", json!({"items":[item.0]})).unwrap();
    verify(&session, item, &before, FrameRate::FPS_24, 48);
    session.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
    assert_eq!(get(&session, item).frame_rate, FrameRate::FPS_24);
    assert_eq!(get(&session, item).native_rate, None);
    assert_eq!(get(&session, item).duration, Tick::from_seconds_f64(2.0));
}

#[test]
fn unconformed_reload_adopts_fresh_native_metadata() {
    let (mut session, item) = setup(FrameRate::FPS_24, 48, "unconformed.mp4");
    session.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
    session.execute("file.reloadFootage", json!({"items":[item.0]})).unwrap();
    assert_eq!(get(&session, item).frame_rate, FrameRate::FPS_24);
    assert_eq!(get(&session, item).native_rate, None);
    assert_eq!(get(&session, item).duration, Tick::from_seconds_f64(2.0));
}

#[test]
fn unchanged_reload_preserves_interpretation_without_an_undo_step() {
    let (mut session, item) = setup(FrameRate::FPS_30, 60, "noop-reload.mp4");
    let before = get(&session, item).clone();
    let undo = session.history.undo.len();
    session.execute("file.reloadFootage", json!({"items":[item.0]})).unwrap();
    assert_eq!(get(&session, item), &before);
    assert_eq!(session.history.undo.len(), undo, "unchanged native metadata is a no-op reload");
}
