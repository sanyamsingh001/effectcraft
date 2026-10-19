//! Static 2D parenting preserves representable skewed child poses.
use effectcraft_engine::{
    Session,
    geom::vec2,
    project::{AutoOrient, BitDepth, LayerId},
    render::{Backend, EvalCtx, Image, NoFootage, RenderOpts, Renderer},
    time::Tick,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn setup() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width":64,"height":64,"frameRate":30,"duration":2})).unwrap();
    s
}
fn solid(s: &mut Session) -> u64 {
    s.execute("layer.newSolid", json!({"width":16,"height":8,"color":"#00ff00"})).unwrap()["layer"].as_u64().unwrap()
}
fn set(s: &mut Session, id: u64, path: &str, value: Value) {
    s.execute("prop.set", json!({"layer":id,"path":path,"value":value})).unwrap();
}
fn solid_corners(s: &Session, id: u64) -> [[f64; 2]; 4] {
    let cid = s.active_comp_id().unwrap();
    let comp = s.active_comp().unwrap();
    let m = EvalCtx::new(&s.project, cid, comp, s.time()).layer_to_comp(comp.layer(LayerId(id)).unwrap()).0;
    [[0.0, 0.0], [16.0, 0.0], [16.0, 8.0], [0.0, 8.0]].map(|p| {
        let v = m.apply(vec2(p[0], p[1]));
        [v.x, v.y]
    })
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
#[test]
fn static_two_d_skew_parent_attachment_and_detachment_keep_child_corners_raster_and_undo() {
    let mut s = setup();
    let child = solid(&mut s);
    let parent = s.execute("layer.newNull", json!({})).unwrap()["layer"].as_u64().unwrap();
    set(&mut s, parent, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, parent, "transform/position", json!([5, 7, 0]));
    set(&mut s, parent, "transform/skew", json!(45));
    let cid = s.active_comp_id().unwrap();
    let original_corners = solid_corners(&s, child);
    let original_image = s.render(cid, Tick::ZERO, Default::default());
    for target in [json!(parent), json!(null)] {
        let before = s.project.to_json_compact();
        s.execute("layer.setParent", json!({"layers":[child],"parent":target})).unwrap();
        let after = s.project.to_json_compact();
        assert_corners(solid_corners(&s, child), original_corners);
        let image = s.render(cid, Tick::ZERO, Default::default());
        let error = image.data.iter().zip(&original_image.data).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0_f32, f32::max);
        assert!(error < 1e-5, "SKEW_PARENT_COMPENSATION: parenting must preserve actual child pixels: {error}");
        assert!(s.undo());
        assert_eq!(s.project.to_json_compact(), before);
        assert!(s.redo());
        assert_eq!(s.project.to_json_compact(), after);
    }
}

