//! Adding a Position pin on a bent layer preserves the displayed pose and remains undoable.
use effectcraft_engine::{
    Session,
    effects::puppet,
    project::{LayerId, Project},
    time::Tick,
};
use serde_json::{Value, json};

fn deformed(s: &mut Session, layer: u64) -> Value {
    s.execute("puppet.info", json!({"layer":layer})).unwrap()["meshes"][0]["deformed"].clone()
}
fn point(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}
fn max_distance(a: &Value, b: &Value) -> f64 {
    a.as_array()
        .unwrap()
        .iter()
        .zip(b.as_array().unwrap())
        .map(|(a, b)| {
            let a = point(a);
            let b = point(b);
            (a[0] - b[0]).hypot(a[1] - b[1])
        })
        .fold(0.0, f64::max)
}

#[test]
fn adding_pin_to_bent_pose_preserves_shape_movement_undo_and_json_roundtrip() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":900,"height":400,"duration":2,"frameRate":30})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"width":800,"height":160,"color":[0,0,1]})).unwrap()["layer"].as_u64().unwrap();
    let a = s.execute("puppet.addPin", json!({"layer":layer,"position":[0,80],"expansion":0})).unwrap();
    let b = s.execute("puppet.addPin", json!({"layer":layer,"position":[800,80]})).unwrap();
    s.set_time(Tick::from_seconds_f64(1.0));
    s.execute("puppet.movePin", json!({"layer":layer,"pin":b["pin"],"position":[640,-180]})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let fx = a["effect"].as_u64().unwrap();
    let (buf, params) = effectcraft_engine::commands::puppet::puppet_eval(&s, cid, LayerId(layer), fx, s.time()).unwrap();
    let overlays = puppet::overlay(&buf, &params);
    let (_, mesh, pose, _) = &overlays[0];
    let t = mesh
        .tris
        .iter()
        .find(|t| {
            let c = [0, 1].map(|k| t.iter().map(|i| mesh.verts[*i][k]).sum::<f64>() / 3.0);
            c[0] > 680.0 && c[0] < 750.0 && c[1] > 30.0 && c[1] < 90.0
        })
        .unwrap();
    let click = [0, 1].map(|k| t.iter().map(|i| pose[*i][k]).sum::<f64>() / 3.0);
    let before = deformed(&mut s, layer);
    let c = s.execute("puppet.addPin", json!({"layer":layer,"position":[click[0],click[1]]})).unwrap();
    assert_eq!(deformed(&mut s, layer), before, "a click must preserve the full posed mesh");
    let uid = c["pin"].as_u64().unwrap();
    let group = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find_group(uid).unwrap();
    assert_eq!(serde_json::to_value(&group.get("attachment").unwrap().value).unwrap(), json!({"t":"Bool","v":true}));
    assert!(serde_json::to_value(&group.get("rest").unwrap().value).unwrap()["v"].is_array());
    let saved = s.project.to_json();
    let mut restored = Session::default();
    restored.replace_project(Project::from_json(&saved).unwrap(), None);
    restored.set_time(Tick::from_seconds_f64(1.0));
    assert_eq!(deformed(&mut restored, layer), before);
    s.execute("puppet.movePin", json!({"layer":layer,"pin":uid,"position":[click[0]+0.0001,click[1]]})).unwrap();
    let moved = deformed(&mut s, layer);
    let d = max_distance(&moved, &before);
    assert!(d > 0.0 && d < 0.01, "tiny move produced {d}px response");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(deformed(&mut s, layer), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(deformed(&mut s, layer), before);
    assert!(s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find_group(uid).is_none());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(deformed(&mut s, layer), before);
    // Adding the pin preserves the existing Position keys at earlier times.
    s.set_time(Tick::ZERO);
    let info = s.execute("puppet.info", json!({"layer":layer})).unwrap();
    let pins = info["meshes"][0]["pins"].as_array().unwrap();
    assert_eq!(pins.iter().find(|p| p["pin"] == a["pin"]).unwrap()["position"], json!([0.0, 80.0]));
    assert_eq!(pins.iter().find(|p| p["pin"] == b["pin"]).unwrap()["position"], json!([800.0, 80.0]));
}

#[test]
fn disabled_attachments_still_count_toward_insertion_budget() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":100,"height":80,"duration":2,"frameRate":30})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"width":40,"height":20,"color":[0,0,1]})).unwrap()["layer"].as_u64().unwrap();
    let first = s.execute("puppet.addPin", json!({"layer":layer,"position":[1,10],"expansion":0})).unwrap();
    let right = s.execute("puppet.addPin", json!({"layer":layer,"position":[39,10]})).unwrap();
    s.execute("puppet.movePin", json!({"layer":layer,"pin":right["pin"],"position":[30,-10]})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let (buf, params) = effectcraft_engine::commands::puppet::puppet_eval(&s, cid, LayerId(layer), first["effect"].as_u64().unwrap(), Tick::ZERO).unwrap();
    let overlays = puppet::overlay(&buf, &params);
    let (_, mesh, pose, pins) = &overlays[0];
    let triangle = mesh
        .tris
        .iter()
        .find(|t| {
            let rest = [0, 1].map(|c| t.iter().map(|i| mesh.verts[*i][c]).sum::<f64>() / 3.0);
            puppet::can_attach(mesh, pins, rest)
        })
        .unwrap();
    let click = [0, 1].map(|c| triangle.iter().map(|i| pose[*i][c]).sum::<f64>() / 3.0);
    let bound = s.execute("puppet.addPin", json!({"layer":layer,"position":[click[0],click[1]]})).unwrap()["pin"].as_u64().unwrap();
    s.execute("prop.setGroupEnabled", json!({"layer":layer,"prop":bound,"value":false})).unwrap();
    for _ in 1..puppet::MAX_ATTACHMENTS {
        s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":bound})).unwrap();
    }
    let before = s.project.to_json();
    let history = s.history.undo.len();
    let result = s.execute("puppet.addPin", json!({"layer":layer,"position":[click[0],click[1]]}));
    assert!(result.is_err(), "disabled serialized bindings must not bypass the cap: {result:?}");
    assert_eq!(s.project.to_json(), before);
    assert_eq!(s.history.undo.len(), history);
    s.execute("prop.removeGroup", json!({"layer":layer,"prop":bound})).unwrap();
    s.execute("puppet.addPin", json!({"layer":layer,"position":[click[0],click[1]]})).unwrap();
}

#[test]
fn duplicating_attached_pin_respects_disabled_budget_without_editing_project() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":100,"height":80,"duration":2,"frameRate":30})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"width":40,"height":20,"color":[0,0,1]})).unwrap()["layer"].as_u64().unwrap();
    let first = s.execute("puppet.addPin", json!({"layer":layer,"position":[1,10],"expansion":0})).unwrap();
    let right = s.execute("puppet.addPin", json!({"layer":layer,"position":[39,10]})).unwrap();
    s.execute("puppet.movePin", json!({"layer":layer,"pin":right["pin"],"position":[30,-10]})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let (buf, params) = effectcraft_engine::commands::puppet::puppet_eval(&s, cid, LayerId(layer), first["effect"].as_u64().unwrap(), Tick::ZERO).unwrap();
    let overlays = puppet::overlay(&buf, &params);
    let (_, mesh, pose, pins) = &overlays[0];
    let triangle = mesh
        .tris
        .iter()
        .find(|t| {
            let rest = [0, 1].map(|c| t.iter().map(|i| mesh.verts[*i][c]).sum::<f64>() / 3.0);
            puppet::can_attach(mesh, pins, rest)
        })
        .unwrap();
    let click = [0, 1].map(|c| triangle.iter().map(|i| pose[*i][c]).sum::<f64>() / 3.0);
    let bound = s.execute("puppet.addPin", json!({"layer":layer,"position":[click[0],click[1]]})).unwrap()["pin"].as_u64().unwrap();
    s.execute("prop.setGroupEnabled", json!({"layer":layer,"prop":bound,"value":false})).unwrap();
    for _ in 1..puppet::MAX_ATTACHMENTS {
        s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":bound})).unwrap();
    }
    let before = s.project.to_json();
    let history = s.history.undo.len();
    let result = s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":bound}));
    assert!(result.is_err(), "duplicating an attached pin must not bypass the mesh budget: {result:?}");
    assert_eq!(s.project.to_json(), before);
    assert_eq!(s.history.undo.len(), history);
    // Legacy vertex pins and unrelated effect groups retain normal duplication behavior.
    s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":first["pin"]})).unwrap();
    let invert = s.execute("effect.apply", json!({"layer":layer,"effect":"Invert"})).unwrap()["effects"][0].as_u64().unwrap();
    s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":invert})).unwrap();
    let copy = s.active_comp().unwrap().layer(LayerId(layer)).unwrap().props.find_group(first["mesh"].as_u64().unwrap()).unwrap();
    let removable = puppet::pins(copy).find(|p| p.uid != bound && p.get("attachment").is_some_and(|p| p.value.as_bool())).unwrap().uid;
    s.execute("prop.removeGroup", json!({"layer":layer,"prop":removable})).unwrap();
    s.execute("prop.duplicateGroup", json!({"layer":layer,"prop":bound})).unwrap();
}
