//! New implicit Transform Scale controls preserve proportional editing metadata.
use crate::{
    Session,
    project::{ItemId, LayerId, Project},
};
use serde_json::{Value, json};
use std::sync::Arc;
fn setup(three_d: bool) -> (Session, ItemId, u64) {
    let mut s = Session::default();
    s.execute_checked("comp.new", json!({"name":"Source","width":64,"height":64,"frameRate":30,"duration":4})).unwrap();
    let comp = s.active_comp_id().unwrap();
    let layer = s.execute_checked("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    if three_d {
        s.execute_checked("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
    }
    s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/scale","value":[100,50,100]})).unwrap();
    (s, comp, layer)
}
fn add(s: &mut Session, layer: u64, path: &str, as_type: Option<&str>) -> u64 {
    let mut q = json!({"layer":layer,"path":path});
    if let Some(a) = as_type {
        q["as"] = json!(a);
    }
    s.execute_checked("essential.addProperty", q).unwrap()["controls"][0].as_u64().unwrap()
}
fn listed(s: &mut Session, comp: ItemId, id: u64) -> Value {
    s.execute_checked("essential.list", json!({"comp":comp.0})).unwrap()["controls"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone()
}
#[test]
fn new_implicit_2d_transform_scale_defaults_to_linked_scale() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", None);
    let row = listed(&mut s, comp, id);
    assert_eq!((row["type"].clone(), row["scaleLinked"].clone()), (json!("scale"), json!(true)), "new implicit Scale has proportional mode by default");
    assert_eq!(row["value"], json!([100.0, 50.0, 100.0]));
}
#[test]
fn new_implicit_3d_transform_scale_defaults_to_linked_scale() {
    let (mut s, comp, layer) = setup(true);
    let id = add(&mut s, layer, "transform/scale", None);
    let row = listed(&mut s, comp, id);
    assert_eq!((row["type"].clone(), row["scaleLinked"].clone()), (json!("scale"), json!(true)), "3D Scale has the same proportional default");
}
#[test]
fn linked_mode_toggle_resolves_mirrors_changes_only_metadata_and_has_one_undo() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", None);
    let mirror = s.execute_checked("essential.addMirror", json!({"comp":comp.0,"control":id})).unwrap()["control"].as_u64().unwrap();
    let value = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/scale").unwrap().value.clone();
    let before = s.project.clone();
    s.history.clear();
    s.execute_checked("essential.setScaleLinked", json!({"comp":comp.0,"control":mirror,"linked":false})).unwrap();
    assert_eq!(listed(&mut s, comp, id)["scaleLinked"], false);
    assert_eq!(listed(&mut s, comp, mirror)["scaleLinked"], false);
    assert_eq!(s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.prop("transform/scale").unwrap().value, value);
    assert_eq!(s.history.undo.len(), 1);
    assert!(s.undo());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert_eq!(listed(&mut s, comp, mirror)["scaleLinked"], true);
    let encoded = s.project.to_json();
    let back = Project::from_json(&encoded).unwrap();
    assert_eq!(serde_json::to_value(back.comp(comp).unwrap().essential.as_ref().unwrap().find(id).unwrap()).unwrap()["scaleLinked"], true);
}
fn instance(s: &mut Session, source: ItemId) -> u64 {
    s.execute_checked("comp.new", json!({"name":"Main","width":64,"height":64,"frameRate":30,"duration":4})).unwrap();
    s.execute_checked("layer.addItem", json!({"item":source.0})).unwrap()["layer"].as_u64().unwrap()
}
#[test]
fn instance_and_mirror_report_the_master_scale_mode() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", None);
    let mirror = s.execute_checked("essential.addMirror", json!({"comp":comp.0,"control":id})).unwrap()["control"].as_u64().unwrap();
    let instance = instance(&mut s, comp);
    let rows = s.execute_checked("essential.instance", json!({"layer":instance})).unwrap();
    for control in [id, mirror] {
        let row = rows["controls"].as_array().unwrap().iter().find(|c| c["control"] == control).unwrap();
        assert_eq!((row["type"].clone(), row["scaleLinked"].clone()), (json!("scale"), json!(true)), "instance mirrors inherit the source mode");
    }
}
#[test]
fn point_properties_and_explicit_point_scale_keep_their_previous_types() {
    let (mut s, comp, layer) = setup(false);
    for (path, as_type) in [("transform/position", None), ("transform/scale", Some("point"))] {
        let id = add(&mut s, layer, path, as_type);
        let row = listed(&mut s, comp, id);
        assert_eq!(row["type"], "point");
        assert!(row.get("scaleLinked").is_none());
    }
}
#[test]
fn explicit_uniform_scale_keeps_legacy_metadata_and_rejects_the_new_mode_toggle() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", Some("scale"));
    let row = listed(&mut s, comp, id);
    assert_eq!(row["type"], "scale");
    assert!(row.get("scaleLinked").is_none());
    let before = s.project.clone();
    s.history.clear();
    assert!(s.execute_checked("essential.setScaleLinked", json!({"comp":comp.0,"control":id,"linked":false})).is_err());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(s.history.undo.is_empty());
}
#[test]
fn explicit_scalar_and_array_override_api_remains_unchanged() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", Some("scale"));
    let instance = instance(&mut s, comp);
    for (input, expected) in [(json!(200), json!([200.0, 200.0, 200.0])), (json!([150, 75, 100]), json!([150.0, 75.0, 100.0]))] {
        s.execute_checked("essential.set", json!({"layer":instance,"control":id,"value":input})).unwrap();
        let rows = s.execute_checked("essential.instance", json!({"layer":instance})).unwrap();
        assert_eq!(rows["controls"][0]["value"], expected);
    }
}
#[test]
fn legacy_saved_controls_omit_scale_mode_and_round_trip() {
    let (mut s, comp, layer) = setup(false);
    let id = add(&mut s, layer, "transform/scale", Some("point"));
    let ctl = s.project.comp(comp).unwrap().essential.as_ref().unwrap().find(id).unwrap();
    let old = serde_json::to_value(ctl).unwrap();
    assert!(old.get("scaleLinked").is_none());
    let back = Project::from_json(&s.project.to_json()).unwrap();
    assert_eq!(serde_json::to_value(back.comp(comp).unwrap().essential.as_ref().unwrap().find(id).unwrap()).unwrap(), old);
}