fn fixture() -> (Session, u64, u64) {
    let mut s = setup();
    Arc::make_mut(&mut s.project).settings.bit_depth = BitDepth::Bpc32;
    let child = solid(&mut s);
    let parent = s.execute("layer.newNull", json!({})).unwrap()["layer"].as_u64().unwrap();
    set(&mut s, parent, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, parent, "transform/position", json!([5, 7, 0]));
    set(&mut s, parent, "transform/skew", json!(45));
    (s, parent, child)
}
fn cpu(s: &Session) -> Image {
    Renderer::new(&s.project, &NoFootage, RenderOpts { backend: Backend::Cpu, motion_blur: false, ..Default::default() })
        .comp_frame_cpu(s.active_comp_id().unwrap(), s.time())
}
fn image_error(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.data.iter().zip(&b.data).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0, f32::max)
}
fn finite_visible(image: &Image) {
    assert_eq!((image.width, image.height), (64, 64));
    assert!(image.data.iter().flatten().all(|v| v.is_finite()));
    assert!(image.data.iter().any(|v| v[1] > 0.9 && v[3] > 0.9), "the synthetic child must be visibly rendered");
}
fn roundtrip(s: &mut Session, parent: u64, child: u64) {
    let corners = solid_corners(s, child);
    let pixels = cpu(s);
    finite_visible(&pixels);
    let z = s.active_comp().unwrap().layer(LayerId(child)).unwrap().props.prop("transform/scale").unwrap().value.as_vec3()[2];
    let selection = s.state.selected_layers.clone();
    for target in [Some(parent), None] {
        let before = s.project.clone();
        let history = s.history.undo.len();
        s.execute_checked("layer.setParent", json!({"layers":[child],"parent":target})).unwrap();
        let after = s.project.clone();
        assert_eq!(s.history.undo.len(), history + 1);
        assert_corners(solid_corners(s, child), corners);
        let image = cpu(s);
        finite_visible(&image);
        assert!(image_error(&pixels, &image) < 2e-5, "STATIC_SKEW_PARENT: actual CPU child pixels must stay in place");
        let tr = s.active_comp().unwrap().layer(LayerId(child)).unwrap().transform().unwrap();
        assert_eq!(tr.get("scale").unwrap().value.as_vec3()[2], z, "2D compensation preserves the stored 3D Scale Z");
        assert!(tr.get("skew").unwrap().value.as_f64().abs() <= 85.0);
        assert_eq!(tr.get("skewAxis").unwrap().value.as_f64(), 0.0);
        assert_eq!(s.state.selected_layers, selection);
        assert!(s.undo());
        assert!(Arc::ptr_eq(&s.project, &before));
        assert!(s.redo());
        assert!(Arc::ptr_eq(&s.project, &after));
    }
}
#[test]
fn static_skew_parent_preserves_rotation_nonuniform_scales_and_child_skew_axis() {
    let (mut s, parent, child) = fixture();
    set(&mut s, parent, "transform/anchor", json!([3, 2, 0]));
    set(&mut s, parent, "transform/rotation", json!(-17));
    set(&mut s, parent, "transform/scale", json!([120, 70, 100]));
    set(&mut s, parent, "transform/skew", json!(38));
    set(&mut s, child, "transform/rotation", json!(29));
    set(&mut s, child, "transform/scale", json!([83, 127, 61]));
    set(&mut s, child, "transform/skew", json!(-12));
    set(&mut s, child, "transform/skewAxis", json!(25));
    roundtrip(&mut s, parent, child);
}
#[test]
fn static_skew_parent_preserves_mirrored_scales_and_axis_ninety() {
    let (mut s, parent, child) = fixture();
    set(&mut s, parent, "transform/rotation", json!(21));
    set(&mut s, parent, "transform/scale", json!([-120, 80, 100]));
    set(&mut s, parent, "transform/skew", json!(-32));
    set(&mut s, parent, "transform/skewAxis", json!(90));
    set(&mut s, child, "transform/rotation", json!(-13));
    set(&mut s, child, "transform/scale", json!([83, -127, 61]));
    set(&mut s, child, "transform/skew", json!(18));
    set(&mut s, child, "transform/skewAxis", json!(450));
    roundtrip(&mut s, parent, child);
}
#[test]
fn static_skew_parent_preserves_child_pixels_with_distinct_source_pixel_aspects() {
    let (mut s, _, child) = fixture();
    s.execute_checked("layer.settings", json!({"layer":child,"pixelAspect":1.5})).unwrap();
    let parent = s.execute_checked("layer.newSolid", json!({"width":16,"height":8,"color":"#ff0000","pixelAspect":2.0})).unwrap()["layer"].as_u64().unwrap();
    s.execute_checked("layer.setSwitch", json!({"layers":[parent],"switch":"video","value":false})).unwrap();
    set(&mut s, parent, "transform/anchor", json!([3, 2, 0]));
    set(&mut s, parent, "transform/position", json!([5, 7, 0]));
    set(&mut s, parent, "transform/skew", json!(45));
    set(&mut s, parent, "transform/skewAxis", json!(20));
    roundtrip(&mut s, parent, child);
}
#[test]
fn static_skew_parent_reparenting_between_nested_spaces_preserves_corners_and_raster() {
    let (mut s, parent, child) = fixture();
    let grand = s.execute_checked("layer.newNull", json!({})).unwrap()["layer"].as_u64().unwrap();
    set(&mut s, grand, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, grand, "transform/position", json!([7, 3, 0]));
    set(&mut s, grand, "transform/rotation", json!(13));
    set(&mut s, grand, "transform/skew", json!(-11));
    s.execute_checked("layer.setParent", json!({"layers":[parent],"parent":grand,"compensate":false})).unwrap();
    let old = s.execute_checked("layer.newNull", json!({})).unwrap()["layer"].as_u64().unwrap();
    set(&mut s, old, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, old, "transform/position", json!([0, 0, 0]));
    set(&mut s, old, "transform/skew", json!(8));
    s.execute_checked("layer.setParent", json!({"layers":[child],"parent":old,"compensate":false})).unwrap();
    roundtrip(&mut s, parent, child);
}
#[test]
fn explicit_uncompensated_skew_parent_retains_authored_transform_and_inherits_shear() {
    let (mut s, parent, child) = fixture();
    let before = s.active_comp().unwrap().layer(LayerId(child)).unwrap().transform().unwrap().clone();
    s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent,"compensate":false})).unwrap();
    assert_eq!(s.active_comp().unwrap().layer(LayerId(child)).unwrap().transform().unwrap(), &before);
    assert_corners(solid_corners(&s, child), [[1.0, 35.0], [17.0, 35.0], [9.0, 43.0], [-7.0, 43.0]]);
}
#[test]
fn legacy_zero_skew_compensation_retains_authored_axis_and_existing_values() {
    let (mut s, parent, child) = fixture();
    set(&mut s, parent, "transform/skew", json!(0));
    set(&mut s, parent, "transform/rotation", json!(90));
    set(&mut s, parent, "transform/scale", json!([200, 100, 100]));
    set(&mut s, child, "transform/rotation", json!(30));
    set(&mut s, child, "transform/scale", json!([120, 80, 77]));
    set(&mut s, child, "transform/skewAxis", json!(45));
    s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent})).unwrap();
    let tr = s.active_comp().unwrap().layer(LayerId(child)).unwrap().transform().unwrap();
    assert!((tr.get("rotation").unwrap().value.as_f64() + 60.0).abs() < 1e-12);
    assert_eq!(tr.get("scale").unwrap().value.as_vec3(), [60.0, 80.0, 77.0]);
    assert_eq!(tr.get("skew").unwrap().value.as_f64(), 0.0);
    assert_eq!(tr.get("skewAxis").unwrap().value.as_f64(), 45.0);
}
fn pose(s: &Session, child: u64) -> Vec<effectcraft_engine::project::Property> {
    let tr = s.active_comp().unwrap().layer(LayerId(child)).unwrap().transform().unwrap();
    ["rotation", "scale", "skew", "skewAxis"].map(|name| tr.get(name).unwrap().clone()).into()
}
#[test]
fn static_skew_compensation_keeps_animated_and_expression_authored_pose_on_legacy_path() {
    for (target, path, keyed) in
        [("child", "transform/skew", true), ("child", "transform/rotation", true), ("parent", "transform/skew", true), ("child", "transform/skew", false)]
    {
        let (mut s, parent, child) = fixture();
        let id = if target == "parent" { parent } else { child };
        if keyed {
            let value = if path.ends_with("skew") { 45 } else { 0 };
            s.execute_checked("prop.addKey", json!({"layer":id,"path":path,"time":0,"value":value})).unwrap();
            s.execute_checked("prop.addKey", json!({"layer":id,"path":path,"time":1,"value":value})).unwrap();
        } else {
            s.execute_checked("prop.setExpression", json!({"layer":id,"path":path,"expression":"value","enabled":false})).unwrap();
        }
        let before = pose(&s, child);
        s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent})).unwrap();
        assert_eq!(pose(&s, child), before, "a sampled QR decomposition must not overwrite authored pose tracks");
        assert!(cpu(&s).data.iter().flatten().all(|v| v.is_finite()));
    }
}
#[test]
fn static_skew_compensation_rejects_unrepresentable_and_singular_child_bases() {
    for singular in [false, true] {
        let (mut s, parent, child) = fixture();
        if singular {
            set(&mut s, child, "transform/scale", json!([0, 100, 100]));
        } else {
            set(&mut s, parent, "transform/skew", json!(-85));
            set(&mut s, child, "transform/skew", json!(85));
        }
        let before = pose(&s, child);
        s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent})).unwrap();
        assert_eq!(pose(&s, child), before, "unsupported QR results use existing authored compensation without clipping Skew");
        assert!(cpu(&s).data.iter().flatten().all(|v| v.is_finite()));
    }
}
#[test]
fn static_skew_compensation_keeps_auto_oriented_child_on_the_existing_path() {
    let (mut s, parent, child) = fixture();
    let cid = s.active_comp_id().unwrap();
    Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(child)).unwrap().auto_orient = AutoOrient::AlongPath;
    let before = pose(&s, child);
    s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent})).unwrap();
    assert_eq!(pose(&s, child), before);
    assert!(cpu(&s).data.iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn static_skew_parent_uses_inherited_space_when_source_par_is_tiny() {
    let (mut s, _, child) = fixture();
    let parent = s.execute_checked("layer.newSolid", json!({"width":16,"height":8,"color":"#ff0000","pixelAspect":1e-301})).unwrap()["layer"].as_u64().unwrap();
    s.execute_checked("layer.setSwitch", json!({"layers":[parent],"switch":"video","value":false})).unwrap();
    set(&mut s, parent, "transform/anchor", json!([0, 0, 0]));
    set(&mut s, parent, "transform/position", json!([5, 7, 0]));
    set(&mut s, parent, "transform/skew", json!(45));
    roundtrip(&mut s, parent, child);
}
#[test]
fn static_skew_compensation_preserves_nonfinite_authored_skew_on_the_existing_path() {
    use effectcraft_engine::keyframe::Value as KV;
    for (name, value) in [("skew", f64::NAN), ("skewAxis", f64::INFINITY)] {
        let (mut s, parent, child) = fixture();
        let cid = s.active_comp_id().unwrap();
        Arc::make_mut(&mut s.project).comp_mut(cid).unwrap().layer_mut(LayerId(child)).unwrap().props.prop_mut(&format!("transform/{name}")).unwrap().value =
            KV::Scalar(value);
        s.execute_checked("layer.setParent", json!({"layers":[child],"parent":parent})).unwrap();
        let actual = s.active_comp().unwrap().layer(LayerId(child)).unwrap().props.prop(&format!("transform/{name}")).unwrap().value.as_f64();
        assert_eq!(actual.to_bits(), value.to_bits(), "a failed static-pose decomposition preserves authored nonfinite data");
        assert!(cpu(&s).data.iter().flatten().all(|v| v.is_finite()));
    }
}
