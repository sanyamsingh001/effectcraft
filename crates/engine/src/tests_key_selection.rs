//! Repeated key references must select or toggle each key only once.

use serde_json::json;

use crate::tests_timeline::{prop, setup};

#[test]
fn duplicate_key_selection_applies_hold_once_and_preserves_order() {
    let (mut s, layer) = setup();
    for time in [0, 1] {
        s.execute("prop.addKey", json!({"layer": layer, "path": "transform/opacity", "time": time, "value": 20})).unwrap();
    }
    let result = s
        .execute(
            "keys.select",
            json!({"keys": [
                {"layer": layer, "path": "transform/opacity", "time": 1},
                {"layer": layer, "path": "transform/opacity", "time": 1},
                {"layer": layer, "path": "transform/opacity", "time": 0},
                {"layer": layer, "path": "transform/opacity", "time": 0}
            ]}),
        )
        .unwrap();
    assert_eq!(result, json!(2));
    assert_eq!(s.state.selected_keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), vec![1.0, 0.0]);
    s.execute("keys.toggleHold", json!({})).unwrap();
    assert!(prop(&s, layer, "transform/opacity").keys.iter().all(|k| k.out_interp == effectcraft_keyframe::Interp::Hold));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(prop(&s, layer, "transform/opacity").keys.iter().all(|k| k.out_interp == effectcraft_keyframe::Interp::Linear));
}

#[test]
fn duplicate_add_and_toggle_entries_apply_once_without_reordering_other_keys() {
    let (mut s, layer) = setup();
    for time in [0, 1, 2] {
        s.execute("prop.addKey", json!({"layer": layer, "path": "transform/opacity", "time": time, "value": 20})).unwrap();
    }
    s.execute("keys.select", json!({"keys": [{"layer": layer, "path": "transform/opacity", "time": 2}]})).unwrap();
    let duplicate = json!([
        {"layer": layer, "path": "transform/opacity", "time": 0},
        {"layer": layer, "path": "transform/opacity", "time": 0}
    ]);
    s.execute("keys.select", json!({"keys": duplicate, "add": true})).unwrap();
    assert_eq!(s.state.selected_keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), vec![2.0, 0.0]);
    s.execute("keys.select", json!({"keys": duplicate, "toggle": true})).unwrap();
    assert_eq!(s.state.selected_keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), vec![2.0]);
    s.execute("keys.select", json!({"keys": duplicate, "toggle": true})).unwrap();
    assert_eq!(s.state.selected_keys.iter().map(|k| k.time.seconds()).collect::<Vec<_>>(), vec![2.0, 0.0]);
}
