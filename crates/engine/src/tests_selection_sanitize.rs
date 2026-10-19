//! Undo/redo must drop references to missing properties, keys and path vertices.

use effectcraft_project::LayerId;
use effectcraft_time::Tick;
use serde_json::json;

use crate::{
    KeyRef, VertexRef,
    tests_timeline::{prop, setup},
};

#[test]
fn undoing_a_key_move_drops_the_missing_key_and_keeps_other_selections() {
    let (mut s, layer) = setup();
    for (path, time) in [("transform/opacity", 0.0), ("transform/opacity", 1.0), ("transform/rotation", 0.0)] {
        s.execute("prop.addKey", json!({"layer": layer, "path": path, "time": time, "value": 20})).unwrap();
    }
    s.execute(
        "keys.select",
        json!({"keys": [{"layer": layer, "path": "transform/opacity", "time": 1}, {"layer": layer, "path": "transform/rotation", "time": 0}]}),
    )
    .unwrap();
    let rotation = s.state.selected_keys[1];
    let layers = s.state.selected_layers.clone();
    s.execute("keys.set", json!({"layer": layer, "path": "transform/opacity", "time": 1, "newTime": 2})).unwrap();
    assert_eq!(s.state.selected_keys[0].time, Tick::from_seconds_f64(2.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.state.selected_keys, vec![rotation]);
    assert_eq!(s.state.selected_layers, layers);
    assert!(prop(&s, layer, "transform/opacity").keys.iter().any(|k| k.time == Tick::from_seconds_f64(1.0)));
    s.execute("keys.easyEase", json!({})).unwrap();
    assert_eq!(prop(&s, layer, "transform/rotation").keys[0].in_interp, effectcraft_keyframe::Interp::Bezier);
    assert_eq!(
        prop(&s, layer, "transform/opacity").keys[1].in_interp,
        effectcraft_keyframe::Interp::Linear,
        "undo must not silently select the key now occupying an old index"
    );
}

#[test]
fn undoing_an_added_effect_drops_its_group_and_parameters_only() {
    let (mut s, layer) = setup();
    let lid = LayerId(layer);
    let opacity = prop(&s, layer, "transform/opacity").uid;
    s.execute("effect.apply", json!({"layer": layer, "effect": "Gaussian Blur"})).unwrap();
    let ly = s.active_comp().unwrap().layer(lid).unwrap();
    let group = ly.props.group("effects").unwrap().children[0].uid();
    let blur = ly.props.prop("effects/#1/blurriness").unwrap().uid;
    s.state.selected_props = vec![(lid, group), (lid, blur), (lid, opacity)];
    s.sanitize_state();
    assert_eq!(s.state.selected_props.len(), 3, "existing group selections remain valid");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.state.selected_props, vec![(lid, opacity)]);
    assert_eq!(s.state.selected_layers, vec![lid]);
    s.execute("edit.redo", json!({})).unwrap();
    assert!(s.active_comp().unwrap().layer(lid).unwrap().props.find_group(group).is_some());
    assert_eq!(s.state.selected_props, vec![(lid, opacity)]);
}

#[test]
fn undoing_an_added_mask_drops_missing_group_keys_and_vertices() {
    let (mut s, layer) = setup();
    let lid = LayerId(layer);
    let mask = |s: &mut crate::Session| {
        s.execute("mask.new", json!({"layer": layer, "vertices": [[10, 10], [50, 10], [50, 50]], "closed": true})).unwrap()["mask"].as_u64().unwrap()
    };
    let first = mask(&mut s);
    let second = mask(&mut s);
    let opacity = prop(&s, layer, "transform/opacity").uid;
    s.state.selected_props = vec![(lid, first), (lid, second), (lid, opacity)];
    s.state.selected_vertices = vec![VertexRef { layer: lid, mask: first, index: 1 }, VertexRef { layer: lid, mask: second, index: 0 }];
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.state.selected_props, vec![(lid, first), (lid, opacity)]);
    assert_eq!(s.state.selected_vertices, vec![VertexRef { layer: lid, mask: first, index: 1 }]);
    // A corrupt/restored selection can also point past the end of a surviving mask.
    s.state.selected_vertices.push(VertexRef { layer: lid, mask: first, index: usize::MAX });
    s.sanitize_state();
    assert_eq!(s.state.selected_vertices.len(), 1);
}

#[test]
fn sanitizing_missing_property_keys_and_no_active_comp_clears_invalid_refs() {
    let (mut s, layer) = setup();
    let lid = LayerId(layer);
    s.state.selected_props = vec![(lid, u64::MAX)];
    s.state.selected_keys = vec![KeyRef { layer: lid, prop: u64::MAX, time: Tick::ZERO }];
    s.sanitize_state();
    assert!(s.state.selected_props.is_empty());
    assert!(s.state.selected_keys.is_empty());
    s.state.selected_props = vec![(lid, prop(&s, layer, "transform/opacity").uid)];
    s.state.active_comp = None;
    s.sanitize_state();
    assert!(s.state.selected_layers.is_empty());
    assert!(s.state.selected_props.is_empty());
}
