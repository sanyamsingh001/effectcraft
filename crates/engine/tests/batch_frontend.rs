//! Virtual Session events only: never dispatch a quit or settings event to an application.
use effectcraft_engine::{Event, Session, config::MemoryConfig};
use serde_json::json;
use std::sync::Arc;
fn setup() -> (Session, Arc<MemoryConfig>) {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    s.execute_checked("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":4})).unwrap();
    s.mark_saved();
    s.history.clear();
    s.events.clear();
    (s, store)
}
fn hooks(s: &Session) -> Vec<String> {
    s.events
        .iter()
        .filter_map(|event| match event {
            Event::Frontend { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect()
}
#[test]
fn failed_atomic_batch_drops_only_its_frontend_requests() {
    for request in ["app.quit", "app.settings"] {
        let (mut s, store) = setup();
        s.execute_checked("app.about", json!({})).unwrap();
        let prior = hooks(&s);
        let project = s.project.clone();
        let state = serde_json::to_value(&s.state).unwrap();
        let prefs = s.prefs.clone();
        let revision = s.prefs_revision;
        let files = store.files.lock().unwrap().clone();
        let result = s.execute_checked(
            "engine.batch",
            json!({"steps":[
             {"command":"layer.newSolid","params":{"name":"Inside","color":"#fff"}},
             {"command":request},
             {"command":"prop.set","params":{"layer":"$1.layer","path":"transform/nope","value":0}}
            ]}),
        );
        assert!(result.is_err());
        assert!(Arc::ptr_eq(&s.project, &project));
        assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
        assert_eq!(s.prefs, prefs);
        assert_eq!(s.prefs_revision, revision);
        assert_eq!(*store.files.lock().unwrap(), files);
        assert!(s.history.undo.is_empty());
        assert!(!s.defer_prefs_write);
        assert!(!s.is_dirty());
        assert_eq!(hooks(&s), prior, "failed atomic batch must not leak a new {request} request; pre-existing requests remain queued");
    }
}
#[test]
fn standalone_and_non_atomic_frontend_requests_still_reach_the_virtual_queue() {
    let (mut s, store) = setup();
    let files = store.files.lock().unwrap().clone();
    s.execute_checked("app.settings", json!({})).unwrap();
    assert_eq!(hooks(&s), vec!["app.settings"]);
    s.events.clear();
    let result = s.execute_checked(
        "engine.batch",
        json!({"atomic":false,"steps":[{"command":"app.settings"},{"command":"prop.set","params":{"layer":"missing","path":"transform/opacity","value":0}}]}),
    );
    assert!(result.is_err());
    assert_eq!(hooks(&s), vec!["app.settings"]);
    assert_eq!(*store.files.lock().unwrap(), files);
}
#[test]
fn successful_atomic_frontend_and_document_batch_keeps_one_undo_and_request() {
    let (mut s, store) = setup();
    let files = store.files.lock().unwrap().clone();
    s.execute_checked("engine.batch", json!({"steps":[{"command":"layer.newSolid","params":{"name":"Inside","color":"#fff"}},{"command":"app.settings"}]}))
        .unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), 1);
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(hooks(&s), vec!["app.settings"]);
    assert_eq!(*store.files.lock().unwrap(), files);
}

#[derive(Default)]
struct VirtualFiles {
    files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}
impl effectcraft_engine::Services for VirtualFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "virtual file missing"))
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.files.lock().unwrap().insert(path.into(), bytes.to_vec());
        Ok(())
    }
    fn exists(&self, path: &str) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }
}
fn virtual_save_setup() -> (Session, Arc<VirtualFiles>) {
    let files = Arc::new(VirtualFiles::default());
    let mut s = Session { services: files.clone(), ..Default::default() };
    s.execute_checked("comp.new", json!({"width":4,"height":4,"frameRate":30,"duration":4})).unwrap();
    s.mark_saved();
    s.history.clear();
    s.events.clear();
    (s, files)
}
const VIRTUAL_COPY: &str = "/virtual/effectcraft-copy.ecproj";
#[test]
fn standalone_save_copy_writes_only_the_virtual_store_and_preserves_checkpoint() {
    let (mut s, files) = virtual_save_setup();
    let project = s.project.clone();
    let checkpoint = s.saved_project.clone();
    let saved = s.saved_revision;
    let result = s.execute_checked("file.saveCopy", json!({"path":VIRTUAL_COPY})).unwrap();
    let bytes = files.files.lock().unwrap().get(VIRTUAL_COPY).unwrap().clone();
    assert_eq!(result["bytes"], json!(bytes.len()));
    assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
    assert!(Arc::ptr_eq(&s.project, &project));
    assert!(Arc::ptr_eq(&s.saved_project, &checkpoint));
    assert_eq!(s.saved_revision, saved);
    assert!(s.history.undo.is_empty() && !s.is_dirty());
}
#[test]
fn non_atomic_save_copy_retains_its_explicitly_requested_virtual_write() {
    let (mut s, files) = virtual_save_setup();
    let result = s.execute_checked(
        "engine.batch",
        json!({"atomic":false,"steps":[
            {"command":"file.saveCopy","params":{"path":VIRTUAL_COPY}},
            {"command":"prop.set","params":{"layer":"missing","path":"transform/opacity","value":0}}
        ]}),
    );
    assert!(result.is_err());
    assert!(files.files.lock().unwrap().contains_key(VIRTUAL_COPY));
    assert!(s.history.undo.is_empty() && !s.is_dirty());
}
#[test]
fn failed_atomic_save_copy_cannot_publish_a_virtual_file_before_rollback() {
    let (mut s, files) = virtual_save_setup();
    let project = s.project.clone();
    let checkpoint = s.saved_project.clone();
    let state = serde_json::to_value(&s.state).unwrap();
    let result = s.execute_checked(
        "engine.batch",
        json!({"steps":[
            {"command":"layer.newSolid","params":{"name":"Inside","color":"#fff"}},
            {"command":"file.saveCopy","params":{"path":VIRTUAL_COPY}},
            {"command":"prop.set","params":{"layer":"$1.layer","path":"transform/nope","value":0}}
        ]}),
    );
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&s.project, &project));
    assert!(Arc::ptr_eq(&s.saved_project, &checkpoint));
    assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    assert!(s.history.undo.is_empty() && !s.is_dirty());
    let published = files.files.lock().unwrap().clone();
    if let Some(bytes) = published.get(VIRTUAL_COPY) {
        assert!(serde_json::from_slice::<serde_json::Value>(bytes).is_ok(), "the leaked copy is a real serialized project");
    }
    assert!(published.is_empty(), "a failed atomic batch must reject saving before the virtual write: {:?}", published.keys().collect::<Vec<_>>());
}
