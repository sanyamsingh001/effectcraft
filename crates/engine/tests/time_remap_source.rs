//! Default Time Remap keys stop at finite sources before extended strips' held regions.
use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    render::{EvalCtx, RenderOpts},
    time::{FrameRate, Tick},
};
use serde_json::json;

fn setup() -> (Session, ItemId, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Source", "width":100,"height":100,"frameRate":24,"duration":2})).unwrap();
    let source = s.active_comp_id().unwrap();
    let solid = s.execute("layer.newSolid", json!({"width":10,"height":10,"color":"#ffffff"})).unwrap()["layer"].as_u64().unwrap();
    for (time, x) in [(0.0, 10.0), (2.0, 90.0)] {
        s.execute("prop.addKey", json!({"layer":solid,"path":"transform/position","time":time,"value":[x,50]})).unwrap();
    }
    s.execute("comp.new", json!({"name":"Outer", "width":100,"height":100,"frameRate":30,"duration":12})).unwrap();
    let layer = s.execute("layer.addItem", json!({"item":source.0})).unwrap()["layer"].as_u64().unwrap();
    (s, source, layer)
}

fn source_time(s: &Session, layer: u64, time: f64) -> Tick {
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    EvalCtx::new(&s.project, cid, comp, Tick::from_seconds_f64(time)).source_time(comp.layer(LayerId(layer)).unwrap())
}

#[test]
fn extended_precomp_holds_its_last_source_frame_and_undo_restores_timing() {
    let (mut s, source, layer) = setup();
    s.execute("layer.timeStretch", json!({"layers":[layer], "percent":200})).unwrap();
    s.execute("layer.timing", json!({"layers":[layer],"delta":1,"out":7})).unwrap();
    let before = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().clone();
    let moving = source_time(&s, layer, 2.0);
    let last = Tick::from_seconds_f64(2.0) - FrameRate::FPS_24.frame_duration();
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    let remap = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap();
    assert_eq!(remap.keys.last().unwrap().time, last, "source frame rate chooses the last frame");
    assert_eq!(source_time(&s, layer, 2.0), moving, "moving content keeps its time mapping");
    assert_eq!(source_time(&s, layer, 6.0), last);
    let reference = s.render(source, last, RenderOpts::default());
    let held = s.render(s.active_comp_id().unwrap(), Tick::from_seconds_f64(6.0), RenderOpts::default());
    assert!(reference.get(88, 50)[3] > 0.0, "last source frame contains the moving square");
    for y in 0..100 {
        for x in 0..100 {
            assert_eq!(held.get(x, y), reference.get(x, y), "held tail differs at {x},{y}");
        }
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active_comp().unwrap().layer(LayerId(layer)).unwrap(), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(source_time(&s, layer, 6.0), last);
}

#[test]
fn extended_reverse_and_trims_hold_the_source_edges_without_new_animation() {
    let (mut s, _, layer) = setup();
    s.execute("layer.timeStretch", json!({"layers":[layer],"percent":200})).unwrap();
    s.execute("layer.timeReverse", json!({"layers":[layer]})).unwrap();
    s.execute("layer.timing", json!({"layers":[layer],"delta":1,"out":7})).unwrap();
    let moving = source_time(&s, layer, 2.0);
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    assert_eq!(source_time(&s, layer, 2.0), moving);
    assert_eq!(source_time(&s, layer, 6.0), Tick::ZERO, "reverse tail holds the first source frame");
    let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap().keys;
    assert_eq!(keys.first().unwrap().time, Tick::ZERO);
    assert_eq!(keys.len(), 2);

    for (start, inp, outp, expected) in [(0, 5, 8, Tick::from_seconds_f64(2.0) - FrameRate::FPS_24.frame_duration()), (3, 0, 2, Tick::ZERO)] {
        let (mut s, _, layer) = setup();
        s.execute("layer.timing", json!({"layers":[layer],"start":start,"in":inp,"out":outp})).unwrap();
        s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
        assert_eq!(s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap().keys.len(), 1);
        assert_eq!(source_time(&s, layer, f64::from(inp + 1)), expected);
    }
}

#[test]
fn source_frames_follow_interpreted_rates_sequence_lengths_and_loops() {
    let (mut s, _, _) = setup();
    let footage = Footage {
        kind: FootageKind::Video,
        frame_rate: FrameRate::FPS_24,
        native_rate: Some(FrameRate::FPS_60),
        duration: Tick::from_seconds_f64(2.0),
        width: 100,
        height: 100,
        has_video: true,
        loop_count: 2,
        ..Default::default()
    };
    let item = std::sync::Arc::make_mut(&mut s.project).add_item("Conformed", effectcraft_engine::color::Label::Aqua, None, ItemKind::Footage(footage));
    let layer = s.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.timing", json!({"layers":[layer],"out":7})).unwrap();
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    let last = Tick::from_seconds_f64(4.0) - FrameRate::FPS_24.frame_duration();
    assert_eq!(source_time(&s, layer, 6.0), last);
    assert_eq!(s.project.item(item).unwrap().source_span(), Some((Tick::from_seconds_f64(4.0), last)));
    let sequence = Footage {
        kind: FootageKind::Sequence,
        frame_rate: FrameRate::FPS_24,
        sequence: vec!["a.png".into(), "b.png".into(), "c.png".into()],
        duration: Tick::from_seconds_f64(99.0),
        loop_count: 2,
        ..Default::default()
    };
    let item = std::sync::Arc::make_mut(&mut s.project).add_item("Sequence", effectcraft_engine::color::Label::Aqua, None, ItemKind::Footage(sequence));
    assert_eq!(s.project.item(item).unwrap().source_span(), Some((FrameRate::FPS_24.tick_of(6), FrameRate::FPS_24.tick_of(5))));
}

#[test]
fn ordinary_and_still_layers_keep_their_existing_default_keys() {
    let (mut s, _, layer) = setup();
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap().keys;
    assert_eq!(keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), [0.0, 2.0]);
    assert_eq!(keys.last().unwrap().value.as_f64(), 2.0);
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":false})).unwrap();
    s.execute("layer.timeReverse", json!({"layers":[layer]})).unwrap();
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap().keys;
    assert_eq!(keys.first().unwrap().time, -FrameRate::FPS_30.frame_duration());
    assert_eq!(keys.first().unwrap().value.as_f64(), 0.0);

    let still = Footage { kind: FootageKind::Still, width: 100, height: 100, has_video: true, ..Default::default() };
    let item = std::sync::Arc::make_mut(&mut s.project).add_item("Still", effectcraft_engine::color::Label::Aqua, None, ItemKind::Footage(still));
    assert!(s.project.item(item).unwrap().source_span().is_none());
    let layer = s.execute("layer.addItem", json!({"item":item.0})).unwrap()["layer"].as_u64().unwrap();
    let points = {
        let l = s.active_comp().unwrap().layer(LayerId(layer)).unwrap();
        [l.layer_time(l.in_point), l.layer_time(l.out_point)]
    };
    s.execute("layer.enableTimeRemap", json!({"layers":[layer],"value":true})).unwrap();
    assert_eq!(s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.get("timeRemap").unwrap().keys.iter().map(|k| k.time).collect::<Vec<_>>(), points);
}
