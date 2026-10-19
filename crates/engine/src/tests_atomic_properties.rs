//! Position writes with separated dimensions are one atomic property edit.
use crate::Session;
use serde_json::json;
use std::sync::Arc;

fn setup(three_d: bool) -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 64, "height": 64, "duration": 4, "frameRate": 30})).unwrap();
    s.execute("layer.newSolid", json!({"name": "$1", "color": "#fff"})).unwrap();
    if three_d {
        s.execute("layer.setSwitch", json!({"layer": "$1", "switch": "threeD", "value": true})).unwrap();
    }
    s.execute("prop.separateDimensions", json!({"layer": "$1"})).unwrap();
    s.history.clear();
    s
}

#[test]
fn separated_dimensions_invalid_later_axis_leaves_project_and_history_unchanged() {
    for (three_d, value) in [(false, json!([10, "bad"])), (true, json!([10, 20, "bad"]))] {
        let mut s = setup(three_d);
        s.execute("prop.select", json!({"layer": "$1", "path": "transform/positionX"})).unwrap();
        let before = s.project.clone();
        let state = serde_json::to_value(&s.state).unwrap();
        let result = s.execute_checked("prop.set", json!({"layer": "$1", "path": "transform/position", "value": value}));
        assert!(result.is_err());
        assert!(s.project.same_content(&before), "an invalid later axis must not leave an earlier axis changed ({three_d})");
        assert!(Arc::ptr_eq(&s.project, &before));
        assert!(s.history.undo.is_empty());
        assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    }
}

#[test]
fn separated_dimensions_successful_position_write_is_one_undo_for_2d_and_3d() {
    for three_d in [false, true] {
        let mut s = setup(three_d);
        let before = s.project.clone();
        let value = if three_d { json!([10, 20, 30]) } else { json!([10, 20]) };
        let result = s.execute_checked("prop.set", json!({"layer": "$1", "path": "transform/position", "value": value})).unwrap();
        assert_eq!(result, if three_d { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0]) });
        let after = s.project.clone();
        assert_eq!(s.history.undo.len(), 1, "one Position write is one undo step ({three_d})");
        assert!(s.undo());
        assert!(Arc::ptr_eq(&s.project, &before));
        assert!(!s.undo());
        assert!(s.redo());
        assert!(Arc::ptr_eq(&s.project, &after));
    }
}

#[test]
fn separated_dimensions_literal_layer_name_and_merged_drag_keep_one_undo() {
    let mut s = setup(false);
    let before = s.project.clone();
    for value in [json!([10, 20]), json!([30, 40])] {
        s.execute_checked("prop.set", json!({"layer": "$1", "path": "transform/position", "value": value, "merge": "position-drag"})).unwrap();
    }
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(s.redo());
    let x = s.execute("prop.get", json!({"layer": "$1", "path": "transform/positionX"})).unwrap();
    let y = s.execute("prop.get", json!({"layer": "$1", "path": "transform/positionY"})).unwrap();
    assert_eq!((x["value"].clone(), y["value"].clone()), (json!(30.0), json!(40.0)));
}

#[test]
fn separated_dimensions_writes_preserve_comp_timing_metadata_and_single_undo() {
    let mut s = setup(false);
    s.execute_checked("layer.timing", json!({"layer": "$1", "start": 1, "in": 1, "out": 4})).unwrap();
    s.execute_checked("layer.timeStretch", json!({"layer": "$1", "percent": 200, "hold": "in"})).unwrap();
    for path in ["transform/positionX", "transform/positionY"] {
        s.execute("prop.addKey", json!({"layer": "$1", "path": path, "timeBase": "comp", "time": 1, "value": 5})).unwrap();
    }
    s.history.clear();
    let before = s.project.clone();
    let result = s.execute_checked("prop.set", json!({"layer": "$1", "path": "transform/position", "timeBase": "comp", "time": 3, "value": [30, 40]})).unwrap();
    let axes = result.as_array().unwrap();
    assert_eq!(axes.len(), 2);
    for (axis, value) in axes.iter().zip([30.0, 40.0]) {
        assert_eq!(axis["value"], value);
        assert_eq!(axis["animated"], true);
        assert_eq!(axis["layerTime"], 1.0);
        assert_eq!(axis["compTime"], 3.0);
    }
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
}

#[test]
fn atomic_batch_project_replacement_failure_preserves_path_saved_state_and_history() {
    for command in ["file.newProject", "file.openDemoProject"] {
        let mut s = Session::default();
        s.execute("comp.new", json!({"name": "Original", "width": 64, "height": 64, "duration": 4})).unwrap();
        s.execute("layer.newSolid", json!({"name": "Box", "color": "#fff"})).unwrap();
        s.path = Some("original.ecproj".into());
        s.mark_saved();
        assert!(!s.is_dirty());
        let project = s.project.clone();
        let history = s.history.clone();
        let state = serde_json::to_value(&s.state).unwrap();
        let result = s.execute_checked(
            "engine.batch",
            json!({"atomic": true, "steps": [
                {"command": command},
                {"command": "does.not.exist"}
            ]}),
        );
        assert!(result.is_err());
        assert!(Arc::ptr_eq(&s.project, &project), "{command}: current project snapshot");
        assert_eq!(s.path.as_deref(), Some("original.ecproj"), "{command}: saved path");
        assert!(!s.is_dirty(), "{command}: saved project remains saved after failed atomic batch");
        assert_eq!(serde_json::to_value(&s.state).unwrap(), state, "{command}: editor state");
        assert_eq!(s.history.undo.len(), history.undo.len(), "{command}: undo count");
        assert_eq!(s.history.redo.len(), history.redo.len(), "{command}: redo count");
        assert_eq!(s.history.branches.len(), history.branches.len(), "{command}: branch count");
        assert_eq!(s.history.merge_key, history.merge_key, "{command}: merge key");
        for ((actual_label, actual), (label, before)) in s.history.undo.iter().zip(&history.undo) {
            assert_eq!(actual_label, label);
            assert!(Arc::ptr_eq(actual, before), "{command}: original undo snapshot");
        }
    }
}
