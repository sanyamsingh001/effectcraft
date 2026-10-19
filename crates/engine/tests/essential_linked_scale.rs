//! Public-command proof that implicitly re-exposing a linked Scale target keeps its existing driver.
use effectcraft_engine::{
    Session,
    project::{LayerId, essential::EgKind},
};
use serde_json::json;

#[test]
fn implicit_scale_reexposing_a_linked_target_creates_a_mirror_of_its_driver() {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"name":"Source","width":128,"height":128,"duration":2,"frameRate":30})).unwrap();
    let source = session.active_comp_id().unwrap();
    let first = session.execute_checked("layer.newSolid", json!({"name":"First","width":32,"height":32})).unwrap()["layer"].as_u64().unwrap();
    let second = session.execute_checked("layer.newSolid", json!({"name":"Second","width":32,"height":32})).unwrap()["layer"].as_u64().unwrap();
    session.execute_checked("prop.set", json!({"layer":first,"path":"transform/scale","value":[100,50,30]})).unwrap();
    let driver = session.execute_checked("essential.addProperty", json!({"layer":first,"path":"transform/scale","name":"Scale"})).unwrap()["controls"][0]
        .as_u64()
        .unwrap();
    session.execute_checked("essential.linkProperty", json!({"control":driver,"layer":second,"path":"transform/scale"})).unwrap();
    let second_uid = session.project.comp(source).unwrap().layer(LayerId(second)).unwrap().props.prop("transform/scale").unwrap().uid;
    let essential = session.project.comp(source).unwrap().essential.as_ref().unwrap();
    assert_eq!(essential.driver_of(LayerId(second), second_uid).unwrap().id, driver, "the linked target already has a driver");
    assert_eq!(session.project.comp(source).unwrap().layer(LayerId(second)).unwrap().props.find(second_uid).unwrap().value.as_vec3(), [100.0, 50.0, 30.0]);
    let before = session.project.clone();
    let history = session.history.undo.len();
    let added = session.execute_checked("essential.addProperty", json!({"layer":second,"path":"transform/scale","name":"Linked Scale"})).unwrap();
    let mirror = added["controls"][0].as_u64().unwrap();
    let controls = session.project.comp(source).unwrap().essential.as_ref().unwrap();
    eprintln!("driver={driver}; added={added}; added-kind={:?}", controls.find(mirror).unwrap().kind);
    assert!(
        matches!(controls.find(mirror).unwrap().kind, EgKind::Mirror{of} if of == driver),
        "implicit Add Property for an already-linked target must mirror its existing control, preserving one shared override"
    );
    assert_eq!(added["mirrors"], json!([mirror]));
    assert_eq!(controls.flat().iter().filter(|control| matches!(control.kind, EgKind::Property { .. })).count(), 1);
    assert_eq!(session.history.undo.len(), history + 1);
    assert!(session.undo());
    assert!(std::sync::Arc::ptr_eq(&session.project, &before));
    assert!(session.redo());

    session.execute_checked("comp.new", json!({"name":"Main","width":128,"height":128,"duration":2,"frameRate":30})).unwrap();
    let instance = session.execute_checked("layer.addItem", json!({"item":source.0})).unwrap()["layer"].as_u64().unwrap();
    let source_before = session.project.comp(source).unwrap().clone();
    for (control, value) in [(driver, json!([200, 100, 30])), (mirror, json!([150, 75, 30]))] {
        session.execute_checked("essential.set", json!({"layer":instance,"control":control,"value":value})).unwrap();
        let rows = session.execute_checked("essential.instance", json!({"layer":instance})).unwrap();
        for id in [driver, mirror] {
            let row = rows["controls"].as_array().unwrap().iter().find(|row| row["control"] == id).unwrap();
            let actual = row["value"].as_array().unwrap();
            let expected = value.as_array().unwrap();
            assert_eq!(actual.len(), 3);
            assert_eq!(expected.len(), 3);
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.as_f64().unwrap(), expected.as_f64().unwrap(), "editing either mirror keeps the one instance value shared");
            }
        }
        assert!(session.project.comp(source).unwrap() == &source_before, "an instance edit does not modify the source");
    }
}

#[test]
fn implicit_scale_readding_its_original_driver_still_creates_a_mirror() {
    let mut session = Session::default();
    session.execute_checked("comp.new", json!({"width":64,"height":64,"duration":2})).unwrap();
    let layer = session.execute_checked("layer.newSolid", json!({"name":"Box"})).unwrap()["layer"].as_u64().unwrap();
    let first = session.execute_checked("essential.addProperty", json!({"layer":layer,"path":"transform/scale"})).unwrap()["controls"][0].as_u64().unwrap();
    let repeated = session.execute_checked("essential.addProperty", json!({"layer":layer,"path":"transform/scale"})).unwrap();
    let second = repeated["controls"][0].as_u64().unwrap();
    let controls = session.active_comp().unwrap().essential.as_ref().unwrap();
    assert!(matches!(controls.find(second).unwrap().kind,EgKind::Mirror{of} if of == first));
    assert_eq!(controls.resolve(second).unwrap().scale_linked, Some(true));
    assert_eq!(repeated["mirrors"], json!([second]));
}
#[test]
fn repeated_explicit_point_and_uniform_scale_controls_keep_their_legacy_representation() {
    for requested in ["point", "scale"] {
        let mut session = Session::default();
        session.execute_checked("comp.new", json!({"width":64,"height":64,"duration":2})).unwrap();
        let layer = session.execute_checked("layer.newSolid", json!({"name":"Box"})).unwrap()["layer"].as_u64().unwrap();
        let first = session.execute_checked("essential.addProperty", json!({"layer":layer,"path":"transform/scale","as":requested})).unwrap()["controls"][0]
            .as_u64()
            .unwrap();
        let repeated = session.execute_checked("essential.addProperty", json!({"layer":layer,"path":"transform/scale","as":requested})).unwrap();
        let second = repeated["controls"][0].as_u64().unwrap();
        let rows = session.execute_checked("essential.list", json!({})).unwrap();
        for id in [first, second] {
            let row = rows["controls"].as_array().unwrap().iter().find(|row| row["id"] == id).unwrap();
            assert_eq!(row["type"], requested);
            assert!(row.get("scaleLinked").is_none(), "explicit legacy editors do not gain proportional component metadata");
        }
        let controls = session.active_comp().unwrap().essential.as_ref().unwrap();
        assert!(matches!(controls.find(second).unwrap().kind,EgKind::Mirror{of} if of == first));
    }
}
