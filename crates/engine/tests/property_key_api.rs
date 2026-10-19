use effectcraft_engine::{Session, project::LayerId};
use serde_json::{Value, json};

fn setup(three: bool) -> (Session, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":100,"height":80,"duration":4,"frameRate":30})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
    if three {
        s.execute("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
    }
    (s, layer)
}
fn position(s: &mut Session, layer: u64) -> Value {
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let layer = comp.layer(LayerId(layer)).unwrap();
    let prop = layer.transform().unwrap().get("position").unwrap();
    let context = effectcraft_engine::render::EvalCtx::new(&s.project, cid, comp, effectcraft_engine::time::Tick::from_seconds_f64(1.0));
    context.value(layer, prop).to_json()
}
#[test]
fn normal_vector_keys_preserve_public_behavior() {
    for three in [false, true] {
        let (mut s, layer) = setup(three);
        let value = if three { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0, 0.0]) };
        let reply = s.execute("prop.addKey", json!({"layer":layer,"path":"transform/position","time":1,"timeBase":"comp","value":value})).unwrap();
        assert_eq!(reply["compTime"], 1.0);
        assert_eq!(reply["layerTime"], 1.0);
        assert_eq!(position(&mut s, layer), value);
    }
}
#[test]
fn separated_position_key_updates_the_axes_used_by_evaluation() {
    for three in [false, true] {
        let (mut s, layer) = setup(three);
        s.execute("prop.separateDimensions", json!({"layer":layer})).unwrap();
        s.history.clear();
        let value = if three { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0, 0.0]) };
        s.execute("prop.addKey", json!({"layer":layer,"path":"transform/position","time":1,"timeBase":"comp","value":value})).unwrap();
        assert_eq!(position(&mut s, layer), value, "separated Position key must update evaluated axes");
        for path in ["transform/positionX", "transform/positionY"].into_iter().chain(three.then_some("transform/positionZ")) {
            let info = s.execute("prop.get", json!({"layer":layer,"path":path,"time":1})).unwrap();
            assert_eq!(info["keys"].as_array().unwrap().len(), 1);
        }
        assert_eq!(s.history.undo.len(), 1);
    }
}
#[test]
fn static_only_pin_metadata_rejects_keys_without_project_or_history_changes() {
    let (mut s, layer) = setup(false);
    let pin = s.execute("puppet.addPin", json!({"layer":layer,"position":[10,10],"expansion":0})).unwrap()["pin"].as_u64().unwrap();
    let rest = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find_group(pin).unwrap().get("rest").unwrap();
    assert!(rest.static_only);
    let rest = rest.uid;
    s.history.clear();
    let before = s.project.to_json();
    let result = s.execute("prop.addKey", json!({"layer":layer,"prop":rest,"time":1,"timeBase":"comp","value":[11,12]}));
    assert!(result.is_err(), "static-only Puppet metadata must reject keyframes: {result:?}");
    assert_eq!(s.project.to_json(), before);
    assert!(s.history.undo.is_empty());
}

