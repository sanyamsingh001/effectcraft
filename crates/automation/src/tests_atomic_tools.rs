//! Keyframe tool failures must not leave partially applied edits.

use serde_json::{Value, json};

use crate::{
    Backend, Session,
    tools::{self, Reply},
};

fn setup() -> Backend {
    let mut s = Session::default();
    s.execute("comp.new", json!({"duration": 20, "frameRate": 30})).unwrap();
    s.execute("layer.newSolid", json!({"name": "A", "color": "#fff"})).unwrap();
    s.execute("layer.timing", json!({"layer": "A", "start": 10.5, "in": 10.5, "out": 20})).unwrap();
    Backend::headless(s)
}

fn add(b: &mut Backend, extra: Value) -> crate::Result<Reply> {
    let mut args = json!({"layer": "A", "path": "transform/opacity", "timeBase": "comp"});
    args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    tools::run(b, "add_keyframe", &args)
}

#[test]
fn invalid_later_keys_and_interpolation_leave_project_history_and_selection_unchanged() {
    let mut b = setup();
    for extra in [
        json!({"keys": [{"time": 15.3, "value": 10}, {"value": 20}]}),
        json!({"keys": [{"time": 15.3, "value": 10}, {"time": "bad", "value": 20}]}),
        json!({"keys": [{"time": 15.3, "value": 10}, {"time": 16, "value": "bad"}]}),
        json!({"keys": [{"time": 15.3, "value": 10}, {"time": 16, "value": 20}], "interpolation": "bogus"}),
        json!({"time": 15.3, "value": 10, "interpolation": 42}),
    ] {
        let s = b.session().unwrap();
        let before = s.project.clone();
        let history = s.history.undo.len();
        let state = serde_json::to_value(&s.state).unwrap();
        assert!(add(&mut b, extra.clone()).is_err(), "{extra}");
        let s = b.session().unwrap();
        assert!(s.project.same_content(&before), "{extra}");
        assert_eq!(s.history.undo.len(), history, "{extra}");
        assert_eq!(serde_json::to_value(&s.state).unwrap(), state, "{extra}");
    }
}

#[test]
fn successful_keys_with_interpolation_are_one_undo_and_keep_timing_metadata() {
    let mut b = setup();
    let before = b.session().unwrap().history.undo.len();
    let Reply::Json(info) = add(&mut b, json!({"keys": [{"time": 15.3, "value": 0}, {"time": 16, "value": 100}], "interpolation": "easyEase"})).unwrap() else {
        panic!("JSON reply")
    };
    assert_eq!(info["addedKeys"][0]["compTime"], 15.3);
    assert_eq!(info["addedKeys"][0]["layerTime"], 4.8);
    assert_eq!(info["keys"][0]["out"], "Bezier");
    assert_eq!(b.session().unwrap().history.undo.len(), before + 1);
    b.exec("edit.undo", json!({})).unwrap();
    let p = b.exec("prop.get", json!({"layer": "A", "path": "transform/opacity"})).unwrap();
    assert!(p["keys"].as_array().unwrap().is_empty());
    b.exec("edit.redo", json!({})).unwrap();
    let p = b.exec("prop.get", json!({"layer": "A", "path": "transform/opacity"})).unwrap();
    assert_eq!(p["keys"].as_array().unwrap().len(), 2);
    assert_eq!(p["keys"][0]["out"], "Bezier");
}

#[test]
fn literal_batch_arguments_preserve_dollar_names_and_text_values() {
    let mut b = setup();
    b.exec("layer.newText", json!({"name": "$1", "text": "original"})).unwrap();
    let Reply::Json(info) =
        tools::run(&mut b, "add_keyframe", &json!({"layer": "$1", "path": "text/sourceText", "time": 0, "value": "$1.layer", "interpolation": "hold"}))
            .unwrap()
    else {
        panic!("JSON reply")
    };
    assert_eq!(info["keys"][0]["value"], "$1.layer");
}

#[test]
fn key_times_colliding_on_a_comp_frame_select_and_toggle_hold_once() {
    let mut b = setup();
    let Reply::Json(info) = add(&mut b, json!({"keys": [{"time": 15.3, "value": 10}, {"time": 15.301, "value": 20}], "interpolation": "linear"})).unwrap()
    else {
        panic!("JSON reply")
    };
    assert_eq!(info["keys"].as_array().unwrap().len(), 1);
    assert_eq!(info["addedKeys"][0]["compTime"], info["addedKeys"][1]["compTime"]);
    assert_eq!(b.session().unwrap().state.selected_keys.len(), 1);
    b.exec("keys.toggleHold", json!({})).unwrap();
    let p = b.exec("prop.get", json!({"layer": "A", "path": "transform/opacity"})).unwrap();
    assert_eq!(p["keys"][0]["out"], "Hold");
    assert_eq!(p["keys"][0]["value"], 20.0);
}

#[test]
fn large_keyframe_tool_call_with_interpolation_is_one_bounded_undo() {
    let mut b = setup();
    b.exec("prefs.set", json!({"key": "general.undoLevels", "value": 2})).unwrap();
    let before = b.session().unwrap().project.clone();
    let keys: Vec<Value> = (0..40).map(|i| json!({"time": 10.5 + i as f64 / 10.0, "value": i})).collect();
    let Reply::Json(info) = add(&mut b, json!({"keys": keys, "interpolation": "easyEase"})).unwrap() else { panic!("JSON reply") };
    assert_eq!(info["keys"].as_array().unwrap().len(), 40);
    assert_eq!(info["addedKeys"].as_array().unwrap().len(), 40);
    assert_eq!(info["addedKeys"][0]["compTime"], 10.5);
    assert_eq!(info["addedKeys"][0]["layerTime"], 0.0);
    assert!(info["keys"].as_array().unwrap().iter().all(|key| key["out"] == "Bezier"));
    assert_eq!(b.session().unwrap().history.undo.len(), 2);
    let after = b.session().unwrap().project.clone();
    b.exec("edit.undo", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&b.session().unwrap().project, &before), "one undo removes all 40 keys and interpolation");
    b.exec("edit.redo", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&b.session().unwrap().project, &after));
}
