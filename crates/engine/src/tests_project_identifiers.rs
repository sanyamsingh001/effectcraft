//! Project loading preserves authored IDs and leaves a usable allocation counter.
use crate::{
    Session,
    project::{ItemKind, Project, essential::EgKind},
};
use serde_json::json;
use std::sync::Arc;
fn fixture(nested: bool) -> (Session, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":2})).unwrap();
    let group = s.execute("essential.addGroup", json!({"name":"Notes"})).unwrap()["control"].as_u64().unwrap();
    s.execute("layer.newSolid", json!({"width":16,"height":8,"color":"#00ff00"})).unwrap();
    let params = if nested { json!({"text":"Saved","group":group}) } else { json!({"text":"Saved"}) };
    let id = s.execute("essential.addComment", params).unwrap()["control"].as_u64().unwrap();
    (s, id)
}
fn max_non_control_id(p: &Project) -> u64 {
    p.items
        .iter()
        .map(|(id, item)| {
            let mut max = id.0;
            if let ItemKind::Comp(c) = &item.kind {
                for l in &c.layers {
                    max = max.max(l.id.0).max(l.props.max_uid());
                }
            }
            max
        })
        .max()
        .unwrap_or(0)
}
fn control_ids(controls: &[crate::project::essential::EgControl]) -> Vec<u64> {
    let mut ids = Vec::new();
    let mut groups = vec![controls.iter()];
    while let Some(group) = groups.last_mut() {
        if let Some(control) = group.next() {
            ids.push(control.id);
            if let EgKind::Group { children } = &control.kind {
                groups.push(children.iter());
            }
        } else {
            groups.pop();
        }
    }
    ids
}
fn stale_control_floor(nested: bool) {
    let (mut s, id) = fixture(nested);
    let cid = s.active_comp_id().unwrap();
    let mut p = (*s.project).clone();
    let target = max_non_control_id(&p) + 1;
    p.comp_mut(cid).unwrap().essential.as_mut().unwrap().find_mut(id).unwrap().id = target;
    let existing = control_ids(&p.comp(cid).unwrap().essential.as_ref().unwrap().controls);
    assert_eq!(existing.iter().filter(|&&id| id == target).count(), 1, "fixture must not contain preexisting duplicate IDs");
    p.next_id = 1;
    let loaded = Project::from_json(&p.to_json()).unwrap();
    let loaded_floor = loaded.next_id;
    let loaded_ids = control_ids(&loaded.comp(cid).unwrap().essential.as_ref().unwrap().controls);
    assert_eq!(loaded_ids, existing, "loading may never remap saved controls");
    s.project = Arc::new(loaded);
    let added = s.execute("essential.addComment", json!({"text":"Added"}));
    eprintln!("ESSENTIAL_ID_FLOOR actual loaded next_id={loaded_floor}; existing={existing:?}; add result={added:?}");
    let added = added.unwrap()["control"].as_u64().unwrap();
    let after = control_ids(&s.active_comp().unwrap().essential.as_ref().unwrap().controls);
    eprintln!("ESSENTIAL_ID_FLOOR after IDs={after:?}");
    assert_ne!(added, target, "ESSENTIAL_ID_FLOOR: next control must not reuse a preserved saved control ID");
    assert_eq!(after.iter().filter(|&&id| id == target).count(), 1);
}
#[test]
fn stale_top_level_essential_control_id_is_not_reallocated() {
    stale_control_floor(false);
}
#[test]
fn stale_nested_essential_control_id_is_not_reallocated() {
    stale_control_floor(true);
}
#[test]
fn exhausted_next_id_counter_is_rejected_without_allocating() {
    let (s, _) = fixture(false);
    let mut p = (*s.project).clone();
    p.next_id = u64::MAX;
    let loaded = Project::from_json(&p.to_json());
    eprintln!("EXHAUSTED_NEXT_ID actual returned floor={:?}", loaded.as_ref().map(|p| p.next_id));
    assert!(loaded.is_err(), "EXHAUSTED_NEXT_ID: a file with no allocatable successor must be rejected without executing alloc");
}
#[test]
fn maximum_minus_one_property_uid_rejects_the_exhausted_successor() {
    let (s, _) = fixture(false);
    let mut p = (*s.project).clone();
    let cid = s.active_comp_id().unwrap();
    let layer = p.comp_mut(cid).unwrap().layers.first_mut().unwrap();
    layer.props.prop_mut("transform/anchor").unwrap().uid = u64::MAX - 1;
    p.next_id = 1;
    let loaded = Project::from_json(&p.to_json());
    eprintln!("EXHAUSTED_NEXT_ID existing UIDMAX-1 actual returned floor={:?}", loaded.as_ref().map(|p| p.next_id));
    assert!(loaded.is_err(), "EXHAUSTED_NEXT_ID: a repaired counter with no allocatable successor must be rejected without executing alloc");
}
#[test]
fn ordinary_stale_id_floor_keeps_authored_items_and_controls_absent() {
    let (s, _) = fixture(false);
    let mut p = (*s.project).clone();
    let cid = s.active_comp_id().unwrap();
    p.comp_mut(cid).unwrap().essential = None;
    p.next_id = 1;
    let expected = max_non_control_id(&p) + 1;
    let loaded = Project::from_json(&p.to_json()).unwrap();
    eprintln!("ORDINARY_ID_FLOOR actual={} expected={expected}", loaded.next_id);
    assert_eq!(loaded.next_id, expected);
    assert_eq!(loaded.items, p.items, "ordinary loading preserves authored items and property IDs");
}
