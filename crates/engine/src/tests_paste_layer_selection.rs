//! Clipboard and layer references preserve ordered selection identities.
use crate::{
    KeyRef, Session,
    project::LayerId,
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

fn session() -> Session {
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":10})).unwrap();
    s
}
fn solid(s: &mut Session, name: &str) -> u64 {
    s.execute_checked("layer.newSolid", json!({"name":name,"color":"#fff"})).unwrap()["layer"].as_u64().unwrap()
}
fn position(s: &mut Session, layer: u64, keys: &[(f64, f64)]) -> u64 {
    for (time, x) in keys {
        s.execute_checked("prop.addKey", json!({"layer":layer,"path":"transform/position","time":time,"value":[x,0]})).unwrap();
    }
    s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/position").unwrap().uid
}
fn middle(s: &Session, layer: u64, uid: u64, x: f64) -> Tick {
    s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(uid).unwrap().keys.iter().find(|k| k.value.as_vec2() == [x, 0.0]).unwrap().time
}
fn roving_clip(s: &mut Session) -> u64 {
    let source = solid(s, "Source");
    let uid = position(s, source, &[(0.0, 0.0), (1.0, 25.0), (2.0, 100.0)]);
    s.execute_checked("keys.select", json!({"keys":[{"layer":source,"prop":uid,"time":1}]})).unwrap();
    s.execute_checked("keys.interpolation", json!({"roving":true})).unwrap();
    assert_eq!(middle(s, source, uid, 25.0), FrameRate::FPS_30.tick_of(15));
    s.execute_checked("keys.copy", json!({})).unwrap();
    source
}
#[test]
fn pasted_roving_key_selection_follows_its_actual_time() {
    let mut s = session();
    roving_clip(&mut s);
    let target = solid(&mut s, "Target");
    let uid = position(&mut s, target, &[(0.0, 0.0), (2.0, 50.0)]);
    s.execute_checked("keys.paste", json!({"layers":[target],"path":"transform/position","time":0.5})).unwrap();
    let time = middle(&s, target, uid, 25.0);
    assert_ne!(time, FrameRate::FPS_30.tick_of(15));
    assert_eq!(time, FrameRate::FPS_30.tick_of(30));
    assert_eq!(s.state.selected_keys, vec![KeyRef { layer: LayerId(target), prop: uid, time }], "pasted selection follows the roving stored key");
}
#[test]
fn pasted_roving_keys_keep_target_order_and_one_undo() {
    let mut s = session();
    roving_clip(&mut s);
    let first = solid(&mut s, "First");
    let first_uid = position(&mut s, first, &[(0.0, 0.0), (2.0, 50.0)]);
    let second = solid(&mut s, "Second");
    let second_uid = position(&mut s, second, &[(0.0, 0.0), (4.0, 100.0)]);
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("keys.paste", json!({"layers":[second,first],"path":"transform/position","time":0.5})).unwrap();
    let a = middle(&s, first, first_uid, 25.0);
    let b = middle(&s, second, second_uid, 25.0);
    assert_eq!(
        s.state.selected_keys,
        vec![KeyRef { layer: LayerId(second), prop: second_uid, time: b }, KeyRef { layer: LayerId(first), prop: first_uid, time: a }],
        "clipboard target order survives roving"
    );
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn pasted_duplicate_targets_select_the_stored_key_once() {
    let mut s = session();
    roving_clip(&mut s);
    let target = solid(&mut s, "Target");
    let uid = position(&mut s, target, &[(0.0, 0.0), (2.0, 50.0)]);
    s.execute_checked("keys.paste", json!({"layers":[target,target],"path":"transform/position","time":0.5})).unwrap();
    let time = middle(&s, target, uid, 25.0);
    assert_eq!(s.state.selected_keys, vec![KeyRef { layer: LayerId(target), prop: uid, time }], "duplicate targets do not add duplicate selections");
}
#[test]
fn scalar_paste_keeps_exact_frame_time_and_grouped_undo() {
    let mut s = session();
    let source = solid(&mut s, "Source");
    let target = solid(&mut s, "Target");
    s.execute_checked("prop.addKey", json!({"layer":source,"path":"transform/opacity","time":1,"value":25})).unwrap();
    let source_uid = s.active_comp().unwrap().layer(LayerId(source)).unwrap().props.prop("transform/opacity").unwrap().uid;
    let uid = s.active_comp().unwrap().layer(LayerId(target)).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute_checked("keys.select", json!({"keys":[{"layer":source,"prop":source_uid,"time":1}]})).unwrap();
    s.execute_checked("keys.copy", json!({})).unwrap();
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("keys.paste", json!({"layers":[target],"path":"transform/opacity","time":0.5})).unwrap();
    assert_eq!(s.state.selected_keys, vec![KeyRef { layer: LayerId(target), prop: uid, time: FrameRate::FPS_30.tick_of(15) }]);
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
fn layer_setup() -> (Session, u64, u64, u64) {
    let mut s = session();
    let a = solid(&mut s, "A");
    let b = solid(&mut s, "B");
    let c = solid(&mut s, "C");
    (s, a, b, c)
}
#[test]
fn layer_selection_removes_duplicates_in_first_reference_order() {
    let (mut s, a, b, c) = layer_setup();
    let result = s.execute_checked("layer.select", json!({"layers":[b,a,b,c,a]})).unwrap();
    assert_eq!(result, json!([b, a, c]), "replacement selection is an ordered set");
    assert_eq!(s.state.selected_layers, vec![LayerId(b), LayerId(a), LayerId(c)]);
}
#[test]
fn layer_selection_add_and_toggle_apply_each_reference_once() {
    let (mut s, a, b, c) = layer_setup();
    s.execute_checked("layer.select", json!({"layers":[b]})).unwrap();
    s.execute_checked("layer.select", json!({"layers":[b,a,a],"add":true})).unwrap();
    assert_eq!(s.state.selected_layers, vec![LayerId(b), LayerId(a)]);
    s.execute_checked("layer.select", json!({"layers":[b,b,c,c],"toggle":true})).unwrap();
    assert_eq!(s.state.selected_layers, vec![LayerId(a), LayerId(c)], "duplicate toggle references apply once");
}
fn starts(s: &Session, ids: &[u64]) -> Vec<Tick> {
    ids.iter().map(|id| s.active_comp().unwrap().layer(LayerId(*id)).unwrap().start_time).collect()
}
#[test]
fn explicit_stagger_layers_deduplicate_references_before_assigning_ranks() {
    let (mut s, a, b, _) = layer_setup();
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("layer.timing", json!({"layers":[b,b,a],"stagger":0.1})).unwrap();
    assert_eq!(starts(&s, &[b, a]), vec![Tick::ZERO, FrameRate::FPS_30.tick_of(3)], "the first unique layer stays anchored");
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}
#[test]
fn selected_stagger_layers_keep_unique_first_reference_order() {
    let (mut s, a, b, _) = layer_setup();
    s.execute_checked("layer.select", json!({"layers":[b,b,a]})).unwrap();
    s.execute_checked("layer.timing", json!({"stagger":0.1})).unwrap();
    assert_eq!(starts(&s, &[b, a]), vec![Tick::ZERO, FrameRate::FPS_30.tick_of(3)], "selection duplicates cannot change stagger ranks");
}
#[test]
fn explicit_duplicate_delta_keeps_ordinary_timing_behavior() {
    let (mut s, a, b, _) = layer_setup();
    s.execute_checked("layer.timing", json!({"layers":[b,b,a],"delta":1})).unwrap();
    assert_eq!(starts(&s, &[b, a]), vec![FrameRate::FPS_30.tick_of(30); 2]);
}
#[test]
fn invalid_explicit_layer_reference_fails_before_any_timing_change() {
    let (mut s, a, _, _) = layer_setup();
    let before = s.project.clone();
    s.history.clear();
    assert!(s.execute_checked("layer.timing", json!({"layers":[a,"Missing",a],"stagger":0.1})).is_err());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(s.history.undo.is_empty());
}
