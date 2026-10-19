use effectcraft_automation::{
    Backend, Session,
    tools::{self, Reply},
};
use serde_json::{Value, json};
fn setup(three: bool, separated: bool) -> Backend {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":100,"height":80,"duration":20,"frameRate":30})).unwrap();
    s.execute("layer.newSolid", json!({"name":"Box","color":"#fff"})).unwrap();
    if three {
        s.execute("layer.setSwitch", json!({"layer":"Box","switch":"threeD","value":true})).unwrap();
    }
    if separated {
        s.execute("prop.separateDimensions", json!({"layer":"Box"})).unwrap();
    }
    s.execute("layer.timing", json!({"layer":"Box","start":10.5,"in":10.5,"out":20})).unwrap();
    s.history.clear();
    Backend::headless(s)
}
fn tool(b: &mut Backend, name: &str, args: Value) -> Value {
    let Reply::Json(info) = tools::run(b, name, &args).unwrap() else { panic!("JSON reply") };
    info
}
fn rendered(b: &mut Backend, time: f64) -> Value {
    let s = b.session().unwrap();
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let l = comp.layers.iter().find(|l| l.name == "Box").unwrap();
    let p = l.transform().unwrap().get("position").unwrap();
    effectcraft_engine::render::EvalCtx::new(&s.project, cid, comp, effectcraft_engine::time::Tick::from_seconds_f64(time)).value(l, p).to_json()
}
#[test]
fn mcp_set_property_keys_separated_axes_at_the_written_comp_time() {
    for three in [false, true] {
        let mut b = setup(three, true);
        let value = if three { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0, 0.0]) };
        let info = tool(&mut b, "set_property", json!({"layer":"Box","path":"transform/position","time":15.3,"timeBase":"comp","value":value}));
        assert_eq!(rendered(&mut b, 15.3), value, "MCP explicit-time Position must update rendered followers");
        assert_eq!(info["value"], value);
        assert_eq!(info["layerTime"], 4.8);
        assert_eq!(info["compTime"], 15.3);
        assert_eq!(info["keyProperties"].as_array().unwrap().len(), if three { 3 } else { 2 });
        assert_eq!(info["axisProperties"].as_array().unwrap().len(), if three { 3 } else { 2 });
        assert!(info["keys"].as_array().unwrap().is_empty());
        assert_eq!(b.session().unwrap().history.undo.len(), 1);
        b.exec("edit.undo", json!({})).unwrap();
        assert_eq!(rendered(&mut b, 15.3), json!([50.0, 40.0, 0.0]));
    }
}
#[test]
fn mcp_interpolation_targets_every_separated_axis_in_one_undo() {
    for three in [false, true] {
        let mut b = setup(three, true);
        let value = if three { json!([10, 20, 30]) } else { json!([10, 20]) };
        let info = tool(
            &mut b,
            "add_keyframe",
            json!({"layer":"Box","path":"transform/position","timeBase":"comp","keys":[{"time":15.3,"value":value},{"time":16,"value":[40,50,60]}],"interpolation":"easyEase"}),
        );
        assert_eq!(
            rendered(&mut b, 15.3),
            if three { json!([10.0, 20.0, 30.0]) } else { json!([10.0, 20.0, 0.0]) },
            "MCP key batch must animate actual followers"
        );
        assert_eq!(info["addedKeys"][0]["layerTime"], 4.8);
        assert_eq!(info["addedKeys"][0]["compTime"], 15.3);
        let axes = info["axisProperties"].as_array().unwrap();
        assert_eq!(axes.len(), if three { 3 } else { 2 });
        for axis in axes {
            assert_eq!(axis["keys"].as_array().unwrap().len(), 2);
            assert_eq!(axis["keys"][0]["out"], "Bezier");
        }
        assert_eq!(b.session().unwrap().history.undo.len(), 1);
        b.exec("edit.undo", json!({})).unwrap();
        for path in ["transform/positionX", "transform/positionY"].into_iter().chain(three.then_some("transform/positionZ")) {
            assert!(b.exec("prop.get", json!({"layer":"Box","path":path})).unwrap()["keys"].as_array().unwrap().is_empty());
        }
        b.exec("edit.redo", json!({})).unwrap();
        assert_eq!(rendered(&mut b, 16.0), if three { json!([40.0, 50.0, 60.0]) } else { json!([40.0, 50.0, 0.0]) });
    }
}
#[test]
fn mcp_normal_vector_reply_keeps_legacy_keys_and_interpolation() {
    let mut b = setup(false, false);
    let info =
        tool(&mut b, "add_keyframe", json!({"layer":"Box","path":"transform/position","time":15.3,"timeBase":"comp","value":[10,20],"interpolation":"hold"}));
    assert_eq!(info["keys"][0]["out"], "Hold");
    assert_eq!(info["value"], json!([10.0, 20.0, 0.0]));
    assert!(info.get("keyProperties").is_none());
    assert_eq!(b.session().unwrap().history.undo.len(), 1);
    assert_eq!(rendered(&mut b, 15.3), json!([10.0, 20.0, 0.0]));
}

