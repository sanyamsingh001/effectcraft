//! Label colors in real Project, Timeline, keyframe and Edit menus (#290).

use effectcraft_engine::{
    Session,
    color::Label,
    project::{ItemId, LayerId},
};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

const NAME: &str = "Accent Green";
const COLOR: [u8; 3] = [19, 201, 87];

fn harness() -> (Harness<'static, EffectcraftApp>, LayerId, ItemId, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Main", "width":320,"height":180,"duration":4})).unwrap();
    let item = s.active_comp_id().unwrap();
    let layer = LayerId(s.execute("layer.newSolid", json!({"name":"Plate","color":"#406080"})).unwrap()["layer"].as_u64().unwrap());
    for time in [0, 1] {
        s.execute("prop.addKey", json!({"layer": layer.0, "path":"transform/opacity", "time":time, "value":20})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(layer).unwrap().props.prop("transform/opacity").unwrap().uid;
    std::sync::Arc::make_mut(&mut s.project).settings.gpu_acceleration = false;
    let mut app = EffectcraftApp::new(s);
    app.set_pref("labels.8.name", json!(NAME)).unwrap();
    app.set_pref("labels.8.color", json!("#13c957")).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| app);
    h.run_steps(3);
    assert_eq!(h.state().session.prefs.label_name(Label::Green), NAME);
    assert_eq!(h.state().tokens.label(Label::Green).to_array()[..3], COLOR);
    (h, layer, item, uid)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

fn open_label_submenu(h: &mut Harness<'_, EffectcraftApp>) {
    let button = h.query_by_label("Label").or_else(|| h.query_by_label("Label ⏵")).expect("Label submenu").rect();
    click(h, button.center(), PointerButton::Primary);
}

/// The custom color must be a small square inside the live row, before the text.
fn assert_swatch(h: &mut Harness<'_, EffectcraftApp>, name: &str) -> Rect {
    let row = h.get_by_label(NAME).rect();
    assert!(h.ctx.content_rect().contains_rect(row));
    let scale = h.ctx.pixels_per_point();
    let img = h.render().expect("offscreen menu render");
    if let Ok(dir) = std::env::var("LABEL_MENU_SNAPSHOTS") {
        img.save(format!("{dir}/{name}.png")).unwrap();
    }
    let mut pixels = Vec::new();
    for y in (row.min.y * scale).ceil() as u32..(row.max.y * scale).floor() as u32 {
        for x in (row.min.x * scale).ceil() as u32..(row.max.x * scale).floor() as u32 {
            let c = img.get_pixel(x, y).0;
            if c[..3].iter().zip(COLOR).all(|(a, b)| a.abs_diff(b) <= 2) {
                pixels.push((x, y));
            }
        }
    }
    assert!(pixels.len() >= 20, "{name}: row has no visible custom-color square");
    let min_x = pixels.iter().map(|p| p.0).min().unwrap();
    let max_x = pixels.iter().map(|p| p.0).max().unwrap();
    let min_y = pixels.iter().map(|p| p.1).min().unwrap();
    let max_y = pixels.iter().map(|p| p.1).max().unwrap();
    assert!((max_x - min_x) as f32 <= 12.0 * scale && (max_y - min_y) as f32 <= 12.0 * scale, "{name}: swatch exceeds its menu gutter");
    assert!((max_x as f32 / scale) < row.left() + 44.0, "{name}: swatch overlays the label name");
    row
}

#[test]
fn project_label_menu_shows_custom_color_and_undoes_the_choice() {
    let (mut h, _, item, _) = harness();
    let before = h.state().session.project.item(item).unwrap().label;
    let at = rect(&h, &format!("project.item.{}.label", item.0)).center();
    click(&mut h, at, PointerButton::Primary);
    let row = assert_swatch(&mut h, "project-label-menu");
    let undo = h.state().session.history.undo.len();
    click(&mut h, row.center(), PointerButton::Primary);
    assert_eq!(h.state().session.project.item(item).unwrap().label, Label::Green);
    assert_eq!(h.state().session.history.undo.len(), undo + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.project.item(item).unwrap().label, before);
}

#[test]
fn timeline_layer_label_menu_shows_custom_color_and_undoes_the_choice() {
    let (mut h, layer, _, _) = harness();
    let before = h.state().session.active_comp().unwrap().layer(layer).unwrap().label;
    let at = rect(&h, &format!("timeline.layer.{}.label", layer.0)).center();
    click(&mut h, at, PointerButton::Secondary);
    let row = assert_swatch(&mut h, "timeline-label-menu");
    let undo = h.state().session.history.undo.len();
    click(&mut h, row.center(), PointerButton::Primary);
    assert_eq!(h.state().session.active_comp().unwrap().layer(layer).unwrap().label, Label::Green);
    assert_eq!(h.state().session.history.undo.len(), undo + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(layer).unwrap().label, before);
}

#[test]
fn keyframe_label_submenu_shows_custom_color_and_undoes_the_choice() {
    let (mut h, layer, _, uid) = harness();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.opacity", json!({})).unwrap();
    h.run_steps(3);
    let at = rect(&h, &format!("timeline.key.{uid}.30")).center();
    click(&mut h, at, PointerButton::Secondary);
    open_label_submenu(&mut h);
    let row = assert_swatch(&mut h, "keyframe-label-menu");
    let undo = h.state().session.history.undo.len();
    click(&mut h, row.center(), PointerButton::Primary);
    let label = |h: &Harness<'_, EffectcraftApp>| h.state().session.active_comp().unwrap().layer(layer).unwrap().props.find(uid).unwrap().keys[1].label;
    assert_eq!(label(&h), Label::ALL.iter().position(|l| *l == Label::Green).unwrap() as u8);
    assert_eq!(h.state().session.history.undo.len(), undo + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(label(&h), 0);
}

#[test]
fn edit_label_submenu_shows_custom_color_and_undoes_the_choice() {
    let (mut h, layer, _, _) = harness();
    h.state_mut().ui.focused = PanelKind::Timeline;
    let before = h.state().session.active_comp().unwrap().layer(layer).unwrap().label;
    let at = rect(&h, "menu.Edit").center();
    click(&mut h, at, PointerButton::Primary);
    open_label_submenu(&mut h);
    let row = assert_swatch(&mut h, "edit-label-menu");
    let undo = h.state().session.history.undo.len();
    click(&mut h, row.center(), PointerButton::Primary);
    assert_eq!(h.state().session.active_comp().unwrap().layer(layer).unwrap().label, Label::Green);
    assert_eq!(h.state().session.history.undo.len(), undo + 1);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layer(layer).unwrap().label, before);
}

fn project_label_popup_visible(h: &Harness<'_, EffectcraftApp>) -> bool {
    h.state().auto.find("project.labelMenu.8").is_some_and(|e| {
        let row = Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]));
        row.is_positive() && h.ctx.content_rect().contains_rect(row)
    })
}

