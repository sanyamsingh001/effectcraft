//! Looped footage conforming follows full source spans and preserves layer edits.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;
fn setup() -> (Session, ItemId, ItemId, ItemId, LayerId, LayerId, LayerId, ItemId, LayerId) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":20})).unwrap();
    let mut add = |name: &str, frames: i64| {
        Arc::make_mut(&mut s.project).add_item(
            name,
            Default::default(),
            None,
            ItemKind::Footage(Footage {
                kind: FootageKind::Video,
                frame_rate: FrameRate::FPS_30,
                duration: FrameRate::FPS_30.tick_of(frames),
                loop_count: 2,
                width: 4,
                height: 4,
                has_video: true,
                ..Default::default()
            }),
        )
    };
    let source = add("Remembered", 30);
    let target = add("Target", 60);
    let mut layer = || LayerId(s.execute("layer.addItem", json!({"item":target.0})).unwrap()["layer"].as_u64().unwrap());
    let full = layer();
    let trim = layer();
    let stretch = layer();
    let comp = s.active_comp_id().unwrap();
    {
        let c = Arc::make_mut(&mut s.project).comp_mut(comp).unwrap();
        let l = c.layer_mut(full).unwrap();
        l.start_time = Tick::from_seconds_f64(1.0);
        l.out_point = Tick::from_seconds_f64(5.0);
        c.layer_mut(trim).unwrap().out_point = Tick::from_seconds_f64(2.0);
        let l = c.layer_mut(stretch).unwrap();
        l.stretch = 200.0;
        l.out_point = Tick::from_seconds_f64(8.0);
    }
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":20})).unwrap();
    let other = s.active_comp_id().unwrap();
    let other_full = LayerId(s.execute("layer.addItem", json!({"item":target.0})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.timing", json!({"layers":[other_full.0],"out":4.0})).unwrap();
    (s, source, target, comp, full, trim, stretch, other, other_full)
}
fn conform(s: &mut Session, source: ItemId, target: ItemId, apply: bool) {
    if apply {
        s.execute("file.interpretFootage", json!({"items":[source.0],"frameRate":15})).unwrap();
        s.execute("file.rememberInterpretation", json!({"items":[source.0]})).unwrap();
        s.execute("file.applyInterpretation", json!({"items":[target.0]})).unwrap();
    } else {
        s.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":15})).unwrap();
    }
}
fn full_case(apply: bool) {
    let (mut s, source, target, comp, full, _, _, other, other_full) = setup();
    conform(&mut s, source, target, apply);
    assert_eq!(s.project.item(target).unwrap().source_span().unwrap().0, Tick::from_seconds_f64(8.0));
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, Tick::from_seconds_f64(9.0), "shifted full clip follows the looped8s source");
    assert_eq!(s.project.comp(other).unwrap().layer(other_full).unwrap().out_point, Tick::from_seconds_f64(8.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, Tick::from_seconds_f64(5.0));
    assert_eq!(s.project.comp(other).unwrap().layer(other_full).unwrap().out_point, Tick::from_seconds_f64(4.0));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, Tick::from_seconds_f64(9.0));
}
fn trim_case(apply: bool) {
    let (mut s, source, target, comp, _, trim, stretch, _, _) = setup();
    conform(&mut s, source, target, apply);
    assert_eq!(
        s.project.comp(comp).unwrap().layer(trim).unwrap().out_point,
        Tick::from_seconds_f64(2.0),
        "a clip trimmed to the old per-loop duration stays trimmed"
    );
    assert_eq!(s.project.comp(comp).unwrap().layer(stretch).unwrap().out_point, Tick::from_seconds_f64(8.0), "stretched edits retain their existing out point");
}
#[test]
fn direct_looped_conform_retimes_full_layers() {
    full_case(false);
}
#[test]
fn applying_looped_conform_retimes_full_layers() {
    full_case(true);
}
#[test]
fn direct_looped_conform_preserves_trim_and_stretch() {
    trim_case(false);
}
#[test]
fn applying_looped_conform_preserves_trim_and_stretch() {
    trim_case(true);
}
#[test]
fn changing_only_loop_count_retimes_full_layers() {
    let (mut s, _, target, comp, full, trim, stretch, other, other_full) = setup();
    s.execute("file.interpretFootage", json!({"items":[target.0],"loop":3})).unwrap();
    assert_eq!(s.project.item(target).unwrap().source_span().unwrap().0, Tick::from_seconds_f64(6.0));
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, Tick::from_seconds_f64(7.0));
    assert_eq!(s.project.comp(other).unwrap().layer(other_full).unwrap().out_point, Tick::from_seconds_f64(6.0));
    assert_eq!(s.project.comp(comp).unwrap().layer(trim).unwrap().out_point, Tick::from_seconds_f64(2.0));
    assert_eq!(s.project.comp(comp).unwrap().layer(stretch).unwrap().out_point, Tick::from_seconds_f64(8.0));
}