mod extra_key_api {
    use effectcraft_engine::Session;
    use serde_json::json;
    fn setup() -> (Session, u64) {
        let mut s = Session::default();
        s.execute("comp.new", json!({"width":100,"height":80,"duration":4,"frameRate":30})).unwrap();
        let layer = s.execute("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
        (s, layer)
    }
    #[test]
    fn static_only_toggle_key_rejects_without_editing_project_or_history() {
        let (mut s, layer) = setup();
        let pin = s.execute("puppet.addPin", json!({"layer":layer,"position":[10,10],"expansion":0})).unwrap()["pin"].as_u64().unwrap();
        let uid = s.active_comp().unwrap().layer(effectcraft_engine::project::LayerId(layer)).unwrap().props.find_group(pin).unwrap().get("rest").unwrap().uid;
        s.history.clear();
        let before = s.project.to_json();
        let result = s.execute("prop.toggleKey", json!({"layer":layer,"prop":uid}));
        assert!(result.is_err(), "static-only toggleKey must reject keyframes: {result:?}");
        assert_eq!(s.project.to_json(), before);
        assert!(s.history.undo.is_empty());
    }
    #[test]
    fn separated_toggle_key_operates_on_actual_axis_tracks_in_one_undo() {
        let (mut s, layer) = setup();
        s.execute("prop.separateDimensions", json!({"layer":layer})).unwrap();
        s.execute("prop.set", json!({"layer":layer,"path":"transform/position","value":[10,20]})).unwrap();
        s.history.clear();
        let before = s.project.to_json();
        s.execute("prop.toggleKey", json!({"layer":layer,"path":"transform/position"})).unwrap();
        for path in ["transform/positionX", "transform/positionY"] {
            let info = s.execute("prop.get", json!({"layer":layer,"path":path})).unwrap();
            assert_eq!(info["keys"].as_array().unwrap().len(), 1, "separated toggleKey must key the rendered axis");
        }
        assert_eq!(s.history.undo.len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.to_json(), before);
        s.execute("edit.redo", json!({})).unwrap();
        s.execute("prop.toggleKey", json!({"layer":layer,"path":"transform/position"})).unwrap();
        for path in ["transform/positionX", "transform/positionY"] {
            assert!(s.execute("prop.get", json!({"layer":layer,"path":path})).unwrap()["keys"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn separated_add_key_captures_axes_at_explicit_time_and_rejects_invalid_values_atomically() {
        let (mut s, layer) = setup();
        s.execute("prop.separateDimensions", json!({"layer":layer})).unwrap();
        s.execute("prop.set", json!({"layer":layer,"path":"transform/position","value":[10,20]})).unwrap();
        s.execute("prop.addKey", json!({"layer":layer,"path":"transform/position","time":1})).unwrap();
        let info = s.execute("prop.get", json!({"layer":layer,"path":"transform/position","time":1})).unwrap();
        assert_eq!(info["value"], json!([10.0, 20.0, 0.0]));
        assert_eq!(info["animated"], true);
        assert!(info["keys"].as_array().unwrap().is_empty());
        assert_eq!(info["keyProperties"].as_array().unwrap().len(), 2);
        for axis in info["axisProperties"].as_array().unwrap() {
            assert_eq!(axis["keys"].as_array().unwrap().len(), 1);
        }
        s.history.clear();
        let before = s.project.to_json();
        let state = serde_json::to_value(&s.state).unwrap();
        assert!(s.execute("prop.addKey", json!({"layer":layer,"path":"transform/position","time":2,"value":[40,"bad"]})).is_err());
        assert_eq!(s.project.to_json(), before);
        assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
        assert!(s.history.undo.is_empty());
        s.execute("prop.set", json!({"layer":layer,"path":"transform/position","value":[30]})).unwrap();
        let info = s.execute("prop.get", json!({"layer":layer,"path":"transform/position"})).unwrap();
        assert_eq!(info["value"], json!([30.0, 20.0, 0.0]));
    }
    #[test]
    fn static_property_values_remain_editable_without_animation() {
        let (mut s, layer) = setup();
        let pin = s.execute("puppet.addPin", json!({"layer":layer,"position":[10,10],"expansion":0})).unwrap()["pin"].as_u64().unwrap();
        let uid = s.active_comp().unwrap().layer(effectcraft_engine::project::LayerId(layer)).unwrap().props.find_group(pin).unwrap().get("rest").unwrap().uid;
        s.execute("prop.set", json!({"layer":layer,"prop":uid,"value":[11,12]})).unwrap();
        let info = s.execute("prop.get", json!({"layer":layer,"prop":uid})).unwrap();
        assert_eq!(info["value"], json!([11.0, 12.0]));
        assert!(info["keys"].as_array().unwrap().is_empty());
    }

    #[test]
    fn malformed_separated_axis_ids_fail_without_recursive_inspection_or_history() {
        let (mut s, layer) = setup();
        s.execute("prop.separateDimensions", json!({"layer":layer})).unwrap();
        let cid = s.active_comp_id().unwrap();
        let layer = effectcraft_engine::project::LayerId(layer);
        let project = std::sync::Arc::make_mut(&mut s.project);
        let tr = project.comp_mut(cid).unwrap().layer_mut(layer).unwrap().props.sub_mut("transform").unwrap();
        let uid = tr.get("position").unwrap().uid;
        tr.get_mut("positionX").unwrap().uid = uid;
        s.history.clear();
        let before = s.project.to_json();
        assert!(s.execute("prop.get", json!({"layer":layer.0,"path":"transform/position"})).is_err());
        assert_eq!(s.project.to_json(), before);
        assert!(s.history.undo.is_empty());
    }
}

mod additive_selection {
    use effectcraft_engine::{Session, keyframe::Interp};
    use serde_json::json;

    #[test]
    fn repeated_additive_property_selection_keeps_keys_unique_and_hold_toggles_once() {
        let mut s = Session::default();
        s.execute_checked("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":4})).unwrap();
        s.execute_checked("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap();
        s.execute_checked("prop.addKey", json!({"layer":"Box","path":"transform/opacity","time":0,"value":10})).unwrap();
        s.execute_checked("prop.select", json!({"layer":"Box","path":"transform/opacity"})).unwrap();
        s.execute_checked("prop.select", json!({"layer":"Box","path":"transform/opacity","add":true})).unwrap();
        let counts = (s.state.selected_props.len(), s.state.selected_keys.len());
        s.history.clear();
        s.execute_checked("keys.toggleHold", json!({})).unwrap();
        let pr = s.active_comp().unwrap().layers[0].props.prop("transform/opacity").unwrap();
        assert_eq!(
            (counts.0, counts.1, pr.keys[0].out_interp, s.history.undo.len()),
            (1, 1, Interp::Hold, 1),
            "additive selection keeps unique refs and Hold changes exactly once"
        );
        assert!(s.undo());
        assert_eq!(s.active_comp().unwrap().layers[0].props.prop("transform/opacity").unwrap().keys[0].out_interp, Interp::Linear);
    }
}

mod animation_toggle {
    use effectcraft_engine::Session;
    use serde_json::json;
    #[test]
    fn parent_animation_toggle_updates_actual_position_tracks_and_preserves_value() {
        for separated in [false, true] {
            for three in [false, true] {
                let mut s = Session::default();
                s.execute("comp.new", json!({"width":100,"height":80,"duration":4,"frameRate":30})).unwrap();
                let layer = s.execute("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
                if three {
                    s.execute("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
                }
                if separated {
                    s.execute("prop.separateDimensions", json!({"layer":layer})).unwrap();
                }
                let paths: Vec<_> = if separated {
                    ["transform/positionX", "transform/positionY"].into_iter().chain(three.then_some("transform/positionZ")).collect()
                } else {
                    vec!["transform/position"]
                };
                for (dimension, path) in paths.iter().enumerate() {
                    for (time, base) in [(0, 10), (1, 40)] {
                        let value = if separated { json!(base + dimension * 10) } else { json!([base, base + 10, if three { base + 20 } else { 0 }]) };
                        s.execute("prop.addKey", json!({"layer":layer,"path":path,"time":time,"value":value})).unwrap();
                    }
                }
                s.history.clear();
                let before = s.project.to_json();
                s.execute("prop.toggleAnimation", json!({"layer":layer,"path":"transform/position","value":false})).unwrap();
                for path in &paths {
                    let info = s.execute("prop.get", json!({"layer":layer,"path":path})).unwrap();
                    assert!(info["keys"].as_array().unwrap().is_empty(), "separated animation toggle must stop rendered axis tracks");
                }
                let cid = s.active_comp_id().unwrap();
                let comp = s.active_comp().unwrap();
                let l = comp.layer(effectcraft_engine::project::LayerId(layer)).unwrap();
                let pr = l.transform().unwrap().get("position").unwrap();
                let actual = effectcraft_engine::render::EvalCtx::new(&s.project, cid, comp, effectcraft_engine::time::Tick::from_seconds_f64(1.0))
                    .value(l, pr)
                    .to_json();
                assert_eq!(actual, if three { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0, 0.0]) });
                assert_eq!(s.history.undo.len(), 1);
                s.execute("prop.toggleAnimation", json!({"layer":layer,"path":"transform/position","value":true})).unwrap();
                for path in &paths {
                    assert_eq!(s.execute("prop.get", json!({"layer":layer,"path":path})).unwrap()["keys"].as_array().unwrap().len(), 1);
                }
                assert_eq!(s.history.undo.len(), 2);
                s.execute("edit.undo", json!({})).unwrap();
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(s.project.to_json(), before);
            }
        }
    }
}

mod scalar_assignments {
    use effectcraft_engine::{Session, project::LayerId, render::EvalCtx};
    use serde_json::{Value, json};
    pub(super) fn setup(three: bool, separated: bool, animated: bool) -> (Session, u64) {
        let mut s = Session::default();
        s.execute_checked("comp.new", json!({"width":100,"height":80,"duration":4,"frameRate":30})).unwrap();
        let layer = s.execute_checked("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap()["layer"].as_u64().unwrap();
        if three {
            s.execute_checked("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
        }
        if separated {
            s.execute_checked("prop.separateDimensions", json!({"layer":layer})).unwrap();
        }
        if animated {
            s.execute_checked("prop.addKey", json!({"layer":layer,"path":"transform/position","time":0,"value":[10,20,30]})).unwrap();
        }
        s.history.clear();
        (s, layer)
    }
    fn position(s: &Session, layer: u64) -> Value {
        let cid = s.active_comp_id().unwrap();
        let comp = s.active_comp().unwrap();
        let l = comp.layer(LayerId(layer)).unwrap();
        let pr = l.transform().unwrap().get("position").unwrap();
        EvalCtx::new(&s.project, cid, comp, s.time_of(cid)).value(l, pr).to_json()
    }
    fn assignment(three: bool, separated: bool, animated: bool, value: Value, number: f64) {
        let (mut s, layer) = setup(three, separated, animated);
        let before = s.project.to_json();
        let reply = s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":value})).unwrap();
        let actual = position(&s, layer);
        let dimensions = if three { 3 } else { 2 };
        for dimension in 0..dimensions {
            assert_eq!(actual[dimension], number, "scalar/bool separated Position assignment must update rendered active axes");
        }
        let info = s.execute_checked("prop.get", json!({"layer":layer,"path":"transform/position"})).unwrap();
        assert_eq!(info["value"], actual);
        assert_eq!(reply.as_array().unwrap().len(), if separated { dimensions } else { 3 });
        assert_eq!(s.history.undo.len(), 1);
        let after = s.project.to_json();
        s.execute_checked("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.to_json(), before);
        s.execute_checked("edit.redo", json!({})).unwrap();
        assert_eq!(s.project.to_json(), after);
    }
    #[test]
    fn normal_scalar_and_bool_position_assignments_keep_broadcast_behavior() {
        for three in [false, true] {
            for animated in [false, true] {
                for (value, number) in [(json!(100), 100.0), (json!(true), 1.0), (json!(false), 0.0)] {
                    assignment(three, false, animated, value, number);
                }
            }
        }
    }
    #[test]
    fn separated_scalar_position_assignments_update_active_tracks_atomically() {
        for three in [false, true] {
            for animated in [false, true] {
                assignment(three, true, animated, json!(100), 100.0);
                assignment(three, true, animated, json!(-12.5), -12.5);
            }
        }
    }
    #[test]
    fn separated_bool_position_assignments_update_active_tracks_atomically() {
        for three in [false, true] {
            for animated in [false, true] {
                assignment(three, true, animated, json!(true), 1.0);
                assignment(three, true, animated, json!(false), 0.0);
            }
        }
    }
    #[test]
    fn separated_partial_arrays_and_empty_assignments_preserve_existing_semantics() {
        for three in [false, true] {
            for animated in [false, true] {
                let (mut s, layer) = setup(three, true, animated);
                s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[1,2,3]})).unwrap();
                let reply = s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[10]})).unwrap();
                assert_eq!(reply, json!([10.0]));
                let actual = position(&s, layer);
                assert_eq!(actual[0], 10.0);
                assert_eq!(actual[1], 2.0);
                assert_eq!(actual[2], if three { 3.0 } else { 0.0 });
                let before = s.project.to_json();
                let history = s.history.undo.len();
                assert_eq!(s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[]})).unwrap(), json!([]));
                assert_eq!(s.project.to_json(), before);
                assert_eq!(s.history.undo.len(), history);
                assert!(s.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":"bad"})).is_err());
                assert_eq!(s.project.to_json(), before);
                assert_eq!(s.history.undo.len(), history);
            }
        }
    }
}