fn project_label_popup() -> (Harness<'static, EffectcraftApp>, ItemId) {
    let (mut h, _, item, _) = harness();
    h.state_mut().session.execute("project.select", json!({"items": [item.0]})).unwrap();
    h.state_mut().ui.focused = PanelKind::Project;
    h.state_mut().ui.maximized = Some(PanelKind::Project);
    h.run_steps(3);
    let label = rect(&h, &format!("project.item.{}.label", item.0));
    assert!(rect(&h, "panel.Project").contains_rect(label), "the label anchor is visible inside Project");
    click(&mut h, label.center(), PointerButton::Primary);
    assert!(project_label_popup_visible(&h), "a label option is visible inside the open Project popup");
    (h, item)
}

fn menu_key(h: &mut Harness<'_, EffectcraftApp>, key: egui::Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

#[test]
fn project_label_popup_owns_delete_until_escape() {
    let (mut h, item) = project_label_popup();
    let revision = h.state().session.revision;
    let selected = h.state().session.state.project_selection.clone();
    menu_key(&mut h, egui::Key::Delete);
    assert!(h.state().session.project.item(item).is_some(), "Delete cannot remove the Project item beneath its open label popup");
    assert_eq!(h.state().session.revision, revision);
    assert_eq!(h.state().session.state.project_selection, selected);
    assert!(h.state().dialog.is_none(), "Delete cannot open another dialog beneath the label popup");
    assert!(project_label_popup_visible(&h), "Delete leaves the label popup open");
    menu_key(&mut h, egui::Key::Escape);
    assert!(!project_label_popup_visible(&h), "Escape closes the label popup");
    menu_key(&mut h, egui::Key::Delete);
    assert!(h.state().session.project.item(item).is_none(), "Delete resumes after the label popup closes");
}

#[test]
fn project_label_popup_owns_space_until_escape() {
    let (mut h, _) = project_label_popup();
    let (revision, time) = (h.state().session.revision, h.state().session.time());
    menu_key(&mut h, egui::Key::Space);
    assert!(!h.state().playback.playing, "Space cannot start a preview beneath the open label popup");
    assert_eq!(h.state().session.revision, revision);
    assert_eq!(h.state().session.time(), time);
    assert!(project_label_popup_visible(&h), "Space leaves the label popup open");
    menu_key(&mut h, egui::Key::Escape);
    assert!(!project_label_popup_visible(&h), "Escape closes the label popup");
    menu_key(&mut h, egui::Key::Space);
    assert!(h.state().playback.playing, "Space resumes after the label popup closes");
}

/// Maximizing another panel hides Project; its old popup cannot keep shortcuts forever.
#[test]
fn hidden_project_label_popup_releases_shortcuts() {
    let (mut h, item) = project_label_popup();
    let layer = h.state().session.active_comp().unwrap().layers[0].id;
    h.state_mut().ui.maximized = Some(PanelKind::Timeline);
    h.state_mut().ui.focused = PanelKind::Timeline;
    h.run_steps(3);
    assert!(!project_label_popup_visible(&h), "the hidden Project popup is no longer drawn");
    menu_key(&mut h, egui::Key::Delete);
    assert!(h.state().session.active_comp().unwrap().layer(layer).is_none(), "the visible Timeline owns Delete after Project is hidden");
    assert!(h.state().session.project.item(item).is_some());
}
