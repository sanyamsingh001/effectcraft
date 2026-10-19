//! New Viewer must add a panel after a saved dock layout is restored.
use effectcraft_engine::Session;
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind, menus, state::UiState};
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

fn session() -> Session {
    let mut session = Session::default();
    session.prefs.disk.disk_cache_enabled = false;
    session.execute("comp.new", json!({"width":32,"height":32,"duration":2})).unwrap();
    session
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn new_viewer_adds_a_panel_after_a_saved_viewer_layout_is_restored() {
    let mut original = EffectcraftApp::new(session());
    let context = egui::Context::default();
    menus::invoke(&mut original, &context, "view.newViewer", json!({})).unwrap();
    let viewer = original.ui.active_viewer;
    assert_eq!(viewer, 1);
    menus::invoke(&mut original, &context, "window.saveWorkspaceAs", json!({"name":"Review"})).unwrap();
    let serialized = serde_json::to_string(&original.ui).unwrap();
    let restored: UiState = serde_json::from_str(&serialized).unwrap();
    let mut fresh = EffectcraftApp::new(session());
    // Workspace persistence restores layouts, rather than the old project's viewer documents.
    fresh.ui.saved_workspaces = restored.saved_workspaces;
    fresh.ui.saved_floating = restored.saved_floating;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| fresh);
    h.run_steps(4);
    let context = h.ctx.clone();
    menus::invoke(h.state_mut(), &context, "window.workspace", json!({"name":"Review"})).unwrap();
    h.run_steps(4); // Allow normal lazy viewer state updates before issuing New Viewer.
    assert!(h.state().ui.dock.contains(PanelKind::Viewer(viewer)));
    assert!(!h.state().ui.viewers.contains_key(&viewer), "a restored passive viewer has no old project document");
    let before = h.state().ui.dock.panel_count();
    let menu = h.state().auto.find("menu.View").expect("View menu");
    let at = pos2(menu.rect[0] + menu.rect[2] / 2.0, menu.rect[1] + menu.rect[3] / 2.0);
    click(&mut h, at);
    let at = h.get_by_label("New Viewer").rect().center();
    click(&mut h, at);
    eprintln!("restored viewer={viewer}, new viewer={}, panel count {before} -> {}", h.state().ui.active_viewer, h.state().ui.dock.panel_count());
    assert_eq!(h.state().ui.dock.panel_count(), before + 1, "New Viewer creates another panel after workspace restore");
    assert_ne!(h.state().ui.active_viewer, viewer, "new viewer does not reuse a restored panel's id");
    assert!(h.state().ui.dock.contains(PanelKind::Viewer(h.state().ui.active_viewer)));
    assert!(h.state().ui.dock.contains(PanelKind::Viewer(viewer)));
}
