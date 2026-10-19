//! Saved workspace restart regression (#320), without a native window.
use effectcraft_engine::{
    Session,
    config::{ConfigStore, DirConfig, MemoryConfig},
};
use effectcraft_ui_egui::{EffectcraftApp, state::UiState};
use serde_json::json;
use std::sync::Arc;

fn app(store: Arc<dyn ConfigStore>) -> EffectcraftApp {
    let mut session = Session { config: Some(store), ..Default::default() };
    // Keep this isolated fixture off platform disk-cache paths.
    session.prefs.disk.disk_cache_enabled = false;
    session.load_settings();
    EffectcraftApp::new(session)
}
fn invoke(app: &mut EffectcraftApp, id: &str, params: serde_json::Value) {
    effectcraft_ui_egui::menus::invoke(app, &egui::Context::default(), id, params).unwrap();
}
fn save_layout(app: &mut EffectcraftApp, name: &str) {
    invoke(app, "window.workspace", json!({"name":"Default"}));
    invoke(app, "window.floatPanel", json!({"panel":"Project","rect":[120,140,320,240]}));
    invoke(app, "window.saveWorkspaceAs", json!({"name":name}));
    assert_eq!(app.saved_workspace_names(), [name]);
    assert!(!app.ui.floating.is_empty());
}

#[test]
fn ui_state_codec_retains_named_and_floating_workspaces() {
    let store = Arc::new(MemoryConfig::default());
    let mut original = app(store);
    save_layout(&mut original, "Cutting");
    let data = serde_json::to_string(&original.ui).unwrap();
    let restored: UiState = serde_json::from_str(&data).unwrap();
    assert_eq!(restored.saved_workspaces, original.ui.saved_workspaces);
    assert_eq!(restored.saved_floating, original.ui.saved_floating);
}
#[test]
fn saved_workspace_reopens_in_a_fresh_app_with_memory_config() {
    let store = Arc::new(MemoryConfig::default());
    let mut original = app(store.clone());
    save_layout(&mut original, "Cutting");
    let (dock, floating) = (original.ui.dock.clone(), original.ui.floating.clone());
    eframe::App::on_exit(&mut original);
    drop(original);
    let mut reopened = app(store);
    assert_eq!(reopened.saved_workspace_names(), ["Cutting"], "saved layouts survive a clean application restart");
    invoke(&mut reopened, "window.workspace", json!({"name":"Cutting"}));
    assert_eq!(reopened.ui.dock, dock);
    assert_eq!(reopened.ui.floating, floating);
}
#[test]
fn saved_workspace_reopens_from_linux_directory_config() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/workspace-restart-after-dir");
    if directory.exists() {
        std::fs::remove_dir_all(&directory).unwrap();
    }
    let mut original = app(Arc::new(DirConfig::new(&directory)));
    save_layout(&mut original, "Editing");
    let (dock, floating) = (original.ui.dock.clone(), original.ui.floating.clone());
    eframe::App::on_exit(&mut original);
    drop(original);
    let mut reopened = app(Arc::new(DirConfig::new(&directory)));
    assert_eq!(reopened.saved_workspace_names(), ["Editing"], "native preferences restore saved layouts from their config directory");
    invoke(&mut reopened, "window.workspace", json!({"name":"Editing"}));
    assert_eq!(reopened.ui.dock, dock);
    assert_eq!(reopened.ui.floating, floating);
}

#[test]
fn explicit_workspace_save_is_durable_before_exit() {
    let store = Arc::new(MemoryConfig::default());
    let mut original = app(store.clone());
    save_layout(&mut original, "Cutting");
    let stored: serde_json::Value = serde_json::from_str(&store.read("workspaces.json").expect("Save As writes the workspace store immediately")).unwrap();
    assert!(stored["saved_workspaces"].get("Cutting").is_some());
    assert!(stored["saved_floating"]["Cutting"].as_array().is_some_and(|v| !v.is_empty()));
    let mut reopened = app(store);
    invoke(&mut reopened, "window.workspace", json!({"name":"Cutting"}));
    assert_eq!(reopened.ui.dock, original.ui.dock);
    assert_eq!(reopened.ui.floating, original.ui.floating);
}

#[test]
fn save_changes_to_a_builtin_workspace_survive_restart() {
    let store = Arc::new(MemoryConfig::default());
    let mut original = app(store.clone());
    invoke(&mut original, "window.floatPanel", json!({"panel":"Project","rect":[120,140,320,240]}));
    invoke(&mut original, "window.saveWorkspace", json!({}));
    assert!(original.saved_workspace_names().is_empty());
    let reopened = app(store);
    assert!(reopened.saved_workspace_names().is_empty());
    assert_eq!(reopened.ui.dock, original.ui.dock);
    assert_eq!(reopened.ui.floating, original.ui.floating);
}

#[test]
fn renamed_and_deleted_workspaces_survive_restart() {
    let store = Arc::new(MemoryConfig::default());
    let mut original = app(store.clone());
    save_layout(&mut original, "Cutting");
    invoke(&mut original, "window.editWorkspaces", json!({"name":"Cutting","rename":"Finishing"}));
    let mut reopened = app(store.clone());
    assert_eq!(reopened.saved_workspace_names(), ["Finishing"]);
    invoke(&mut reopened, "window.workspace", json!({"name":"Finishing"}));
    assert_eq!(reopened.ui.dock, original.ui.dock);
    assert_eq!(reopened.ui.floating, original.ui.floating);
    invoke(&mut reopened, "window.editWorkspaces", json!({"name":"Finishing","delete":true}));
    let reopened = app(store);
    assert!(reopened.saved_workspace_names().is_empty());
    assert!(reopened.ui.saved_floating.is_empty());
    assert_eq!(reopened.ui.workspace, "Default");
}

#[test]
fn malformed_workspace_store_keeps_the_default_layout() {
    let store = Arc::new(MemoryConfig::default());
    store.write("workspaces.json", "{not json").unwrap();
    let reopened = app(store);
    assert!(reopened.saved_workspace_names().is_empty());
    assert_eq!(reopened.ui.workspace, "Default");
    assert_eq!(reopened.ui.dock, effectcraft_ui_egui::dock::workspace("Default"));
}

#[test]
fn workspace_store_write_errors_are_reported_by_the_save_command() {
    struct Unwritable;
    impl ConfigStore for Unwritable {
        fn read(&self, _: &str) -> Option<String> {
            None
        }
        fn write(&self, _: &str, _: &str) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "workspace fixture is read-only"))
        }
        fn remove(&self, _: &str) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut original = app(Arc::new(Unwritable));
    let error = effectcraft_ui_egui::menus::invoke(&mut original, &egui::Context::default(), "window.saveWorkspaceAs", json!({"name":"Cutting"})).unwrap_err();
    assert!(error.contains("workspace fixture is read-only"), "{error}");
}
