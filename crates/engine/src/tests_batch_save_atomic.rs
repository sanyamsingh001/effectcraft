//! Atomic batches cannot persist a project or replace its saved checkpoint.
use crate::Session;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn setup(case: &str) -> (Session, PathBuf, PathBuf, String) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().unwrap().parent().unwrap();
    let dir = workspace.join(".tmp").join("batch-save-regressions").join(format!("{}-{case}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("Original.ecproj");
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":4})).unwrap();
    s.execute_checked("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap();
    s.execute_checked("file.saveAs", json!({"path":original.to_str().unwrap()})).unwrap();
    let original_bytes = std::fs::read_to_string(&original).unwrap();
    s.history.clear();
    assert!(!s.is_dirty());
    (s, dir, original, original_bytes)
}
fn save_params(command: &str, dir: &std::path::Path) -> Value {
    if command == "file.saveAs" { json!({"path":dir.join("Other.ecproj").to_str().unwrap()}) } else { json!({}) }
}
fn steps(command: &str, dir: &std::path::Path) -> Value {
    json!([
        {"command":"prop.set","params":{"layer":"Box","path":"transform/opacity","value":20}},
        {"command":command,"params":save_params(command,dir)},
        {"command":"prop.set","params":{"layer":"Box","path":"transform/no-such-property","value":10}}
    ])
}
fn atomic_rejects_save(command: &str, case: &str) {
    let (mut s, dir, original, original_bytes) = setup(case);
    let before = s.project.clone();
    let path = s.path.clone();
    let error = s.execute_checked("engine.batch", json!({"steps":steps(command,&dir)})).unwrap_err().to_string();
    let files = std::fs::read_dir(&dir).unwrap().count();
    let same_file = std::fs::read_to_string(&original).unwrap() == original_bytes;
    let checkpoint = (Arc::ptr_eq(&s.project, &before), s.path == path, !s.is_dirty(), same_file, files);
    eprintln!("{command}: {error}; checkpoint={checkpoint:?}; path={:?}", s.path);
    assert_eq!(checkpoint, (true, true, true, true, 1), "atomic failure preserves project, path, cleanliness and saved file");
    assert!(s.history.undo.is_empty());
    assert!(error.contains("step 2") && error.contains(command) && error.contains("atomic"), "{error}");
}
#[test]
fn atomic_batch_rejects_save_before_overwriting_the_existing_project_file() {
    atomic_rejects_save("file.save", "atomic-save");
}
#[test]
fn atomic_batch_rejects_save_as_before_changing_saved_path_or_creating_a_file() {
    atomic_rejects_save("file.saveAs", "atomic-save-as");
}
#[test]
fn atomic_batch_rejects_increment_save_before_creating_the_next_project_file() {
    atomic_rejects_save("file.incrementAndSave", "atomic-increment");
}
#[test]
fn explicit_non_atomic_batches_keep_all_save_variants_and_the_applied_prefix() {
    for (command, case) in [("file.save", "non-atomic-save"), ("file.saveAs", "non-atomic-save-as"), ("file.incrementAndSave", "non-atomic-increment")] {
        let (mut s, dir, _, _) = setup(case);
        let before = s.project.clone();
        let error = s.execute_checked("engine.batch", json!({"atomic":false,"steps":steps(command,&dir)})).unwrap_err().to_string();
        assert!(error.contains("step 3") && error.contains("steps 1"), "{error}");
        assert!(!s.is_dirty(), "the applied project was saved");
        let saved = s.path.as_ref().unwrap();
        assert!(PathBuf::from(saved).starts_with(&dir));
        assert!(std::fs::metadata(saved).unwrap().is_file());
        let opacity = s.execute_checked("prop.get", json!({"layer":"Box","path":"transform/opacity"})).unwrap();
        assert_eq!(opacity["value"], 20.0);
        assert_eq!(s.history.undo.len(), 1);
        assert!(s.undo());
        assert!(Arc::ptr_eq(&s.project, &before));
        assert!(s.is_dirty());
    }
}
#[test]
fn explicit_non_atomic_save_copy_keeps_its_existing_checkpoint_and_one_undo_behavior() {
    let (mut s, dir, original, _) = setup("atomic-copy");
    let before = s.project.clone();
    let copy = dir.join("Copy.ecproj");
    s.execute_checked(
        "engine.batch",
        json!({"atomic":false,"steps":[
            {"command":"prop.set","params":{"layer":"Box","path":"transform/opacity","value":20}},
            {"command":"file.saveCopy","params":{"path":copy.to_str().unwrap()}}
        ]}),
    )
    .unwrap();
    assert_eq!(s.path.as_deref(), original.to_str());
    assert!(s.is_dirty());
    assert!(copy.is_file());
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(!s.is_dirty());
}
