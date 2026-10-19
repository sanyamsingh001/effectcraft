//! Generated Particle World content follows the composition's enabled active camera.
use effectcraft_effects::{self as effects, EffectCtx, EffectEnv, Params};
use effectcraft_keyframe::{Keyframe, Value};
use effectcraft_project::{BitDepth, Comp, ItemId, ItemKind, Layer, LayerId, LayerSource, Project, Solid, build};
use effectcraft_raster::Image;
use effectcraft_render::{Backend, LayerCache, NoFootage, RenderOpts, Renderer};
use effectcraft_time::{FrameRate, Tick};
use std::hash::Hasher;
const W: u32 = 96;
const H: u32 = 64;
const ID: &str = "ec.sim.ccparticleworld";
fn set(layer: &mut Layer, path: &str, value: Value) {
    layer.props.prop_mut(path).unwrap().value = value;
}
fn scene(camera: bool) -> (Project, ItemId, LayerId, LayerId) {
    let mut project = Project::default();
    project.settings.bit_depth = BitDepth::Bpc32;
    let mut comp = Comp::new(W, H, FrameRate::FPS_30, Tick::from_seconds_f64(2.5));
    let item = project.add_item("Black", Default::default(), None, ItemKind::Solid(Solid { width: W, height: H, color: [0.0; 3], pixel_aspect: 1.0 }));
    let mut particles = build::layer(&mut project, &comp, "Particles", LayerSource::Solid { item }, (W, H), None);
    let mut next_effect_id = 5000;
    let mut fx = effects::instantiate(effects::find(ID).unwrap(), &mut build::Ids(&mut next_effect_id), "CC Particle World", [W as f64, H as f64]);
    for (path, value) in [
        ("birthRate", Value::Scalar(2.0)),
        ("longevity", Value::Scalar(3.0)),
        ("producer/producerX", Value::Scalar(0.12)),
        ("producer/producerY", Value::Scalar(0.08)),
        ("producer/producerZ", Value::Scalar(0.25)),
        ("producer/radiusX", Value::Scalar(0.07)),
        ("producer/radiusY", Value::Scalar(0.06)),
        ("producer/radiusZ", Value::Scalar(0.13)),
        ("physics/velocity", Value::Scalar(0.0)),
        ("physics/gravity", Value::Scalar(0.0)),
        ("particle/particleType", Value::Enum(1)),
        ("particle/birthSize", Value::Scalar(0.2)),
        ("particle/deathSize", Value::Scalar(0.2)),
    ] {
        fx.prop_mut(path).unwrap().value = value;
    }
    particles.props.sub_mut("effects").unwrap().children.push(fx.into());
    let pid = particles.id;
    comp.layers.push(particles);
    let mut cam = build::layer(&mut project, &comp, "Camera", LayerSource::Camera, (W, H), None);
    set(&mut cam, "transform/position", Value::Vec3([48.0, 32.0, -192.0]));
    set(&mut cam, "transform/poi", Value::Vec3([48.0, 32.0, 0.0]));
    set(&mut cam, "cameraOptions/zoom", Value::Scalar(192.0));
    cam.switches.video = camera;
    let camid = cam.id;
    comp.layers.insert(0, cam);
    let cid = project.add_item("Comp", Default::default(), None, ItemKind::Comp(comp.into()));
    (project, cid, pid, camid)
}
fn frame(project: &Project, cid: ItemId, time: Tick, cache: Option<&LayerCache>) -> Image {
    let mut r = Renderer::new(project, &NoFootage, RenderOpts { backend: Backend::Cpu, motion_blur: false, ..Default::default() });
    r.cache = cache;
    r.comp_frame_cpu(cid, time)
}
fn finite(image: &Image) {
    assert!(image.data.iter().flatten().all(|v| v.is_finite()));
    assert!(image.data.iter().any(|p| p[0] + p[1] + p[2] > 0.05), "generated particles must be visible");
}
fn hash(image: &Image) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in image.data.iter().flatten() {
        h.write_u32(v.to_bits());
    }
    h.finish()
}
#[test]
fn explicit_camera_moves_and_rotates_generated_particles() {
    let (mut p, cid, _, camid) = scene(true);
    let time = Tick::from_seconds_f64(1.0);
    let base = frame(&p, cid, time, None);
    finite(&base);
    let cam = p.comp_mut(cid).unwrap().layer_mut(camid).unwrap();
    set(cam, "transform/position", Value::Vec3([68.0, 42.0, -192.0]));
    set(cam, "transform/poi", Value::Vec3([68.0, 42.0, 0.0]));
    let moved = frame(&p, cid, time, None);
    finite(&moved);
    assert!(base != moved, "moving the active comp camera must move Particle World");
    let cam = p.comp_mut(cid).unwrap().layer_mut(camid).unwrap();
    set(cam, "transform/position", Value::Vec3([48.0, 32.0, -192.0]));
    set(cam, "transform/poi", Value::Vec3([48.0, 32.0, 0.0]));
    set(cam, "transform/rotation", Value::Scalar(35.0));
    let rotated = frame(&p, cid, time, None);
    finite(&rotated);
    assert!(base != rotated, "camera rotation must rotate Particle World");
}
#[test]
fn cached_camera_and_transform_edits_match_uncached_frames() {
    let (mut p, cid, pid, camid) = scene(true);
    let time = Tick::from_seconds_f64(1.0);
    let cache = LayerCache::new(8 * 1024 * 1024);
    let base = frame(&p, cid, time, Some(&cache));
    assert_eq!(base, frame(&p, cid, time, Some(&cache)));
    assert!(cache.stats().hits > 0, "fixture must really reuse layer content");
    let check = |p: &Project, t: Tick| {
        let fresh = frame(p, cid, t, None);
        finite(&fresh);
        assert_eq!(frame(p, cid, t, Some(&cache)), fresh, "cached content must follow evaluated scene matrices");
        fresh
    };
    let cam = p.comp_mut(cid).unwrap().layer_mut(camid).unwrap();
    set(cam, "transform/position", Value::Vec3([66.0, 32.0, -192.0]));
    set(cam, "transform/poi", Value::Vec3([66.0, 32.0, 0.0]));
    assert_ne!(check(&p, time), base);
    p.comp_mut(cid).unwrap().layer_mut(camid).unwrap().switches.video = false;
    let disabled = check(&p, time);
    let (default, default_id, _, _) = scene(false);
    assert_eq!(disabled, frame(&default, default_id, time, None));
    p.comp_mut(cid).unwrap().layer_mut(camid).unwrap().switches.video = true;
    assert_ne!(check(&p, time), disabled);
    let cam = p.comp_mut(cid).unwrap().layer_mut(camid).unwrap();
    cam.props.prop_mut("transform/position").unwrap().keys =
        vec![Keyframe::new(Tick::ZERO, Value::Vec3([48.0, 32.0, -192.0])), Keyframe::new(Tick::from_seconds_f64(2.0), Value::Vec3([68.0, 36.0, -220.0]))];
    for t in [0.7, 1.3, 0.7] {
        check(&p, Tick::from_seconds_f64(t));
    }
    set(p.comp_mut(cid).unwrap().layer_mut(pid).unwrap(), "transform/position", Value::Vec3([54.0, 35.0, 0.0]));
    check(&p, time);
    let mut comp = p.comp(cid).unwrap().clone();
    let parent = build::layer(&mut p, &comp, "Parent", LayerSource::Null, (W, H), None);
    let parentid = parent.id;
    comp.layers.insert(0, parent);
    comp.layer_mut(pid).unwrap().parent = Some(parentid);
    *p.comp_mut(cid).unwrap() = comp;
    check(&p, time);
    set(p.comp_mut(cid).unwrap().layer_mut(parentid).unwrap(), "transform/position", Value::Vec3([55.0, 27.0, 0.0]));
    check(&p, time);
}
#[test]
fn disabled_absent_and_no_host_controls() {
    let (mut p, cid, _, camid) = scene(false);
    let time = Tick::from_seconds_f64(1.0);
    let disabled = frame(&p, cid, time, None);
    finite(&disabled);
    p.comp_mut(cid).unwrap().layers.retain(|l| l.id != camid);
    let absent = frame(&p, cid, time, None);
    assert_eq!(disabled, absent);
    let spec = effects::find(ID).unwrap();
    let params = Params { values: spec.params.iter().map(|v| (v.id.to_string(), effects::default_value(v, [W as f64, H as f64]))).collect() };
    let ctx = EffectCtx { params: &params, time: 1.0, layer_size: [W as f64, H as f64], seed: 17, adjustment: false, env: EffectEnv::default() };
    let no_host = (spec.render)(&ctx, effects::Buf { img: Image::filled(W, H, [0.0, 0.0, 0.0, 1.0]), offset: [0.0; 2], scale: 1.0 });
    finite(&no_host.img);
    eprintln!("PARTICLE_CONTROLS disabled={:016x} absent={:016x} no_host={:016x}", hash(&disabled), hash(&absent), hash(&no_host.img));
}
