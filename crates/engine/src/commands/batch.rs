//! `engine.batch`: run several commands in one call, as one undo step, with later steps able to
//! use what earlier ones returned (`"$1.layer"` is step 1's `layer` result). A step with
//! `substitute: false` keeps its params literal, including strings beginning with `$`.
//! Agents build a
//! layer, animate it and style it in one round trip; a failing step rolls everything back.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session};

/// Commands that can't run inside a batch (they replace the project or nest batches).
const NOT_IN_BATCH: &[&str] = &[
    "engine.batch",
    "file.runScript",
    "script.run",
    "file.newProject",
    "file.openDemoProject",
    "file.open",
    "file.openRecent",
    "file.revert",
    "file.closeProject",
];

/// Saving commits a file and replaces the saved checkpoint. Explicit preference and shortcut
/// commands update settings or configuration immediately; rollback cannot undo those side effects.
const NOT_IN_ATOMIC_BATCH: &[&str] = &[
    "file.save",
    "file.saveAs",
    "file.incrementAndSave",
    "file.saveCopy",
    "prefs.set",
    "prefs.reset",
    "shortcuts.set",
    "shortcuts.reset",
    "shortcuts.preset",
    "shortcuts.import",
];

/// Resolve a `$N` / `$N.key.0.key` reference against the results so far.
fn reference(s: &str, results: &[Value]) -> Option<Result<Value>> {
    let rest = s.strip_prefix('$')?;
    let mut parts = rest.split('.');
    let n: usize = parts.next()?.parse().ok()?;
    let path: Vec<&str> = parts.collect();
    if path.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        return None;
    }
    let Some(mut v) = n.checked_sub(1).and_then(|i| results.get(i)) else {
        return Some(Err(bad("engine.batch", format!("`{s}` refers to step {n}, but only {} step(s) ran before", results.len()))));
    };
    for p in &path {
        let next = match (v, p.parse::<usize>()) {
            (Value::Array(a), Ok(i)) => a.get(i),
            (Value::Object(o), _) => o.get(*p),
            _ => None,
        };
        match next {
            Some(x) => v = x,
            None => return Some(Err(bad("engine.batch", format!("`{s}`: step {n} returned {v}, which has no `{p}`")))),
        }
    }
    Some(Ok(v.clone()))
}

/// Replace every `$N…` string in `v` (recursively) with the referenced result.
fn substitute(v: &Value, results: &[Value]) -> Result<Value> {
    Ok(match v {
        Value::String(s) => match reference(s, results) {
            Some(r) => r?,
            None => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, results)).collect::<Result<_>>()?),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| Ok((k.clone(), substitute(x, results)?))).collect::<Result<_>>()?),
        _ => v.clone(),
    })
}

fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let steps = p.get("steps").and_then(Value::as_array).ok_or_else(|| bad("engine.batch", "missing `steps`: [{command, params?}, …]"))?.clone();
    let label = str_p(p, "label").unwrap_or("Batch").to_string();
    let atomic = p.get("atomic").and_then(Value::as_bool).unwrap_or(true);
    let (project, history, state, key_move) = (s.project.clone(), s.history.clone(), s.state.clone(), s.key_move.clone());
    let (prefs, prefs_revision, defer_prefs_write) = (s.prefs.clone(), s.prefs_revision, s.defer_prefs_write);
    let events = s.events.clone();
    if atomic {
        s.defer_prefs_write = true;
    }
    let mut results: Vec<Value> = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let run = |s: &mut Session, results: &[Value]| -> Result<Value> {
            let id = step
                .get("command")
                .or_else(|| step.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| bad("engine.batch", "each step needs `command` (a command id)"))?;
            if NOT_IN_BATCH.contains(&id) {
                return Err(bad("engine.batch", format!("`{id}` can't run inside a batch")));
            }
            if atomic && NOT_IN_ATOMIC_BATCH.contains(&id) {
                let action = if id.starts_with("prefs.") {
                    "change preferences"
                } else if id.starts_with("shortcuts.") {
                    "change shortcuts"
                } else {
                    "save"
                };
                return Err(bad("engine.batch", format!("{id} cannot run inside an atomic batch; {action} after the batch or pass atomic: false")));
            }
            let empty = json!({});
            let raw = step.get("params").unwrap_or(&empty);
            let params = if step.get("substitute").and_then(Value::as_bool) == Some(false) { raw.clone() } else { substitute(raw, results)? };
            s.execute_checked(id, params)
        };
        match run(s, &results) {
            Ok(r) => results.push(r),
            Err(e) => {
                let id = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str).unwrap_or("?");
                let note = if atomic {
                    s.project = project;
                    s.history = history;
                    s.state = state;
                    s.key_move = key_move;
                    s.prefs = prefs;
                    s.prefs_revision = prefs_revision;
                    s.defer_prefs_write = defer_prefs_write;
                    // Failed steps cannot leave a dialog, quit or other frontend action queued.
                    s.events = events;
                    s.sanitize_state();
                    s.bump();
                    "; the batch was rolled back (nothing changed)".to_string()
                } else {
                    fold(s, &project, &history, &label);
                    format!("; steps 1–{i} were applied")
                };
                return Err(EngineError::Other(format!("step {} ({id}): {e}{note}", i + 1)));
            }
        }
    }
    fold(s, &project, &history, &label);
    if atomic {
        s.defer_prefs_write = defer_prefs_write;
        if s.prefs != prefs {
            s.save_prefs();
        }
    }
    Ok(json!({"steps": results.len(), "results": results}))
}

