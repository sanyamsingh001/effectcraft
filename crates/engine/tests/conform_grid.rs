//! Full source clips keep their composition-grid endpoint when conformed.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemKind, LayerId},
    time::FrameRate,
};
use serde_json::json;
use std::sync::Arc;
fn grid_case(apply: bool) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":24,"duration":10})).unwrap();
    let comp = s.active_comp_id().unwrap();
    let mut footage = |name: &str| {
        Arc::make_mut(&mut s.project).add_item(
            name,
            Default::default(),
            None,
            ItemKind::Footage(Footage {
                kind: FootageKind::Video,
                frame_rate: FrameRate::FPS_30,
                duration: FrameRate::FPS_30.tick_of(1),
                width: 4,
                height: 4,
                has_video: true,
                ..Default::default()
            }),
        )
    };
    let source = footage("Remembered");
    let target = footage("Target");
    let full = LayerId(s.execute("layer.addItem", json!({"item":target.0})).unwrap()["layer"].as_u64().unwrap());
    let shifted = LayerId(s.execute("layer.addItem", json!({"item":target.0,"time":0.125})).unwrap()["layer"].as_u64().unwrap());
    let rate = FrameRate::FPS_24;
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, rate.tick_of(1));
    assert_eq!(s.project.comp(comp).unwrap().layer(shifted).unwrap().out_point, rate.tick_of(4));
    assert_ne!(rate.tick_of(1), s.project.item(target).unwrap().source_span().unwrap().0);
    if apply {
        s.execute("file.interpretFootage", json!({"items":[source.0],"frameRate":15})).unwrap();
        s.execute("file.rememberInterpretation", json!({"items":[source.0]})).unwrap();
        s.execute("file.applyInterpretation", json!({"items":[target.0]})).unwrap();
    } else {
        s.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":15})).unwrap();
    }
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, rate.tick_of(2));
    assert_eq!(s.project.comp(comp).unwrap().layer(shifted).unwrap().out_point, rate.tick_of(5));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, rate.tick_of(1));
    assert_eq!(s.project.comp(comp).unwrap().layer(shifted).unwrap().out_point, rate.tick_of(4));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, rate.tick_of(2));
}
#[test]
fn direct_conform_follows_composition_grid_for_default_full_layers() {
    grid_case(false);
}
#[test]
fn applying_conform_follows_composition_grid_for_default_full_layers() {
    grid_case(true);
}
#[test]
fn shortened_one_frame_source_keeps_a_composition_frame() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":24,"duration":10})).unwrap();
    let comp = s.active_comp_id().unwrap();
    let target = Arc::make_mut(&mut s.project).add_item(
        "Target",
        Default::default(),
        None,
        ItemKind::Footage(Footage {
            kind: FootageKind::Video,
            frame_rate: FrameRate::FPS_30,
            duration: FrameRate::FPS_30.tick_of(1),
            width: 4,
            height: 4,
            has_video: true,
            ..Default::default()
        }),
    );
    let full = LayerId(s.execute("layer.addItem", json!({"item":target.0})).unwrap()["layer"].as_u64().unwrap());
    s.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":60})).unwrap();
    assert_eq!(s.project.comp(comp).unwrap().layer(full).unwrap().out_point, FrameRate::FPS_24.tick_of(1));
}

#[test]
fn legacy_exact_full_length_endpoints_keep_their_existing_timing() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":4,"height":4,"frameRate":24,"duration":10})).unwrap();
    let comp = s.active_comp_id().unwrap();
    let target = Arc::make_mut(&mut s.project).add_item(
        "Legacy",
        Default::default(),
        None,
        ItemKind::Footage(Footage {
            kind: FootageKind::Video,
            frame_rate: FrameRate::FPS_30,
            duration: FrameRate::FPS_30.tick_of(60),
            width: 4,
            height: 4,
            has_video: true,
            ..Default::default()
        }),
    );
    let layer = LayerId(s.execute("layer.addItem", json!({"item":target.0})).unwrap()["layer"].as_u64().unwrap());
    {
        let c = Arc::make_mut(&mut s.project).comp_mut(comp).unwrap();
        let l = c.layer_mut(layer).unwrap();
        l.start_time = effectcraft_engine::time::Tick::from_seconds_f64(0.01);
        l.out_point = l.start_time + effectcraft_engine::time::Tick::from_seconds_f64(2.0);
    }
    s.execute("file.interpretFootage", json!({"items":[target.0],"frameRate":15})).unwrap();
    let l = s.project.comp(comp).unwrap().layer(layer).unwrap();
    assert_eq!(l.out_point, l.start_time + effectcraft_engine::time::Tick::from_seconds_f64(4.0));
}
