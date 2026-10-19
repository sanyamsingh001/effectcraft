//! Menu keyboard ownership and navigation, without native-window dependencies.
use effectcraft_engine::Session;
use effectcraft_ui_egui::{Dialog, EffectcraftApp};
use egui::{Event, Key, Modifiers, PointerButton, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Main", "width":320,"height":180,"duration":4})).unwrap();
    s.execute("layer.newNull", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1200.0, 520.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn open(h: &mut Harness<'_, EffectcraftApp>, name: &str) {
    let e = h.state().auto.find(&format!("menu.{name}")).unwrap();
    let at = Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3])).center();
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    assert!(egui::Popup::is_any_open(&h.ctx));
}

fn key(h: &mut Harness<'_, EffectcraftApp>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

#[test]
fn open_menus_own_preview_delete_and_time_shortcuts() {
    let mut h = harness();
    open(&mut h, "Composition");
    let layers = h.state().session.active_comp().unwrap().layers.len();
    let time = h.state().session.time();
    for k in [Key::Space, Key::Delete, Key::PageDown] {
        key(&mut h, k);
        assert!(egui::Popup::is_any_open(&h.ctx));
        assert!(!h.state().playback.playing);
        assert_eq!(h.state().session.active_comp().unwrap().layers.len(), layers);
        assert_eq!(h.state().session.time(), time);
    }
    key(&mut h, Key::Escape);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    key(&mut h, Key::PageDown);
    assert!(h.state().session.time() > time, "shortcuts resume when the menu closes");
}

#[test]
fn arrows_focus_menu_entries_and_enter_runs_the_focused_command() {
    let mut h = harness();
    open(&mut h, "Composition");
    key(&mut h, Key::ArrowDown);
    assert!(h.ctx.memory(|m| m.focused()).is_some());
    key(&mut h, Key::Enter);
    assert_eq!(h.state().dialog, Some(Dialog::NewComp));
}

#[test]
fn arrow_keys_open_and_close_submenus_and_change_headings() {
    let mut h = harness();
    open(&mut h, "Layer");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowRight);
    assert!(h.query_by_label("Null Object").is_some(), "Right opens Layer's focused New submenu");
    key(&mut h, Key::ArrowLeft);
    assert!(h.query_by_label("Null Object").is_none(), "Left returns from the submenu");
    key(&mut h, Key::ArrowLeft);
    assert!(h.query_by_label_contains("New Composition").is_some(), "Left changes the top-level heading");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowUp);
    key(&mut h, Key::Enter);
    assert_eq!(h.state().dialog, Some(Dialog::NewComp));
}

#[test]
fn switching_menu_headings_focuses_the_first_command_for_enter() {
    let mut h = harness();
    open(&mut h, "Layer");
    key(&mut h, Key::ArrowLeft);
    assert!(h.query_by_label_contains("New Composition").is_some());
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "the opened heading focuses its first enabled item");
    key(&mut h, Key::Enter);
    assert_eq!(h.state().dialog, Some(Dialog::NewComp));
}

#[test]
fn left_during_submenu_sizing_returns_to_its_parent_heading() {
    let mut h = harness();
    open(&mut h, "Layer");
    key(&mut h, Key::ArrowDown);
    h.event(Event::Key { key: Key::ArrowRight, physical_key: Some(Key::ArrowRight), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    // The new child measures its rows on this next pass. Left must return to Layer.
    h.event(Event::Key { key: Key::ArrowRight, physical_key: Some(Key::ArrowRight), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.event(Event::Key { key: Key::ArrowLeft, physical_key: Some(Key::ArrowLeft), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::Key { key: Key::ArrowLeft, physical_key: Some(Key::ArrowLeft), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.query_by_label("Null Object").is_none(), "Left closes the child");
    assert!(h.query_by_label("New").is_some(), "the Layer heading remains open");
    assert!(h.query_by_label_contains("New Composition").is_none(), "Left must not skip the parent heading");
    key(&mut h, Key::ArrowRight);
    assert!(h.query_by_label("Null Object").is_some(), "the parent retains keyboard focus for the next navigation");
}

#[test]
fn effect_controls_context_submenu_focuses_its_first_command_for_enter() {
    use effectcraft_engine::menus::MenuNode;
    use effectcraft_ui_egui::dock::PanelKind;

    let mut h = harness();
    h.state_mut().session.execute("effect.apply", json!({"effect":"Fill"})).unwrap();
    h.state_mut().show_panel(PanelKind::EffectControls);
    h.state_mut().ui.maximized = Some(PanelKind::EffectControls);
    h.run_steps(3);
    let layer = h.state().session.active_comp().unwrap().layers[0].id;
    let before = h.state().session.project.clone();
    let effects = h.state().session.active_comp().unwrap().layer(layer).unwrap().effects().unwrap().groups().count();
    let history = h.state().session.history.undo.len();
    let panel = h.state().auto.find("panel.EffectControls").unwrap();
    let rect = Rect::from_min_size(pos2(panel.rect[0], panel.rect[1]), vec2(panel.rect[2], panel.rect[3]));
    let at = pos2(rect.center().x, rect.max.y - 30.0);
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    let category = h.get_by_label("Blur & Sharpen").rect().center();
    h.event(Event::PointerMoved(category));
    h.run_steps(3);
    let first = effectcraft_engine::menus::menu_bar()
        .iter()
        .find_map(|node| match node {
            MenuNode::Submenu { label, children } if label == "Effect" => children.iter().find_map(|node| match node {
                MenuNode::Submenu { label, children } if label == "Blur & Sharpen" => children.iter().find_map(|node| match node {
                    MenuNode::Item(entry) => Some(entry.label.clone()),
                    _ => None,
                }),
                _ => None,
            }),
            _ => None,
        })
        .unwrap();
    assert!(h.query_by_label(&first).is_some(), "the real context submenu opened");
    // Leave the submenu button before keyboard navigation so hover cannot reopen it.
    h.event(Event::PointerMoved(pos2(rect.max.x + 20.0, rect.center().y)));
    h.step();
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowLeft);
    assert!(h.query_by_label(&first).is_none(), "Left returns to the context parent");
    key(&mut h, Key::ArrowRight);
    assert!(h.query_by_label(&first).is_some(), "Right opens the focused context submenu");
    key(&mut h, Key::Enter);
    let after = h.state().session.active_comp().unwrap().layer(layer).unwrap().effects().unwrap();
    assert_eq!(after.groups().count(), effects + 1);
    assert_eq!(after.groups().last().unwrap().name, first, "Enter runs the first enabled submenu command");
    assert_eq!(h.state().session.history.undo.len(), history + 1);
    assert!(h.state_mut().session.undo());
    assert!(std::sync::Arc::ptr_eq(&h.state().session.project, &before));
}
