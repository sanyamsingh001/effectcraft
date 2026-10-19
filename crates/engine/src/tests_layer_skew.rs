//! Bounded public layer-skew gap and existing shape-group mathematical controls.
use crate::{
    Session,
    geom::vec2,
    project::LayerId,
    render::{EvalCtx, Image, shapes},
    time::Tick,
};
use serde_json::json;

fn setup() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":2})).unwrap();
    s
}
fn solid(s: &mut Session) -> u64 {
    s.execute("layer.newSolid", json!({"width":16,"height":8,"color":"#00ff00"})).unwrap()["layer"].as_u64().unwrap()
}
fn assert_corners(actual: impl IntoIterator<Item = [f64; 2]>, expected: [[f64; 2]; 4]) {
    let actual: Vec<_> = actual.into_iter().collect();
    assert_eq!(actual.len(), 4);
    for (point, target) in actual.iter().zip(expected) {
        for (value, goal) in point.iter().zip(target) {
            assert!((value - goal).abs() < 1e-9, "corner {point:?} differs from independent target {target:?}");
        }
    }
}
fn assert_sheared_raster(image: &Image) {
    assert_eq!((image.width, image.height), (64, 64));
    for (x, y) in [(32, 32), (41, 29), (22, 34)] {
        let p = image.get(x, y);
        assert!(p[1] > 0.9 && p[3] > 0.9 && p[0] < 0.01 && p[2] < 0.01, "independent interior sample ({x},{y}): {p:?}");
    }
    for (x, y) in [(20, 29), (42, 34), (32, 26), (32, 38)] {
        assert!(image.get(x, y)[3] < 0.01, "independent outside sample ({x},{y})");
    }
}
#[test]
fn existing_shape_group_skew_matches_independent_corners_and_raster() {
    let mut s = setup();
    let id = s.execute("layer.newShape", json!({"kind":"rect","size":[16,8],"position":[32,32],"fill":"#00ff00","strokeWidth":0})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    let rect = s.active_comp().unwrap().layer(LayerId(id)).unwrap().props.group("contents/group/contents/rect").unwrap().uid;
    s.execute("prop.set", json!({"layer":id,"path":"contents/group/transform/skew","value":45})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let layer = comp.layer(LayerId(id)).unwrap();
    let ctx = EvalCtx::new(&s.project, cid, comp, Tick::ZERO);
    let matrix = ctx.layer_to_comp(layer).0 * shapes::item_matrix(&ctx, layer, rect).unwrap();
    // At45° and axis0, the existing group's convention is x'=x-y, y'=y.
    let corners = [[-8.0, -4.0], [8.0, -4.0], [8.0, 4.0], [-8.0, 4.0]];
    assert_corners(
        corners.map(|p| {
            let v = matrix.apply(vec2(p[0], p[1]));
            [v.x, v.y]
        }),
        [[28.0, 28.0], [44.0, 28.0], [36.0, 36.0], [20.0, 36.0]],
    );
    assert_sheared_raster(&s.render(cid, Tick::ZERO, Default::default()));
}
#[test]
fn zero_skew_legacy_transform_and_raster_are_unchanged() {
    let mut s = setup();
    let id = solid(&mut s);
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let layer = comp.layer(LayerId(id)).unwrap();
    let matrix = EvalCtx::new(&s.project, cid, comp, Tick::ZERO).layer_to_comp(layer).0;
    assert_corners(
        [[0.0, 0.0], [16.0, 0.0], [16.0, 8.0], [0.0, 8.0]].map(|p| {
            let v = matrix.apply(vec2(p[0], p[1]));
            [v.x, v.y]
        }),
        [[24.0, 28.0], [40.0, 28.0], [40.0, 36.0], [24.0, 36.0]],
    );
    let before = s.render(cid, Tick::ZERO, Default::default());
    let loaded = crate::project::Project::from_json(&s.project.to_json()).unwrap();
    let after = crate::render::Renderer::new(&loaded, &crate::render::NoFootage, Default::default()).comp_frame_cpu(cid, Tick::ZERO);
    assert_eq!(before.data, after.data);
    assert_eq!(before.get(32, 32), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(before.get(41, 29)[3], 0.0);
}
#[test]
fn two_d_layer_skew_public_api_is_animatable_and_drives_raster() {
    let mut s = setup();
    let id = solid(&mut s);
    let set = s.execute("prop.set", json!({"layer":id,"path":"transform/skew","value":45}));
    assert!(set.is_ok(), "LAYER_SKEW_350: a2D layer must expose animatable Skew: {set:?}");
    s.execute("prop.set", json!({"layer":id,"path":"transform/skewAxis","value":0})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let layer = comp.layer(LayerId(id)).unwrap();
    assert!(!layer.is_3d());
    let matrix = EvalCtx::new(&s.project, cid, comp, Tick::ZERO).layer_to_comp(layer).0;
    assert_corners(
        [[0.0, 0.0], [16.0, 0.0], [16.0, 8.0], [0.0, 8.0]].map(|p| {
            let v = matrix.apply(vec2(p[0], p[1]));
            [v.x, v.y]
        }),
        [[28.0, 28.0], [44.0, 28.0], [36.0, 36.0], [20.0, 36.0]],
    );
    assert_sheared_raster(&s.render(cid, Tick::ZERO, Default::default()));
    s.execute("prop.addKey", json!({"layer":id,"path":"transform/skew","time":0,"value":0})).unwrap();
    let before = s.project.to_json_compact();
    s.execute("prop.addKey", json!({"layer":id,"path":"transform/skew","time":1,"value":45})).unwrap();
    let value = s.execute("prop.get", json!({"layer":id,"path":"transform/skew","time":0.5})).unwrap();
    assert_eq!(value["value"], json!(22.5));
    assert_eq!(value["keys"].as_array().unwrap().len(), 2);
    assert!(s.undo());
    assert_eq!(s.project.to_json_compact(), before, "animation edit must undo independently");
}

fn set(s: &mut Session, id: u64, path: &str, value: serde_json::Value) {
    s.execute("prop.set", json!({"layer":id,"path":path,"value":value})).unwrap();
}
fn corner_matrix(s: &Session, id: u64) -> crate::geom::Mat3 {
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    EvalCtx::new(&s.project, cid, comp, s.time()).layer_to_comp(comp.layer(LayerId(id)).unwrap()).0
}
fn solid_corners(s: &Session, id: u64) -> [[f64; 2]; 4] {
    let m = corner_matrix(s, id);
    [[0.0, 0.0], [16.0, 0.0], [16.0, 8.0], [0.0, 8.0]].map(|p| {
        let v = m.apply(vec2(p[0], p[1]));
        [v.x, v.y]
    })
}
#[test]
fn layer_skew_axis_negative_angles_and_transform_order_match_independent_corners() {
    let mut s = setup();
    let id = solid(&mut s);
    set(&mut s, id, "transform/skew", json!(-45));
    assert_corners(solid_corners(&s, id), [[20.0, 28.0], [36.0, 28.0], [44.0, 36.0], [28.0, 36.0]]);
    set(&mut s, id, "transform/skew", json!(45));
    set(&mut s, id, "transform/skewAxis", json!(450));
    assert_corners(solid_corners(&s, id), [[24.0, 20.0], [40.0, 36.0], [40.0, 44.0], [24.0, 28.0]]);
    set(&mut s, id, "transform/skewAxis", json!(0));
    set(&mut s, id, "transform/scale", json!([200, 50, 100]));
    set(&mut s, id, "transform/rotation", json!(90));
    assert_corners(solid_corners(&s, id), [[34.0, 18.0], [34.0, 50.0], [30.0, 46.0], [30.0, 14.0]]);
}
#[test]
fn layer_skew_is_two_d_only_and_retained_when_switching_dimensions() {
    let mut s = setup();
    let id = solid(&mut s);
    set(&mut s, id, "transform/skew", json!(45));
    set(&mut s, id, "transform/skewAxis", json!(12));
    let two_d = corner_matrix(&s, id);
    s.execute("layer.setSwitch", json!({"layers":[id],"switch":"threeD","value":true})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let l = comp.layer(LayerId(id)).unwrap();
    assert!(l.props.prop("transform/skew").unwrap().two_d_only);
    assert!(l.props.prop("transform/skewAxis").unwrap().two_d_only);
    let m = EvalCtx::new(&s.project, cid, comp, Tick::ZERO).local_matrix(l);
    let supported = crate::commands::essential::supported_properties(&s.project, cid);
    assert!(!supported.iter().any(|p| p["layer"] == json!(id) && matches!(p["path"].as_str(), Some("transform/skew" | "transform/skewAxis"))));
    set(&mut s, id, "transform/skew", json!(0));
    let comp = s.active_comp().unwrap();
    assert_eq!(EvalCtx::new(&s.project, cid, comp, Tick::ZERO).local_matrix(comp.layer(LayerId(id)).unwrap()), m, "3D ignores layer skew");
    set(&mut s, id, "transform/skew", json!(45));
    s.execute("layer.setSwitch", json!({"layers":[id],"switch":"threeD","value":false})).unwrap();
    assert_eq!(corner_matrix(&s, id), two_d);
}
#[test]
fn layer_skew_has_safe_evaluation_for_hostile_saved_or_expression_values() {
    use crate::keyframe::Value as KV;
    use std::sync::Arc;
    let mut s = setup();
    let id = solid(&mut s);
    let cid = s.active_comp_id().unwrap();
    let original = corner_matrix(&s, id);
    for (skew, axis) in [(f64::NAN, 0.0), (f64::INFINITY, 0.0), (0.0, f64::NEG_INFINITY)] {
        let l = Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(id)).unwrap();
        l.props.prop_mut("transform/skew").unwrap().value = KV::Scalar(skew);
        l.props.prop_mut("transform/skewAxis").unwrap().value = KV::Scalar(axis);
        assert_eq!(corner_matrix(&s, id), original);
    }
    for (skew, axis) in [(1e300, 1e300), (-1e300, f64::NAN), (45.0, f64::INFINITY)] {
        let l = Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(id)).unwrap();
        l.props.prop_mut("transform/skew").unwrap().value = KV::Scalar(skew);
        l.props.prop_mut("transform/skewAxis").unwrap().value = KV::Scalar(axis);
        assert!(corner_matrix(&s, id).0.into_iter().flatten().all(f64::is_finite));
        let image = s.render(cid, Tick::ZERO, Default::default());
        assert!(image.data.iter().flatten().all(|x| x.is_finite()));
    }
}
#[test]
fn two_d_parent_skew_inherits_through_the_shared_world_matrix() {
    let mut s = setup();
    let child = solid(&mut s);
    let parent = s.execute("layer.newNull", json!({})).unwrap()["layer"].as_u64().unwrap();
    set(&mut s, parent, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, parent, "transform/position", json!([25, 7, 0]));
    set(&mut s, parent, "transform/skew", json!(45));
    s.execute("layer.setParent", json!({"layers":[child],"parent":parent,"compensate":false})).unwrap();
    assert_corners(solid_corners(&s, child), [[21.0, 35.0], [37.0, 35.0], [29.0, 43.0], [13.0, 43.0]]);
}
