//! The current 3D source box projected into the layer handle plane or directly into the view.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use effectcraft_engine::geom::{Mat3, vec2, vec3};
use effectcraft_engine::project::{ItemKind, Layer, LayerSource, PrimitiveKind};
use effectcraft_engine::render::three_d::{CameraState, adv::scene::model_to_layer, camera::NEAR};
use effectcraft_engine::render::{EvalCtx, FootageSource};

type Bounds = ([f64; 3], [f64; 3]);

struct PoseBounds {
    model: Weak<effectcraft_model::Model>,
    clip: Option<usize>,
    time: f64,
    bounds: Option<Bounds>,
}

thread_local! {
    // Reusing one evaluated pose avoids rescanning imported mesh vertices for every overlay,
    // hit test and pointer update. Weak references let the footage cache release old models.
    static POSES: RefCell<VecDeque<PoseBounds>> = const { RefCell::new(VecDeque::new()) };
}

fn pose_bounds(model: &Arc<effectcraft_model::Model>, clip: Option<usize>, time: f64) -> Option<Bounds> {
    let weak = Arc::downgrade(model);
    if let Some(bounds) = POSES.with(|cache| cache.borrow().iter().find(|p| p.model.ptr_eq(&weak) && p.clip == clip && p.time == time).map(|p| p.bounds)) {
        return bounds;
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for instance in model.instances(clip, time) {
        for primitive in &instance.mesh.primitives {
            for (index, vertex) in primitive.positions.iter().enumerate() {
                let p = effectcraft_model::skin_point(&instance, primitive, index, *vertex);
                if p.iter().all(|v| v.is_finite()) {
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(p[axis]);
                        hi[axis] = hi[axis].max(p[axis]);
                    }
                }
            }
        }
    }
    let bounds = lo.iter().all(|v| v.is_finite()).then_some((lo, hi));
    POSES.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|p| p.model.strong_count() != 0);
        if cache.len() >= 32 {
            cache.pop_front();
        }
        cache.push_back(PoseBounds { model: weak, clip, time, bounds });
    });
    bounds
}

fn source_bounds(ctx: &EvalCtx, layer: &Layer, footage: &dyn FootageSource) -> Option<(Bounds, f64)> {
    let geometry = layer.props.sub("geometryOptions")?;
    let f = |name, default| ctx.f(layer, geometry, name, default);
    match layer.source {
        LayerSource::Primitive { kind } => {
            let half = match kind {
                PrimitiveKind::Cube => [f("width", 300.0).abs() * 0.5, f("height", 300.0).abs() * 0.5, f("depth", 300.0).abs() * 0.5],
                PrimitiveKind::Plane => [f("width", 300.0).abs() * 0.5, f("height", 300.0).abs() * 0.5, 0.0],
                PrimitiveKind::Sphere => [f("radius", 150.0).abs(); 3],
                PrimitiveKind::Torus => {
                    let tube = f("tubeRadius", 50.0).abs();
                    [f("radius", 150.0).abs() + tube, tube, f("radius", 150.0).abs() + tube]
                }
                PrimitiveKind::Cone | PrimitiveKind::Cylinder => [f("radius", 150.0).abs(), f("height", 300.0).abs() * 0.5, f("radius", 150.0).abs()],
            };
            half.iter().all(|v| v.is_finite()).then_some(((half.map(|v| -v), half), 1.0))
        }
        LayerSource::Model { item } => {
            let ItemKind::Footage(source) = &ctx.project.item(item)?.kind else { return None };
            let model = footage.model(item, source)?;
            let clip = (ctx.e(layer, geometry, "animation") as usize).checked_sub(1).filter(|c| *c < model.animations.len());
            let mut time = if clip.is_some() { layer.layer_time(ctx.time).seconds() * f("animationSpeed", 100.0) / 100.0 } else { 0.0 };
            if let Some(clip) = clip {
                let duration = model.animations[clip].duration();
                if ctx.b(layer, geometry, "loopAnimation") && duration > 0.0 {
                    time = time.rem_euclid(duration);
                }
            }
            Some((pose_bounds(&model, clip, time)?, f("unitScale", 1.0)))
        }
        _ => None,
    }
}