/// Record the batch from the retained entry snapshot: individual steps may already have
/// trimmed that snapshot out of their temporary undo line, or merged with an earlier gesture.
fn fold(s: &mut Session, snap: &Arc<effectcraft_project::Project>, history: &crate::History, label: &str) {
    if Arc::ptr_eq(&s.project, snap) {
        return;
    }
    s.history = history.clone();
    let levels = s.prefs.general.undo_levels.max(1) as usize;
    s.history.record(label, snap.clone(), levels);
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "engine.batch",
        label: "Run Commands (Batch)",
        menu: &[],
        shortcut: None,
        params: "{steps: [{command, params?}], label? (undo step name, default Batch), atomic?: bool (default true: a failing step rolls the whole batch back)} → {steps, results: [each step's result]}; a string param \"$N\" or \"$N.key.0\" is step N's result (1-based), e.g. {\"layer\": \"$1.layer\"}",
        enabled: always,
        run: batch,
        // The steps journal themselves.
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn batch_refs_one_undo_step_and_rollback() {
        let mut s = Session::default();
        s.execute("comp.new", json!({"name": "B", "width": 100, "height": 100})).unwrap();
        let undo_before = s.history.undo.len();
        let r = s
            .execute_checked(
                "engine.batch",
                json!({"label": "Build", "steps": [
                    {"command": "layer.newSolid", "params": {"name": "Box", "color": "#ff0000"}},
                    {"command": "prop.set", "params": {"layer": "$1.layer", "path": "transform/opacity", "value": 40}},
                    {"command": "layer.rename", "params": {"layer": "$1.layer", "name": "Red Box"}},
                ]}),
            )
            .unwrap();
        assert_eq!(r["steps"], 3);
        let lid = r["results"][0]["layer"].as_u64().unwrap();
        let comp = s.active_comp().unwrap();
        let l = comp.layer(effectcraft_project::LayerId(lid)).unwrap();
        assert_eq!(l.name, "Red Box");
        assert_eq!(l.props.prop("transform/opacity").unwrap().value.as_f64(), 40.0);
        assert_eq!(s.history.undo.len(), undo_before + 1, "one undo step");
        assert_eq!(s.history.undo.last().unwrap().0, "Build");
        assert!(s.undo());
        assert!(s.active_comp().unwrap().layers.is_empty());
        assert!(s.redo());

        // A failing step rolls back the earlier ones and names the step.
        let n = s.active_comp().unwrap().layers.len();
        let e = s
            .execute_checked(
                "engine.batch",
                json!({"steps": [
                    {"command": "layer.newNull", "params": {}},
                    {"command": "prop.set", "params": {"layer": "$1.layer", "path": "transform/nope", "value": 1}},
                ]}),
            )
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("step 2 (prop.set)") && e.contains("rolled back"), "{e}");
        assert_eq!(s.active_comp().unwrap().layers.len(), n);
        // Bad references are reported.
        let e = s.execute_checked("engine.batch", json!({"steps": [{"command": "layer.rename", "params": {"layer": "$3.layer", "name": "x"}}]})).unwrap_err();
        assert!(e.to_string().contains("refers to step 3"), "{e}");
    }

    fn small_history_session() -> Session {
        let mut s = Session::default();
        s.execute("comp.new", json!({"width": 64, "height": 64, "duration": 10, "frameRate": 30})).unwrap();
        s.execute("layer.newSolid", json!({"name": "Box", "color": "#ff0000"})).unwrap();
        s.execute("prefs.set", json!({"key": "general.undoLevels", "value": 2})).unwrap();
        s.history.clear();
        s
    }

    fn three_keys() -> serde_json::Value {
        json!([
            {"command": "prop.addKey", "params": {"layer": "Box", "path": "transform/opacity", "time": 0, "value": 0}},
            {"command": "prop.addKey", "params": {"layer": "Box", "path": "transform/opacity", "time": 1, "value": 50}},
            {"command": "prop.addKey", "params": {"layer": "Box", "path": "transform/opacity", "time": 2, "value": 100}}
        ])
    }

    #[test]
    fn batch_larger_than_history_budget_keeps_entry_snapshot_and_one_redo() {
        let mut s = small_history_session();
        for width in [65, 66] {
            s.execute("comp.settings", json!({"width": width})).unwrap();
        }
        let before = s.project.clone();
        let previous = s.history.undo.last().unwrap().1.clone();
        s.execute_checked("engine.batch", json!({"label": "Three keys", "steps": three_keys()})).unwrap();
        let after = s.project.clone();
        assert_eq!(s.history.undo.len(), 2, "the batch consumes one bounded history slot");
        assert_eq!(s.history.undo.last().unwrap().0, "Three keys");
        assert!(std::sync::Arc::ptr_eq(&s.history.undo.last().unwrap().1, &before));
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.project, &before), "one undo removes every key");
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.project, &previous), "the earlier edit survives intermediate trims");
        assert!(s.redo() && s.redo());
        assert!(std::sync::Arc::ptr_eq(&s.project, &after));
    }

    #[test]
    fn batch_larger_than_history_budget_preserves_the_previous_redo_branch() {
        let mut s = small_history_session();
        s.execute("comp.settings", json!({"width": 65})).unwrap();
        s.execute("comp.settings", json!({"width": 66})).unwrap();
        let future = s.project.clone();
        assert!(s.undo());
        let before = s.project.clone();
        s.execute_checked("engine.batch", json!({"steps": three_keys()})).unwrap();
        assert!(s.history.redo.is_empty());
        assert_eq!(s.history.branches.len(), 1);
        assert!(std::sync::Arc::ptr_eq(&s.history.branches[0].parent, &before));
        assert!(std::sync::Arc::ptr_eq(&s.history.branches[0].steps[0].1, &future));
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.project, &before));
        let id = s.history_tree().iter().find(|n| !n.line).unwrap().id.clone();
        assert!(s.goto_history(&id));
        assert!(std::sync::Arc::ptr_eq(&s.project, &future));
    }

    #[test]
    fn failed_batches_past_history_budget_restore_or_fold_the_complete_applied_prefix() {
        for atomic in [true, false] {
            let mut s = small_history_session();
            let before = s.project.clone();
            let mut steps = three_keys().as_array().unwrap().clone();
            steps.push(json!({"command": "prop.set", "params": {"layer": "Box", "path": "missing", "value": 10}}));
            let e = s.execute_checked("engine.batch", json!({"label": "Partial keys", "atomic": atomic, "steps": steps})).unwrap_err();
            assert!(e.to_string().contains("step 4"));
            if atomic {
                assert!(std::sync::Arc::ptr_eq(&s.project, &before));
                assert!(s.history.undo.is_empty());
            } else {
                assert_eq!(s.history.undo.len(), 1);
                assert_eq!(s.history.undo[0].0, "Partial keys");
                let after = s.project.clone();
                assert!(s.undo());
                assert!(std::sync::Arc::ptr_eq(&s.project, &before));
                assert!(s.redo());
                assert!(std::sync::Arc::ptr_eq(&s.project, &after));
            }
        }
    }

    #[test]
    fn batch_does_not_merge_into_a_preceding_gesture_with_the_same_key() {
        let mut s = small_history_session();
        s.execute("prop.set", json!({"layer": "Box", "path": "transform/opacity", "value": 90, "merge": "drag"})).unwrap();
        let before = s.project.clone();
        s.execute_checked(
            "engine.batch",
            json!({"steps": [
                {"command": "prop.set", "params": {"layer": "Box", "path": "transform/opacity", "value": 80, "merge": "drag"}}
            ]}),
        )
        .unwrap();
        assert_eq!(s.history.undo.len(), 2);
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.project, &before));
        assert!(s.undo());
        let info = s.execute("prop.get", json!({"layer": "Box", "path": "transform/opacity"})).unwrap();
        assert_eq!(info["value"], 100.0);
    }
}