#[test]
fn invalid_separated_batch_restores_axes_history_and_selection() {
    let mut b = setup(true, true);
    let s = b.session().unwrap();
    let before = s.project.clone();
    let state = serde_json::to_value(&s.state).unwrap();
    let result = tools::run(
        &mut b,
        "add_keyframe",
        &json!({"layer":"Box","path":"transform/position","timeBase":"comp","keys":[{"time":15.3,"value":[10,20,30]},{"time":16,"value":[40,"bad",60]}],"interpolation":"hold"}),
    );
    assert!(result.is_err());
    let s = b.session().unwrap();
    assert!(s.project.same_content(&before));
    assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    assert!(s.history.undo.is_empty());
}
#[test]
fn mcp_omitted_position_value_captures_real_axis_tracks() {
    let mut b = setup(true, true);
    tool(&mut b, "set_property", json!({"layer":"Box","path":"transform/position","value":[12,23,34]}));
    b.session().unwrap().history.clear();
    let info = tool(&mut b, "add_keyframe", json!({"layer":"Box","path":"transform/position","time":4.8,"timeBase":"layer","interpolation":"hold"}));
    assert_eq!(info["layerTime"], 4.8);
    assert_eq!(info["compTime"], 15.3);
    assert_eq!(rendered(&mut b, 15.3), json!([12.0, 23.0, 34.0]));
    for axis in info["axisProperties"].as_array().unwrap() {
        assert_eq!(axis["keys"][0]["out"], "Hold");
    }
    assert_eq!(b.session().unwrap().history.undo.len(), 1);
}

mod scalar_assignments {
    use effectcraft_engine::Session;
    use serde_json::json;
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
}

mod mcp_assignments {
    use effectcraft_automation::{
        Backend,
        tools::{self, Reply},
    };
    use serde_json::json;
    #[test]
    fn mcp_scalar_and_bool_position_assignments_report_the_real_followers() {
        for three in [false, true] {
            for animated in [false, true] {
                for (value, number) in [(json!(100), 100.0), (json!(true), 1.0), (json!(false), 0.0)] {
                    let (s, layer) = super::scalar_assignments::setup(three, true, animated);
                    let mut b = Backend::headless(s);
                    let before = b.session().unwrap().project.to_json();
                    let Reply::Json(info) = tools::run(&mut b, "set_property", &json!({"layer":layer,"path":"transform/position","value":value})).unwrap()
                    else {
                        panic!("JSON reply")
                    };
                    for dimension in 0..if three { 3 } else { 2 } {
                        assert_eq!(info["value"][dimension], number, "MCP scalar/bool Position must report changed active followers");
                    }
                    assert_eq!(info["keyProperties"].as_array().unwrap().len(), if three { 3 } else { 2 });
                    assert_eq!(b.session().unwrap().history.undo.len(), 1);
                    b.exec("edit.undo", json!({})).unwrap();
                    assert_eq!(b.session().unwrap().project.to_json(), before);
                }
            }
        }
    }
}