/// A usable inverse for local pointer edits. Remove projected translation before comparing
/// the two plane axes, so an edge-on plane is rejected without treating a large comp origin
/// as an ill-conditioned transform. The original matrix still places the anchor and gizmo.
pub(super) fn plane_inverse(matrix: &Mat3) -> Option<Mat3> {
    let m = &matrix.0;
    if m[2][2] == 0.0 || !m[2][2].is_finite() {
        return None;
    }
    let origin = matrix.apply(vec2(0.0, 0.0));
    let axes = [m[0][0] - origin.x * m[2][0], m[0][1] - origin.x * m[2][1], m[1][0] - origin.y * m[2][0], m[1][1] - origin.y * m[2][1]];
    if axes.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let scale = axes.iter().map(|v| v.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return None;
    }
    let [a, b, c, d] = axes.map(|v| v / scale);
    if (a * d - b * c).abs() <= 1e-12 {
        return None;
    }
    matrix.inverse().filter(|inverse| inverse.0.iter().flatten().all(|v| v.is_finite()))
}

/// A box in the layer's XY plane that encloses its projected 3D source. Keeping the plane
/// matrix makes the existing corner/edge scale gesture use the same coordinates as its handles.
pub(super) fn bounds(ctx: &EvalCtx, layer: &Layer, footage: &dyn FootageSource, matrix: &Mat3, camera: &CameraState) -> Option<[f64; 4]> {
    projection_bounds(ctx, layer, footage, &plane_inverse(matrix)?, camera)
}

/// The visible source envelope in comp pixels, independent of a singular layer XY plane.
/// These coordinates are for outlines and body picking, never local Scale coordinates.
pub(super) fn projected_bounds(ctx: &EvalCtx, layer: &Layer, footage: &dyn FootageSource, camera: &CameraState) -> Option<[f64; 4]> {
    projection_bounds(ctx, layer, footage, &Mat3::IDENTITY, camera)
}

