use crate::{Comp, Expression, ItemId, ItemKind, Keyframe, LayerId, LayerSource, Project, Value, build};
use effectcraft_time::{FrameRate, Tick};
fn setup() -> (Project, ItemId, LayerId) {
    let mut p = Project::default();
    let comp = Comp::new(64, 64, FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
    let cid = p.add_item("Main", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let layer = build::layer(&mut p, &comp, "Shape", LayerSource::Shape, (64, 64), None);
    let lid = layer.id;
    p.comp_mut(cid).unwrap().layers.push(layer);
    (p, cid, lid)
}
fn remove(p: &mut Project, cid: ItemId, lid: LayerId, ids: &[&str]) {
    p.comp_mut(cid).unwrap().layer_mut(lid).unwrap().props.sub_mut("transform").unwrap().children.retain(|node| !ids.contains(&node.match_id()));
}
#[test]
fn legacy_layer_skew_migration_preserves_numeric_paths_uids_and_is_idempotent() {
    let (mut old, cid, lid) = setup();
    remove(&mut old, cid, lid, &["skew", "skewAxis"]);
    let tr = old.comp(cid).unwrap().layer(lid).unwrap().transform().unwrap().clone();
    let next = old.next_id;
    let loaded = Project::from_json(&old.to_json()).unwrap();
    let new = loaded.comp(cid).unwrap().layer(lid).unwrap().transform().unwrap();
    assert_eq!(new.uid, tr.uid);
    assert_eq!(&new.children[..tr.children.len()], &tr.children);
    for id in ["skew", "skewAxis"] {
        let prop = new.get(id).unwrap();
        assert_eq!(prop.value, Value::Scalar(0.0));
        assert!(prop.two_d_only);
        assert!(prop.uid >= next);
    }
    assert_eq!(new.prop("#7").unwrap().match_id, "rotation");
    assert_eq!(loaded.next_id, next + 2);
    assert_eq!(Project::from_json(&loaded.to_json()).unwrap(), loaded);
}
#[test]
fn partial_layer_skew_migration_preserves_authored_value_keys_expression_and_uid() {
    let (mut old, cid, lid) = setup();
    remove(&mut old, cid, lid, &["skewAxis"]);
    let prop = old.comp_mut(cid).unwrap().layer_mut(lid).unwrap().props.prop_mut("transform/skew").unwrap();
    prop.value = Value::Scalar(12.0);
    prop.keys.push(Keyframe::new(Tick::ZERO, Value::Scalar(12.0)));
    prop.expr = Some(Expression { text: "value+2".into(), enabled: true });
    let authored = prop.clone();
    let next = old.next_id;
    let loaded = Project::from_json(&old.to_json()).unwrap();
    let tr = loaded.comp(cid).unwrap().layer(lid).unwrap().transform().unwrap();
    assert_eq!(tr.get("skew").unwrap(), &authored);
    assert_eq!(tr.get("skewAxis").unwrap().value, Value::Scalar(0.0));
    assert_eq!(loaded.next_id, next + 1);
}
#[test]
fn legacy_layer_skew_migration_rejects_exhausted_uid_range() {
    let (mut old, cid, lid) = setup();
    remove(&mut old, cid, lid, &["skew", "skewAxis"]);
    old.next_id = u64::MAX - 1;
    let error = Project::from_json(&old.to_json()).unwrap_err();
    assert!(error.to_string().contains("not enough property identifiers"));
}
#[test]
fn cameras_and_lights_do_not_gain_layer_skew_during_load() {
    let (mut p, cid, _) = setup();
    let comp = p.comp(cid).unwrap().clone();
    for source in [LayerSource::Camera, LayerSource::Light { kind: crate::LightKind::Spot }] {
        let layer = build::layer(&mut p, &comp, "Rig", source, (64, 64), None);
        assert!(layer.transform().unwrap().get("skew").is_none());
        p.comp_mut(cid).unwrap().layers.push(layer);
    }
    let loaded = Project::from_json(&p.to_json()).unwrap();
    for l in &loaded.comp(cid).unwrap().layers {
        if l.is_camera() || l.is_light() {
            assert!(l.transform().unwrap().get("skew").is_none());
            assert!(l.transform().unwrap().get("skewAxis").is_none());
        }
    }
}

#[test]
fn exhausted_existing_property_layer_and_item_ids_are_rejected_without_remapping() {
    let (mut p, cid, lid) = setup();
    remove(&mut p, cid, lid, &["skew", "skewAxis"]);
    p.next_id = 1;
    let mut property = p.clone();
    property.comp_mut(cid).unwrap().layer_mut(lid).unwrap().props.prop_mut("transform/anchor").unwrap().uid = u64::MAX;
    let mut layer = p.clone();
    layer.comp_mut(cid).unwrap().layer_mut(lid).unwrap().id = LayerId(u64::MAX);
    let mut item = p;
    let authored = item.items.remove(&cid).unwrap();
    item.items.insert(ItemId(u64::MAX), authored);
    for exhausted in [property, layer, item] {
        let error = Project::from_json(&exhausted.to_json()).unwrap_err();
        assert!(error.to_string().contains("exhausted its identifier range"));
        let mut infallible_repair = exhausted.clone();
        infallible_repair.fix_next_id();
        assert_eq!(infallible_repair.next_id, u64::MAX);
        assert_eq!(infallible_repair.items, exhausted.items, "repair never silently remaps authored identifiers");
    }
}

#[test]
fn legacy_layer_skew_migration_preserves_an_allocatable_successor() {
    let (mut old, cid, lid) = setup();
    remove(&mut old, cid, lid, &["skew", "skewAxis"]);
    old.next_id = u64::MAX - 3;
    let loaded = Project::from_json(&old.to_json()).unwrap();
    let tr = loaded.comp(cid).unwrap().layer(lid).unwrap().transform().unwrap();
    assert_eq!(tr.get("skew").unwrap().uid, u64::MAX - 3);
    assert_eq!(tr.get("skewAxis").unwrap().uid, u64::MAX - 2);
    assert_eq!(loaded.next_id, u64::MAX - 1);
    assert_eq!(Project::from_json(&loaded.to_json()).unwrap(), loaded);
}

#[test]
fn legacy_layer_skew_migration_rejects_consuming_the_last_successor() {
    let (mut old, cid, lid) = setup();
    remove(&mut old, cid, lid, &["skew", "skewAxis"]);
    old.next_id = u64::MAX - 2;
    let loaded = Project::from_json(&old.to_json());
    eprintln!("SKEW_MIGRATION_SUCCESSOR returned floor={:?}", loaded.as_ref().map(|p| p.next_id));
    assert!(loaded.is_err(), "SKEW_MIGRATION_SUCCESSOR: migration must not consume the last allocatable successor");
    assert!(loaded.unwrap_err().to_string().contains("not enough property identifiers"));
}