#[test]
fn new_scale_mode_preserves_explicit_vector_and_scalar_override_inputs() {
    for (three_d, vector) in [(false, json!([200, 100, 30])), (true, json!([200, 100, 60]))] {
        let (mut s, source, layer) = setup(three_d);
        s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/scale","value":[100,50,30]})).unwrap();
        let control = add(&mut s, layer, "transform/scale", None);
        let instance = instance(&mut s, source);
        assert!(!s.active_comp().unwrap().layer(LayerId(instance)).unwrap().is_3d(), "the containing precomp is 2D even when the source control is 3D");
        s.execute_checked("essential.set", json!({"layer":instance,"control":control,"value":vector})).unwrap();
        let values = s.execute_checked("essential.instance", json!({"layer":instance})).unwrap();
        let expected = if three_d { json!([200.0, 100.0, 60.0]) } else { json!([200.0, 100.0, 30.0]) };
        assert_eq!(values["controls"][0]["value"], expected);
        s.execute_checked("essential.set", json!({"layer":instance,"control":control,"value":250})).unwrap();
        assert_eq!(
            s.execute_checked("essential.instance", json!({"layer":instance})).unwrap()["controls"][0]["value"],
            json!([250.0, 250.0, 250.0]),
            "scalar API inputs still set every axis"
        );
    }
}

#[test]
fn malformed_scale_mode_toggle_does_not_change_the_project_or_history() {
    let (mut s, comp, layer) = setup(false);
    let control = add(&mut s, layer, "transform/scale", None);
    let before = s.project.clone();
    s.history.clear();
    assert!(s.execute_checked("essential.setScaleLinked", json!({"comp":comp.0,"control":control,"linked":"false"})).is_err());
    assert!(Arc::ptr_eq(&s.project, &before));
    assert!(s.history.undo.is_empty());
}