fn projection_bounds(ctx: &EvalCtx, layer: &Layer, footage: &dyn FootageSource, projection: &Mat3, camera: &CameraState) -> Option<[f64; 4]> {
    let ((lo, hi), units) = source_bounds(ctx, layer, footage)?;
    let world = ctx.world_matrix(layer) * model_to_layer(units);
    let points = std::array::from_fn::<_, 8, _>(|i| {
        camera.view.apply(world.apply(vec3(
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        )))
    });
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut add = |p: effectcraft_engine::geom::Vec3| {
        let scale = if camera.ortho { camera.zoom } else { camera.zoom / p.z };
        let local = projection.apply(vec2(p.x * scale + ctx.comp.width as f64 * 0.5, p.y * scale + ctx.comp.height as f64 * 0.5));
        if local.x.is_finite() && local.y.is_finite() {
            bounds[0] = bounds[0].min(local.x);
            bounds[1] = bounds[1].min(local.y);
            bounds[2] = bounds[2].max(local.x);
            bounds[3] = bounds[3].max(local.y);
        }
    };
    for point in points {
        if camera.ortho || point.z >= NEAR {
            add(point);
        }
    }
    // Include the box edges clipped by the near plane rather than projecting behind-camera
    // vertices into inverted or unbounded controls.
    if !camera.ortho {
        for i in 0..8 {
            for bit in [1, 2, 4] {
                let other = i ^ bit;
                if i < other && (points[i].z < NEAR) != (points[other].z < NEAR) {
                    let a = points[i];
                    let b = points[other];
                    let p = a + (b - a) * ((NEAR - a.z) / (b.z - a.z));
                    add(vec3(p.x, p.y, NEAR));
                }
            }
        }
    }
    (bounds.iter().all(|v| v.is_finite()) && bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use effectcraft_engine::Session;
    use serde_json::json;

    fn cube(z: f64) -> (Session, effectcraft_engine::project::LayerId) {
        let mut session = Session::default();
        session.execute_checked("comp.new", json!({"width":320,"height":240,"duration":0.04,"frameRate":25})).unwrap();
        let layer = session.execute_checked("layer.newPrimitive", json!({"kind":"cube","width":4,"height":4,"depth":4,"position":[0,0,z]})).unwrap()["layer"]
            .as_u64()
            .unwrap();
        session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/rotationY","value":90})).unwrap();
        (session, effectcraft_engine::project::LayerId(layer))
    }
    fn camera(ortho: bool) -> CameraState {
        CameraState { view: effectcraft_engine::geom::Mat4::IDENTITY, eye: vec3(0.0, 0.0, 0.0), zoom: if ortho { 2.0 } else { 1.0 }, ortho, dof: None }
    }
    fn assert_bounds(actual: [f64; 4], expected: [f64; 4]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-9, "projected bound {actual} must equal {expected}");
        }
    }
    #[test]
    fn singular_layer_plane_still_projects_its_near_clipped_source_volume() {
        let (session, id) = cube(2.0);
        let comp = session.active_comp().unwrap();
        let layer = comp.layer(id).unwrap();
        let ctx = EvalCtx::new(&session.project, session.active_comp_id().unwrap(), comp, session.time());
        let camera = camera(false);
        let matrix = effectcraft_engine::render::three_d::layer_to_view(&ctx, layer, &camera);
        assert!(plane_inverse(&matrix).is_none(), "the source-volume fallback is independent of its singular XY plane");
        assert!(bounds(&ctx, layer, session.footage.as_ref(), &matrix, &camera).is_none(), "there are no valid local Scale coordinates");
        let projected = projected_bounds(&ctx, layer, session.footage.as_ref(), &camera).unwrap();
        // The box spans depth 0..4. Clipping its edges at NEAR=1 expands the front
        // envelope to +/-2, rather than only projecting the surviving depth-4 corners.
        assert_bounds(projected, [158.0, 118.0, 162.0, 122.0]);
    }
    #[test]
    fn local_plane_inverse_preserves_tilt_mirror_and_uniform_tiny_scale() {
        for matrix in [
            Mat3::translate(vec2(100_000.0, -80_000.0)) * Mat3::rotate_deg(25.0) * Mat3::scale(vec2(-2.0, 0.5)),
            Mat3::translate(vec2(100_000.0, -80_000.0)) * Mat3::scale(vec2(1e-8, 1e-8)),
        ] {
            let inverse = plane_inverse(&matrix).expect("ordinary plane axes retain their relative conditioning");
            let restored = inverse.apply(matrix.apply(vec2(20.0, -10.0)));
            assert!((restored.x - 20.0).abs() < 0.01 && (restored.y + 10.0).abs() < 0.01);
        }
    }
    #[test]
    fn nonfinite_and_collapsed_local_planes_have_no_pointer_inverse() {
        for matrix in [Mat3::scale(vec2(0.0, 1.0)), Mat3::scale(vec2(f64::NAN, 1.0)), Mat3::scale(vec2(1e-16, 1.0))] {
            assert!(plane_inverse(&matrix).is_none());
        }
    }
    #[test]
    fn a_source_volume_behind_the_near_plane_has_no_projected_pick_box() {
        let (session, id) = cube(-5.0);
        let comp = session.active_comp().unwrap();
        let layer = comp.layer(id).unwrap();
        let ctx = EvalCtx::new(&session.project, session.active_comp_id().unwrap(), comp, session.time());
        assert!(projected_bounds(&ctx, layer, session.footage.as_ref(), &camera(false)).is_none());
    }
    #[test]
    fn orthographic_source_projection_preserves_the_view_scale_without_local_coordinates() {
        let (session, id) = cube(2.0);
        let comp = session.active_comp().unwrap();
        let layer = comp.layer(id).unwrap();
        let ctx = EvalCtx::new(&session.project, session.active_comp_id().unwrap(), comp, session.time());
        assert_bounds(projected_bounds(&ctx, layer, session.footage.as_ref(), &camera(true)).unwrap(), [156.0, 116.0, 164.0, 124.0]);
    }
}
