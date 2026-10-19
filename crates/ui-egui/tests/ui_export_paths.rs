//! Export dialogs use the selected media extension (#293).

use std::cell::RefCell;
use std::rc::Rc;

use effectcraft_engine::Session;
use effectcraft_ui_egui::{EffectcraftApp, menus};
use egui::{Event, Modifiers, Pos2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

fn app() -> EffectcraftApp {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Spot", "width": 64, "height": 36, "frameRate": 25, "duration": 1})).unwrap();
    EffectcraftApp::new(s)
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn media_exports_offer_their_own_extension() {
    let mut app = app();
    let asked: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    let log = asked.clone();
    app.hooks.pick_save_file = Some(Box::new(move |name, ext| {
        log.borrow_mut().push((name.into(), ext.into()));
        None // Cancel: no output is written.
    }));
    app.hooks.pick_save = Some(Box::new(|_| panic!("media export opened the project picker")));
    let ctx = egui::Context::default();
    for (command, name, extension) in [
        ("file.exportLottie", "Animation.json", "json"),
        ("comp.saveFrameAs", "Frame.png", "png"),
        ("comp.saveFrameAsPsd", "Frame.psd", "psd"),
        ("comp.saveFrameAsExr", "Frame.exr", "exr"),
        ("view.exportGuides", "Guides.json", "json"),
        ("file.saveCopyAsXml", "Untitled Project.ecprojx", "ecprojx"),
        ("render.saveCurrentPreview", "Preview.mp4", "mp4"),
    ] {
        assert_eq!(menus::invoke(&mut app, &ctx, command, json!({})).unwrap(), serde_json::Value::Null);
        assert_eq!(asked.borrow().last(), Some(&(name.into(), extension.into())), "{command}");
    }
}

#[test]
fn project_save_keeps_the_project_picker() {
    let mut app = app();
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = asked.clone();
    app.hooks.pick_save = Some(Box::new(move |name| {
        log.borrow_mut().push(name.into());
        None
    }));
    app.hooks.pick_save_file = Some(Box::new(|_, _| panic!("project save opened a media picker")));
    menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({})).unwrap();
    assert_eq!(*asked.borrow(), ["Untitled Project.ecproj"]);
}

#[test]
fn timeline_browse_uses_the_selected_format_and_folder() {
    let mut app = app();
    app.session.path = Some("/projects/current.ecproj".into());
    let asked: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    let log = asked.clone();
    app.hooks.pick_save_file = Some(Box::new(move |name, ext| {
        log.borrow_mut().push((name.into(), ext.into()));
        Some("/projects/Spot.fcpxml".into())
    }));
    app.hooks.pick_save = Some(Box::new(|_| panic!("timeline export opened the project picker")));
    menus::invoke(&mut app, &egui::Context::default(), "file.exportTimeline", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1000.0)).build_eframe(|_| app);
    h.run_steps(3);
    let format = h.state().auto.find("form.field.format").unwrap();
    let at = egui::pos2(format.rect[0] + format.rect[2] / 2.0, format.rect[1] + format.rect[3] / 2.0);
    click(&mut h, at);
    let option = h.query_by_label("FCPXML (.fcpxml)").expect("format menu option").rect();
    click(&mut h, option.center());
    let browse = h.state().auto.find("form.field.path.browse").unwrap();
    let at = egui::pos2(browse.rect[0] + browse.rect[2] / 2.0, browse.rect[1] + browse.rect[3] / 2.0);
    click(&mut h, at);
    assert_eq!(*asked.borrow(), [("/projects/Spot.fcpxml".into(), "fcpxml".into())]);
    assert!(h.state().auto.find("form.field.path").is_some(), "the chosen output remains in the form");
}
