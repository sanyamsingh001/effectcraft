use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use effectcraft_engine::config::{ConfigStore, MemoryConfig};
use effectcraft_engine::prefs::PREFS_FILE;
use effectcraft_engine::shortcuts::SHORTCUTS_FILE;
use effectcraft_engine::{Services, Session};
use serde_json::{Value, json};

#[derive(Default)]
struct CountingConfig {
    memory: MemoryConfig,
    writes: AtomicUsize,
}

impl ConfigStore for CountingConfig {
    fn read(&self, name: &str) -> Option<String> {
        self.memory.read(name)
    }
    fn write(&self, name: &str, data: &str) -> std::io::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.memory.write(name, data)
    }
    fn remove(&self, name: &str) -> std::io::Result<()> {
        self.memory.remove(name)
    }
}

struct SyntheticFiles;

impl Services for SyntheticFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        match path {
            "synthetic-data.json" => Ok(br#"{"rows":[1,2,3]}"#.to_vec()),
            "synthetic-preset.ffx" => Ok(br#"{"effectcraftAnimationPreset":1,"properties":[],"effects":[]}"#.to_vec()),
            _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unknown synthetic file")),
        }
    }
    fn write_file(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "synthetic files are read-only"))
    }
}

fn execute(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute_checked(id, params).unwrap()
}

fn session() -> (Session, Arc<CountingConfig>) {
    let mut s = Session::default();
    s.prefs.disk.disk_cache_enabled = false;
    s.services = Arc::new(SyntheticFiles);
    execute(&mut s, "comp.new", json!({"width":64,"height":64,"duration":3}));
    execute(&mut s, "layer.newSolid", json!({"name":"Box","color":"#f00"}));
    execute(&mut s, "layer.newText", json!({"text":"Before","name":"Caption"}));
    let store = Arc::new(CountingConfig::default());
    s.config = Some(store.clone());
    s.prefs.recent_projects.push("synthetic-project.ec".into());
    s.save_prefs();
    execute(&mut s, "shortcuts.set", json!({"command":"edit.undo","keys":"Ctrl+Alt+U"}));
    store.writes.store(0, Ordering::SeqCst);
    s.history.clear();
    (s, store)
}

fn invalid_step() -> Value {
    json!({"command":"prop.set","params":{"layer":"Box","path":"missing","value":10}})
}

fn shortcut_params(s: &mut Session, command: &str) -> Value {
    match command {
        "shortcuts.set" => json!({"command":"edit.undo","keys":"Ctrl+Shift+U"}),
        "shortcuts.reset" => json!({"command":"edit.undo"}),
        "shortcuts.preset" => json!({"op":"new","name":"Atomic preset"}),
        "shortcuts.import" => {
            let doc = execute(s, "shortcuts.export", json!({}));
            json!({"preset":doc,"name":"Atomic imported"})
        }
        _ => unreachable!(),
    }
}

fn atomic_shortcut_rejected(command: &str) {
    let (mut s, store) = session();
    let params = shortcut_params(&mut s, command);
    let original = s.keymaps.to_json();
    let stored = store.read(SHORTCUTS_FILE);
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
    assert_eq!(
        (s.keymaps.to_json(), store.read(SHORTCUTS_FILE), store.writes.load(Ordering::SeqCst)),
        (original, stored, 0),
        "atomic shortcut mutation must be rejected before session or persistent store changes"
    );
    assert_eq!(s.active_comp().unwrap().width, 64);
    assert!(s.history.undo.is_empty());
}

#[test]
fn atomic_shortcuts_set_cannot_escape_rollback() {
    atomic_shortcut_rejected("shortcuts.set");
}
#[test]
fn atomic_shortcuts_reset_cannot_escape_rollback() {
    atomic_shortcut_rejected("shortcuts.reset");
}
#[test]
fn atomic_shortcuts_preset_cannot_escape_rollback() {
    atomic_shortcut_rejected("shortcuts.preset");
}
#[test]
fn atomic_shortcuts_import_cannot_escape_rollback() {
    atomic_shortcut_rejected("shortcuts.import");
}

fn incidental_steps(s: &Session) -> Vec<Value> {
    vec![
        json!({"command":"view.extendedViewer","params":{"value":!s.prefs.three_d.extended_viewer}}),
        json!({"command":"layer.setText","params":{"layer":"Caption","text":"After","font":"Synthetic Test Font"}}),
        json!({"command":"anim.applyPreset","params":{"layer":"Caption","path":"synthetic-preset.ffx"}}),
        json!({"command":"file.import","params":{"paths":["synthetic-data.json"]}}),
    ]
}

#[test]
fn failed_incidental_preferences_restore_session_revision_and_store() {
    let (mut s, store) = session();
    let mut steps = incidental_steps(&s);
    steps.push(invalid_step());
    let original = s.project.clone();
    let prefs = s.prefs.clone();
    let revision = s.prefs_revision;
    let stored = store.read(PREFS_FILE);
    let error = s.execute_checked("engine.batch", json!({"steps":steps})).unwrap_err();
    assert!(error.to_string().contains("step 5"), "{error}");
    assert!(Arc::ptr_eq(&s.project, &original));
    assert_eq!(
        (s.prefs.clone(), s.prefs_revision, store.read(PREFS_FILE), store.writes.load(Ordering::SeqCst)),
        (prefs, revision, stored, 0),
        "failed atomic batch must restore incidental preferences without persisting an intermediate state"
    );
    assert!(s.history.undo.is_empty());
}

