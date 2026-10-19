//! Moving keys preserves selection identity, order and merged gestures.
use effectcraft_engine::{
    KeyRef, Session,
    keyframe::{Keyframe, Value},
    project::LayerId,
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;
fn setup() -> (Session, u64) {
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":4})).unwrap();
    let layer = s.execute_checked("layer.newSolid", json!({"name":"A","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    (s, layer)
}
fn uid(s: &Session, layer: u64, path: &str) -> u64 {
    s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop(path).unwrap().uid
}
fn add(s: &mut Session, layer: u64, path: &str, time: f64, value: serde_json::Value) {
    s.execute_checked("prop.addKey", json!({"layer":layer,"path":path,"time":time,"value":value})).unwrap();
}
fn select(s: &mut Session, targets: serde_json::Value) {
    s.execute_checked("keys.select", json!({"keys":targets})).unwrap();
}
fn loaded_close_keys(s: &mut Session, layer: u64, prop: u64) {
    let cid = s.active_comp_id().unwrap();
    let property = Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(layer)).unwrap().props.find_mut(prop).unwrap();
    property.keys = vec![Keyframe::new(Tick::ZERO, Value::Scalar(0.0)), Keyframe::new(Tick::from_seconds_f64(0.01), Value::Scalar(50.0))];
}
#[test]
fn move_keeps_interleaved_selection_order() {
    let (mut s, a) = setup();
    let b = s.execute_checked("layer.newSolid", json!({"name":"B","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    add(&mut s, a, "transform/opacity", 0.0, json!(10));
    add(&mut s, a, "transform/position", 1.0, json!([20, 30]));
    add(&mut s, b, "transform/opacity", 1.0, json!(20));
    let ap = uid(&s, a, "transform/opacity");
    let pos = uid(&s, a, "transform/position");
    let bp = uid(&s, b, "transform/opacity");
    select(&mut s, json!([{"layer":b,"prop":bp,"time":1},{"layer":a,"prop":pos,"time":1},{"layer":a,"prop":ap,"time":0}]));
    let mut expected = s.state.selected_keys.clone();
    for key in &mut expected {
        key.time += FrameRate::FPS_30.tick_of(1);
    }
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("keys.move", json!({"delta":1.0/30.0})).unwrap();
    assert_eq!(s.state.selected_keys, expected, "moving preserves the caller's interleaved order for its next stagger gesture");
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn move_deduplicates_snapped_destinations() {
    let (mut s, layer) = setup();
    let prop = uid(&s, layer, "transform/opacity");
    loaded_close_keys(&mut s, layer, prop);
    s.execute_checked("prop.select", json!({"layer":layer,"prop":prop})).unwrap();
    assert_eq!(s.state.selected_keys.len(), 2);
    let before = s.project.clone();
    s.history.clear();
    let result = s.execute_checked("keys.move", json!({"delta":1.0/30.0})).unwrap();
    let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(prop).unwrap().keys;
    assert_eq!(keys.len(), 1, "snapping merges the loaded subframe keys");
    assert_eq!(keys[0].value, Value::Scalar(50.0), "retain the existing winner behavior");
    assert_eq!(result, json!(1), "one surviving selected key");
    assert_eq!(s.state.selected_keys, vec![KeyRef { layer: LayerId(layer), prop, time: FrameRate::FPS_30.tick_of(1) }]);
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn move_tracks_roving_identity_in_original_order() {
    let (mut s, a) = setup();
    for (time, x) in [(0.0, 0), (1.0, 25), (2.0, 100)] {
        add(&mut s, a, "transform/position", time, json!([x, 0]));
    }
    let pos = uid(&s, a, "transform/position");
    select(&mut s, json!([{"layer":a,"prop":pos,"time":1}]));
    s.execute_checked("keys.interpolation", json!({"roving":true})).unwrap();
    let b = s.execute_checked("layer.newSolid", json!({"name":"B","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    add(&mut s, b, "transform/opacity", 0.0, json!(50));
    let bp = uid(&s, b, "transform/opacity");
    select(&mut s, json!([{"layer":b,"prop":bp,"time":0},{"layer":a,"prop":pos,"time":0.5},{"layer":a,"prop":pos,"time":2},{"layer":a,"prop":pos,"time":0}]));
    let mut expected = s.state.selected_keys.clone();
    for key in &mut expected {
        key.time += FrameRate::FPS_30.tick_of(1);
    }
    s.history.clear();
    s.execute_checked("keys.move", json!({"delta":1.0/30.0})).unwrap();
    let keys = &s.active_comp().unwrap().layer(LayerId(a)).unwrap().props.find(pos).unwrap().keys;
    let middle = keys.iter().find(|key| key.value.as_vec2() == [25.0, 0.0]).unwrap();
    assert_eq!(middle.time, FrameRate::FPS_30.tick_of(16));
    assert!(middle.roving);
    assert_eq!(s.state.selected_keys, expected, "the roving identity and unrelated selection retain their original positions");
    assert_eq!(s.history.undo.len(), 1);
}
#[test]
fn move_merge_recovers_original_keys_after_a_snapped_collision() {
    let (mut s, layer) = setup();
    let prop = uid(&s, layer, "transform/opacity");
    loaded_close_keys(&mut s, layer, prop);
    s.execute_checked("prop.select", json!({"layer":layer,"prop":prop})).unwrap();
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("keys.move", json!({"delta":1.0/30.0,"merge":"move"})).unwrap();
    s.execute_checked("keys.move", json!({"delta":-0.75/30.0,"merge":"move"})).unwrap();
    let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(prop).unwrap().keys;
    assert_eq!(keys.iter().map(|key| key.value.as_f64()).collect::<Vec<_>>(), vec![0.0, 50.0]);
    assert_eq!(keys.iter().map(|key| key.time).collect::<Vec<_>>(), vec![Tick::ZERO, FrameRate::FPS_30.tick_of(1)]);
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}

#[test]
fn snapped_collision_keeps_the_winners_position_among_other_selected_properties() {
    let (mut s, a) = setup();
    let ap = uid(&s, a, "transform/opacity");
    loaded_close_keys(&mut s, a, ap);
    let b = s.execute_checked("layer.newSolid", json!({"name":"B","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    add(&mut s, b, "transform/opacity", 0.0, json!(20));
    let bp = uid(&s, b, "transform/opacity");
    select(&mut s, json!([{"layer":a,"prop":ap,"time":0},{"layer":b,"prop":bp,"time":0},{"layer":a,"prop":ap,"time":0.01}]));
    s.history.clear();
    let result = s.execute_checked("keys.move", json!({"delta":1.0/30.0})).unwrap();
    assert_eq!(result, json!(2));
    assert_eq!(
        s.state.selected_keys,
        vec![
            KeyRef { layer: LayerId(b), prop: bp, time: FrameRate::FPS_30.tick_of(1) },
            KeyRef { layer: LayerId(a), prop: ap, time: FrameRate::FPS_30.tick_of(1) },
        ],
        "the collision drops the displaced source identity without moving the winner ahead of the unrelated property"
    );
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(s.active_comp().unwrap().layer(LayerId(a)).unwrap().props.find(ap).unwrap().keys[0].value, Value::Scalar(50.0));
}
