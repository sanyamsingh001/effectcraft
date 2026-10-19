use std::sync::Arc;

use effectcraft_engine::config::{ConfigStore, MemoryConfig};
use effectcraft_engine::prefs::PREFS_FILE;
use effectcraft_engine::{Session, project::LayerId};
use serde_json::{Value, json};

fn execute(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute_checked(id, params).unwrap()
}

fn session() -> (Session, LayerId) {
    let mut s = Session::default();
    execute(&mut s, "comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":3}));
    let layer = LayerId(execute(&mut s, "layer.newSolid", json!({"name":"Box","color":"#f00"}))["layer"].as_u64().unwrap());
    for (time, value) in [(0, 0), (1, 100)] {
        execute(&mut s, "prop.addKey", json!({"layer":layer.0,"path":"transform/opacity","time":time,"value":value}));
    }
    execute(&mut s, "prop.select", json!({"layer":layer.0,"path":"transform/opacity","selectKeys":true}));
    s.history.clear();
    (s, layer)
}

fn times(s: &Session, layer: LayerId) -> Vec<i64> {
    let comp = s.active_comp().unwrap();
    comp.layer(layer).unwrap().props.prop("transform/opacity").unwrap().keys.iter().map(|key| comp.frame_rate.frame_at(key.time)).collect()
}

fn invalid_step() -> Value {
    json!({"command":"prop.set","params":{"layer":"Box","path":"missing","value":10}})
}

fn atomic_preference_rejected(command: &str, params: Value) {
    let (mut s, _) = session();
    let store = Arc::new(MemoryConfig::default());
    s.prefs.disk.disk_cache_enabled = false;
    s.config = Some(store.clone());
    execute(&mut s, "prefs.set", json!({"key":"general.undoLevels","value":7}));
    s.history.clear();
    let saved = store.read(PREFS_FILE).unwrap();
    let before = s.project.clone();
    let before_prefs = s.prefs.clone();
    let error = s
        .execute_checked(
            "engine.batch",
            json!({"steps":[
                {"command":"comp.settings","params":{"width":65}},
                {"command":command,"params":params},
                invalid_step()
            ]}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("step"), "{error}");
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(s.history.undo.is_empty());
    assert_eq!(
        (s.prefs, store.read(PREFS_FILE)),
        (before_prefs, Some(saved)),
        "atomic batch must reject irreversible preference writes before changing session or store"
    );
}

#[test]
fn atomic_prefs_set_cannot_escape_rollback_or_persist() {
    atomic_preference_rejected("prefs.set", json!({"key":"general.undoLevels","value":1}));
}

#[test]
fn atomic_prefs_reset_cannot_escape_rollback_or_persist() {
    atomic_preference_rejected("prefs.reset", json!({"page":"general"}));
}

#[test]
fn failed_atomic_batch_restores_active_from_start_gesture() {
    let (mut s, layer) = session();
    let original = s.project.clone();
    execute(&mut s, "keys.transform", json!({"timeOffset":0.1,"fromStart":true,"merge":"drag"}));
    assert_eq!(times(&s, layer), [3, 33]);
    let before = s.project.clone();
    let state = serde_json::to_value(&s.state).unwrap();
    let error = s
        .execute_checked(
            "engine.batch",
            json!({"steps":[
                {"command":"keys.transform","params":{"timeOffset":0.1,"fromStart":true,"merge":"other"}},
                invalid_step()
            ]}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("step 2"), "{error}");
    assert!(Arc::ptr_eq(&s.project, &before));
    assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    execute(&mut s, "keys.transform", json!({"timeOffset":0.2,"fromStart":true,"merge":"drag"}));
    assert_eq!(times(&s, layer), [6, 36], "atomic rollback must restore the original fromStart gesture cache");
    assert_eq!(s.history.undo.len(), 1, "the resumed drag remains one undo step");
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &original));
    assert!(s.redo());
    assert_eq!(times(&s, layer), [6, 36]);
}

#[test]
fn failed_atomic_batch_does_not_leave_a_new_gesture_cache() {
    let (mut s, layer) = session();
    assert!(s.key_move.is_none());
    let error = s
        .execute_checked(
            "engine.batch",
            json!({"steps":[
                {"command":"keys.transform","params":{"timeOffset":0.1,"fromStart":true,"merge":"other"}},
                invalid_step()
            ]}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("step 2"), "{error}");
    assert_eq!(times(&s, layer), [0, 30]);
    assert!(s.key_move.is_none(), "atomic rollback must remove a gesture cache created by the failed batch");
}

#[test]
fn non_atomic_preference_set_and_reset_still_persist() {
    let (mut s, _) = session();
    let store = Arc::new(MemoryConfig::default());
    s.prefs.disk.disk_cache_enabled = false;
    s.config = Some(store.clone());
    execute(&mut s, "engine.batch", json!({"atomic":false,"steps":[{"command":"prefs.set","params":{"key":"general.undoLevels","value":7}}]}));
    assert_eq!(s.prefs.general.undo_levels, 7);
    let persisted: Value = serde_json::from_str(&store.read(PREFS_FILE).unwrap()).unwrap();
    assert_eq!(persisted["general"]["undoLevels"], 7);
    execute(&mut s, "engine.batch", json!({"atomic":false,"steps":[{"command":"prefs.reset","params":{"page":"general"}}]}));
    assert_eq!(s.prefs.general.undo_levels, 32);
    let persisted: Value = serde_json::from_str(&store.read(PREFS_FILE).unwrap()).unwrap();
    assert_eq!(persisted["general"]["undoLevels"], 32);
}

#[test]
fn ordinary_from_start_gesture_and_undo_redo_are_unchanged() {
    let (mut s, layer) = session();
    let original = s.project.clone();
    for offset in [0.1, 0.2] {
        execute(&mut s, "keys.transform", json!({"timeOffset":offset,"fromStart":true,"merge":"drag"}));
    }
    assert_eq!(times(&s, layer), [6, 36]);
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &original));
    assert!(s.redo());
    assert_eq!(times(&s, layer), [6, 36]);
}
