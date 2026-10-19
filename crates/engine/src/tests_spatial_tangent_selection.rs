//! Spatial handle edits keep selected roving keys on their stored identities.
use crate::{
    KeyRef, Session,
    project::LayerId,
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::sync::Arc;

fn setup(roving: bool) -> (Session, u64, u64) {
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":4})).unwrap();
    let layer = s.execute_checked("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    for (time, x) in [(0, 0), (1, 25), (2, 100)] {
        s.execute_checked("prop.addKey", json!({"layer":layer,"path":"transform/position","time":time,"value":[x,0]})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/position").unwrap().uid;
    if roving {
        s.execute_checked("keys.select", json!({"keys":[{"layer":layer,"prop":uid,"time":1}]})).unwrap();
        s.execute_checked("keys.interpolation", json!({"roving":true})).unwrap();
        assert_eq!(middle(&s, layer, uid), FrameRate::FPS_30.tick_of(15));
    }
    (s, layer, uid)
}
fn middle(s: &Session, layer: u64, uid: u64) -> Tick {
    let pr = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(uid).unwrap();
    let key = pr.keys.iter().find(|k| k.value.as_vec2() == [25.0, 0.0]).unwrap();
    key.time
}
fn bend(s: &mut Session, layer: u64, uid: u64) {
    s.execute_checked("keys.setSpatialTangents", json!({"layer":layer,"prop":uid,"time":0,"out":[0,100],"break":true})).unwrap();
}
#[test]
fn spatial_tangent_edit_keeps_the_selected_roving_middle_key() {
    let (mut s, layer, uid) = setup(true);
    let before = s.state.selected_keys.clone();
    s.history.clear();
    bend(&mut s, layer, uid);
    let time = middle(&s, layer, uid);
    assert_ne!(time, before[0].time, "the longer first path segment actually retimes the middle key");
    assert_eq!(
        s.state.selected_keys,
        vec![KeyRef { layer: LayerId(layer), prop: uid, time }],
        "selection follows the middle key after the spatial handle edit"
    );
    assert_eq!(s.history.undo.len(), 1);
}
#[test]
fn spatial_tangent_edit_keeps_interleaved_selection_order_and_one_undo() {
    let (mut s, layer, uid) = setup(true);
    let other = s.execute_checked("layer.newSolid", json!({"name":"Other","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    s.execute_checked("prop.addKey", json!({"layer":other,"path":"transform/opacity","time":0,"value":20})).unwrap();
    let opacity = s.active_comp().unwrap().layer(LayerId(other)).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute_checked(
        "keys.select",
        json!({"keys":[
            {"layer":layer,"prop":uid,"time":2},{"layer":other,"prop":opacity,"time":0},
            {"layer":layer,"prop":uid,"time":0.5},{"layer":layer,"prop":uid,"time":0}
        ]}),
    )
    .unwrap();
    let order = s.state.selected_keys.clone();
    let before = s.project.clone();
    s.history.clear();
    bend(&mut s, layer, uid);
    let mut expected = order;
    expected[2].time = middle(&s, layer, uid);
    assert_ne!(expected[2].time, FrameRate::FPS_30.tick_of(15));
    assert_eq!(s.state.selected_keys, expected, "unrelated property and endpoint selection positions remain unchanged");
    assert_eq!(s.history.undo.len(), 1);
    s.execute_checked("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.project, &before), "one undo restores the curve and roving times");
}
#[test]
fn non_roving_tangent_edit_keeps_key_times_and_selection_unchanged() {
    let (mut s, layer, uid) = setup(false);
    s.execute_checked("keys.select", json!({"keys":[{"layer":layer,"prop":uid,"time":1},{"layer":layer,"prop":uid,"time":0}]})).unwrap();
    let order = s.state.selected_keys.clone();
    let before = s.project.clone();
    s.history.clear();
    bend(&mut s, layer, uid);
    assert_eq!(middle(&s, layer, uid), Tick::from_seconds_f64(1.0));
    assert_eq!(s.state.selected_keys, order);
    assert_eq!(s.history.undo.len(), 1);
    s.execute_checked("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.project, &before));
    assert_eq!(s.state.selected_keys, order);
}

#[test]
fn spatial_tangent_nearest_key_handles_full_tick_range() {
    for (times, target, expected) in [([i64::MIN, -1], 0.0, 1), ([1, i64::MAX], -1e100, 0), ([i64::MIN, -1], 1e100, 1), ([-10, 10], 0.0, 0), ([0, 10], 0.0, 0)]
    {
        let (mut s, layer, uid) = setup(false);
        let cid = s.active_comp_id().unwrap();
        let pr = Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(layer)).unwrap().props.find_mut(uid).unwrap();
        pr.keys.truncate(2);
        for (key, time) in pr.keys.iter_mut().zip(times) {
            key.time = Tick(time);
            key.spatial_auto = false;
            key.spatial_in = [0.0; 3];
            key.spatial_out = [0.0; 3];
        }
        let before = s.project.clone();
        s.history.clear();
        s.execute_checked("keys.setSpatialTangents", json!({"layer":layer,"prop":uid,"time":target,"out":[0,100,0],"break":true})).unwrap();
        let keys = &s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find(uid).unwrap().keys;
        assert_eq!(keys[expected].spatial_out, [0.0, 100.0, 0.0], "nearest key for {times:?} at {target}");
        assert_eq!(keys[1 - expected].spatial_out, [0.0; 3], "the other key is unchanged");
        assert_eq!(keys.iter().map(|key| key.time.0).collect::<Vec<_>>(), times);
        assert_eq!(s.history.undo.len(), 1);
        assert!(s.undo());
        assert!(Arc::ptr_eq(&s.project, &before), "one undo restores the hostile-time project");
    }
}