#[test]
fn successful_incidental_preferences_flush_once_and_keep_one_project_undo() {
    let (mut s, store) = session();
    let steps = incidental_steps(&s);
    let original = s.project.clone();
    let result = execute(&mut s, "engine.batch", json!({"steps":steps}));
    assert_eq!(result["steps"], 4);
    assert_eq!(result["results"][3]["errors"], json!([]));
    assert_eq!(result["results"][3]["items"].as_array().unwrap().len(), 1);
    assert!(s.prefs.recent_fonts.iter().any(|font| font == "Synthetic Test Font"));
    assert!(s.prefs.recent_presets.iter().any(|path| path == "synthetic-preset.ffx"));
    assert!(s.prefs.recent_footage.iter().any(|path| path == "synthetic-data.json"));
    assert_eq!(store.read(PREFS_FILE), Some(s.prefs.to_json()));
    assert_eq!(store.writes.load(Ordering::SeqCst), 1, "successful atomic incidental preference edits must flush only the committed state once");
    assert_eq!(s.history.undo.len(), 1);
    let after = s.project.clone();
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &original));
    assert!(s.redo());
    assert!(Arc::ptr_eq(&s.project, &after));
}

#[test]
fn successful_atomic_preferences_with_no_net_change_do_not_flush() {
    let (mut s, store) = session();
    let value = s.prefs.three_d.extended_viewer;
    let stored = store.read(PREFS_FILE);
    execute(
        &mut s,
        "engine.batch",
        json!({"steps":[
            {"command":"view.extendedViewer","params":{"value":!value}},
            {"command":"view.extendedViewer","params":{"value":value}}
        ]}),
    );
    assert_eq!(s.prefs.three_d.extended_viewer, value);
    assert_eq!(store.read(PREFS_FILE), stored);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0, "an atomic preference transaction with no net change must not write");
}

#[test]
fn successful_atomic_multiple_preferences_flush_once() {
    let (mut s, store) = session();
    let value = s.prefs.three_d.extended_viewer;
    execute(
        &mut s,
        "engine.batch",
        json!({"steps":[
            {"command":"view.extendedViewer","params":{"value":!value}},
            {"command":"file.clearRecent","params":{}}
        ]}),
    );
    assert!(s.prefs.recent_projects.is_empty());
    assert_eq!(store.read(PREFS_FILE), Some(s.prefs.to_json()));
    assert_eq!(store.writes.load(Ordering::SeqCst), 1, "atomic preference changes must flush once");
}

#[test]
fn non_atomic_shortcut_mutations_remain_allowed_and_persist() {
    for command in ["shortcuts.set", "shortcuts.reset", "shortcuts.preset", "shortcuts.import"] {
        let (mut s, store) = session();
        let params = shortcut_params(&mut s, command);
        execute(&mut s, "engine.batch", json!({"atomic":false,"steps":[{"command":command,"params":params}]}));
        assert_eq!(store.read(SHORTCUTS_FILE), Some(s.keymaps.to_json()), "{command}");
        assert_eq!(store.writes.load(Ordering::SeqCst), 1, "{command}");
    }
}

#[test]
fn non_atomic_failed_batch_preserves_incidental_preference_prefix() {
    let (mut s, store) = session();
    let value = !s.prefs.three_d.extended_viewer;
    let error = s
        .execute_checked(
            "engine.batch",
            json!({"atomic":false,"steps":[
                {"command":"view.extendedViewer","params":{"value":value}},
                invalid_step()
            ]}),
        )
        .unwrap_err();
    assert!(error.to_string().contains("steps 1–1"), "{error}");
    assert_eq!(s.prefs.three_d.extended_viewer, value);
    assert_eq!(store.read(PREFS_FILE), Some(s.prefs.to_json()));
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
}

mod deferred {
    use serde_json::json;
    use std::sync::atomic::Ordering;

    use super::{execute, invalid_step, session};
    use effectcraft_engine::config::ConfigStore;
    use effectcraft_engine::prefs::PREFS_FILE;

    #[test]
    fn outer_deferred_preferences_stay_deferred_after_success() {
        let (mut s, store) = session();
        let stored = store.read(PREFS_FILE);
        s.defer_prefs_write = true;
        let value = !s.prefs.three_d.extended_viewer;
        execute(&mut s, "engine.batch", json!({"steps":[{"command":"view.extendedViewer","params":{"value":value}}]}));
        assert!(s.defer_prefs_write);
        assert_eq!(s.prefs.three_d.extended_viewer, value);
        assert_eq!(store.read(PREFS_FILE), stored);
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
        s.defer_prefs_write = false;
        s.save_prefs();
        assert_eq!(store.read(PREFS_FILE), Some(s.prefs.to_json()));
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn outer_deferred_preferences_stay_deferred_after_rollback() {
        let (mut s, store) = session();
        let stored = store.read(PREFS_FILE);
        let prefs = s.prefs.clone();
        let revision = s.prefs_revision;
        s.defer_prefs_write = true;
        let value = !s.prefs.three_d.extended_viewer;
        let error = s
            .execute_checked(
                "engine.batch",
                json!({"steps":[
                    {"command":"view.extendedViewer","params":{"value":value}}, invalid_step()
                ]}),
            )
            .unwrap_err();
        assert!(error.to_string().contains("rolled back"), "{error}");
        assert!(s.defer_prefs_write);
        assert_eq!(s.prefs, prefs);
        assert_eq!(s.prefs_revision, revision);
        assert_eq!(store.read(PREFS_FILE), stored);
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    }
}
