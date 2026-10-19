//! Essential Graphics Scale uses percentage axes and a persisted proportions switch.
use effectcraft_engine::{
    Session,
    project::{ItemId, LayerId},
};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

struct ScaleFixture {
    source: ItemId,
    source_layer: u64,
    control: u64,
    instance: Option<u64>,
}

fn app(three_d: bool, with_instance: bool) -> (EffectcraftApp, ScaleFixture) {
    let mut s = Session::default();
    s.prefs.disk.disk_cache_enabled = false;
    s.execute("comp.new", json!({"name":"Scale Controls","width":64,"height":64,"duration":2})).unwrap();
    let source = s.active_comp_id().unwrap();
    let source_layer = s.execute("layer.newSolid", json!({"name":"Card","color":"#d85f32"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("prop.set", json!({"layer":source_layer,"path":"transform/scale","value":[100,50,75]})).unwrap();
    if three_d {
        s.execute("layer.setSwitch", json!({"layer":source_layer,"switch":"threeD","value":true})).unwrap();
    }
    let control =
        s.execute("essential.addProperty", json!({"layer":source_layer,"path":"transform/scale","name":"Scale"})).unwrap()["controls"][0].as_u64().unwrap();
    let instance = if with_instance {
        s.execute("comp.new", json!({"name":"Main","width":64,"height":64,"duration":2})).unwrap();
        let layer = s.execute("layer.addItem", json!({"item":source.0})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.select", json!({"layers":[layer]})).unwrap();
        Some(layer)
    } else {
        None
    };
    let mut app = EffectcraftApp::new(s);
    app.show_panel(PanelKind::EssentialGraphics);
    (app, ScaleFixture { source, source_layer, control, instance })
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn type_into(h: &mut Harness<'_, EffectcraftApp>, id: &str, text: &str) {
    let at = rect(h, id).center();
    click(h, at);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(text.into()));
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(3);
}

fn source_value(h: &Harness<'_, EffectcraftApp>, fixture: &ScaleFixture) -> [f64; 3] {
    h.state().session.project.comp(fixture.source).unwrap().layer(LayerId(fixture.source_layer)).unwrap().props.prop("transform/scale").unwrap().value.as_vec3()
}

fn instance_value(h: &mut Harness<'_, EffectcraftApp>, fixture: &ScaleFixture) -> Value {
    let values = h.state_mut().session.execute("essential.instance", json!({"layer":fixture.instance.unwrap()})).unwrap();
    values["controls"].as_array().unwrap().iter().find(|c| c["control"] == fixture.control).unwrap()["value"].clone()
}

fn linked(h: &mut Harness<'_, EffectcraftApp>, fixture: &ScaleFixture) -> bool {
    let controls = h.state_mut().session.execute("essential.list", json!({"comp":fixture.source.0})).unwrap();
    controls["controls"].as_array().unwrap().iter().find(|c| c["id"] == fixture.control).unwrap()["scaleLinked"]
        .as_bool()
        .expect("new Scale control exposes its proportions switch")
}

fn assert_axes(h: &Harness<'_, EffectcraftApp>, base: &str, three_d: bool) {
    let panel = rect(h, "panel.EssentialGraphics");
    for axis in 0..if three_d { 3 } else { 2 } {
        let id = format!("{base}.{axis}");
        let field = rect(h, &id);
        assert!(panel.contains_rect(field), "{id} stays inside its displayed panel: {field:?} in {panel:?}");
        assert_eq!(h.state().auto.find(&id).unwrap().label, ["Scale X", "Scale Y", "Scale Z"][axis]);
    }
    assert!(panel.contains_rect(rect(h, &format!("{base}.constrain"))), "proportions switch stays inside its displayed panel");
    if !three_d {
        assert!(h.state().auto.find(&format!("{base}.2")).is_none(), "2D controls hide the Z field");
    }
}

fn check_source(three_d: bool) {
    let (app, fixture) = app(three_d, false);
    let mut h = Harness::builder().with_size(vec2(1700.0, 1100.0)).build_eframe(|_| app);
    h.run_steps(6);
    let base = format!("essential.control.{}.value", fixture.control);
    assert_axes(&h, &base, three_d);
    assert!(linked(&mut h, &fixture));
    let axis = if three_d { 2 } else { 0 };
    type_into(&mut h, &format!("{base}.{axis}"), if three_d { "150" } else { "200" });
    let constrained = if three_d { [200.0, 100.0, 150.0] } else { [200.0, 100.0, 75.0] };
    assert_eq!(source_value(&h, &fixture), constrained, "linked percentages retain the original axis ratios and hidden 2D Z");
    let at = rect(&h, &format!("{base}.constrain")).center();
    click(&mut h, at);
    assert!(!linked(&mut h, &fixture));
    type_into(&mut h, &format!("{base}.1"), "25");
    assert_eq!(source_value(&h, &fixture), [200.0, 25.0, constrained[2]], "unlinked editing changes only the typed axis");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(source_value(&h, &fixture), constrained);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert!(linked(&mut h, &fixture), "the proportions switch is undoable independently");
}

fn check_instance(three_d: bool) {
    let (app, fixture) = app(three_d, true);
    let mut h = Harness::builder().with_size(vec2(1700.0, 1100.0)).build_eframe(|_| app);
    h.run_steps(6);
    let base = format!("essential.instance.{}.value", fixture.control);
    assert_axes(&h, &base, three_d);
    assert!(linked(&mut h, &fixture));
    let axis = if three_d { 2 } else { 0 };
    type_into(&mut h, &format!("{base}.{axis}"), if three_d { "150" } else { "200" });
    let constrained = if three_d { json!([200.0, 100.0, 150.0]) } else { json!([200.0, 100.0, 75.0]) };
    assert_eq!(instance_value(&mut h, &fixture), constrained, "instance percentages use the source layer's dimensions and ratios");
    assert_eq!(source_value(&h, &fixture), [100.0, 50.0, 75.0], "the instance edit leaves its source unchanged");
    let at = rect(&h, &format!("{base}.constrain")).center();
    click(&mut h, at);
    assert!(!linked(&mut h, &fixture));
    type_into(&mut h, &format!("{base}.1"), "25");
    let independent = if three_d { json!([200.0, 25.0, 150.0]) } else { json!([200.0, 25.0, 75.0]) };
    assert_eq!(instance_value(&mut h, &fixture), independent);
    assert_eq!(source_value(&h, &fixture), [100.0, 50.0, 75.0]);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(instance_value(&mut h, &fixture), constrained);
}

#[test]
fn essential_scale_source_2d_keeps_ratios_and_hidden_z_until_unlinked() {
    check_source(false);
}

#[test]
fn essential_scale_source_3d_edits_all_percentage_axes() {
    check_source(true);
}

#[test]
fn essential_scale_instance_2d_keeps_ratios_and_hidden_z_until_unlinked() {
    check_instance(false);
}

#[test]
fn essential_scale_instance_3d_uses_its_source_layer_dimensions() {
    check_instance(true);
}
