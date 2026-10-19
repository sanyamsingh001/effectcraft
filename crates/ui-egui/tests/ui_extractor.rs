//! EXtractoR exposes the footage's actual channel names (#295).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemId, ItemKind, LayerId},
    raster::{AuxChannels, Image},
    render::FootageSource,
};
use effectcraft_time::Tick;
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Modifiers, Pos2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

struct Channels(Arc<AtomicUsize>);
impl FootageSource for Channels {
    fn frame(&self, _: ItemId, _: &Footage, _: Tick) -> Option<Arc<Image>> {
        None
    }
    fn aux(&self, _: ItemId, _: &Footage, _: Tick) -> Option<Arc<AuxChannels>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        let mut aux = AuxChannels::new(4, 4, 1.0);
        aux.channels.push(("beauty.R".into(), vec![0.25; 16]));
        aux.channels.push(("depth.Z".into(), vec![0.5; 16]));
        aux.channels.push(("diffuse.R".into(), vec![0.75; 16]));
        aux.channels.push(("-".into(), vec![0.1; 16]));
        aux.channels.push(("None".into(), vec![0.9; 16]));
        Some(Arc::new(aux))
    }
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn extractor_channel_dropdown_sets_a_named_exr_channel_and_undoes() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 64, "height": 36, "frameRate": 25, "duration": 1})).unwrap();
    let item = Arc::make_mut(&mut s.project).add_item(
        "Layers.exr",
        effectcraft_engine::color::Label::None,
        None,
        ItemKind::Footage(Footage { path: "Layers.exr".into(), width: 4, height: 4, kind: FootageKind::Still, has_video: true, ..Default::default() }),
    );
    let layer = s.execute("layer.addItem", json!({"item": item.0})).unwrap()["layer"].as_u64().unwrap();
    s.execute("effect.apply", json!({"layers": [layer], "effect": "EXtractoR"})).unwrap();
    let uid = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().effects().unwrap().groups().next().unwrap().get("red").unwrap().uid;
    let reads = Arc::new(AtomicUsize::new(0));
    s.footage = Arc::new(Channels(reads.clone()));
    let mut app = EffectcraftApp::new(s);
    app.show_panel(PanelKind::EffectControls);
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app);
    h.run_steps(4);
    let red = |h: &Harness<'_, EffectcraftApp>| {
        h.state().session.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop(&format!("@{uid}")).unwrap().value.clone()
    };
    for (label, value) in [("diffuse.R", "diffuse.R"), ("- (channel)", "-"), ("None (channel)", "None")] {
        let control = h.state().auto.find(&format!("effectControls.prop.{uid}.value")).expect("channel dropdown");
        let at = egui::pos2(control.rect[0] + control.rect[2] / 2.0, control.rect[1] + control.rect[3] / 2.0);
        click(&mut h, at);
        let named = h.query_by_label(label).expect("actual EXR channel").rect();
        assert!(h.query_by_label("beauty.R").is_some());
        assert!(h.query_by_label("depth.Z").is_some());
        click(&mut h, named.center());
        assert_eq!(red(&h), effectcraft_engine::keyframe::Value::Str(value.into()));
        h.state_mut().session.undo();
        assert_eq!(red(&h), effectcraft_engine::keyframe::Value::Str("R".into()));
        h.run_steps(2);
    }
    assert!(reads.load(Ordering::Relaxed) > 0);
}
