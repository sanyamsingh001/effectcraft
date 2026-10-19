//! GPU particle render passes (`fx_particles`) vs the CPU effects (the oracle): the frame's
//! plan is shared, the raster runs on each side. Direct on a buffer (full and half resolution)
//! and composited at 8 and 32 bpc (`tests_fx_sim`'s harness).

use effectcraft_keyframe::Value;

use crate::tests::n;
use crate::tests_fx_sim::{case, composited, e, off, on};

fn v2(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

#[test]
fn particle_world_types_and_modes() {
    for kind in 0..9 {
        case(
            "ec.sim.ccparticleworld",
            &[("birthRate", n(1.5)), ("particle/particleType", e(kind)), ("particle/transferMode", e(kind % 4)), ("particle/birthSize", n(0.3))],
            1.0,
            0.0,
        );
    }
    for anim in [1, 2, 5, 6, 8] {
        case("ec.sim.ccparticleworld", &[("physics/animation", e(anim)), ("particle/particleType", e(1)), ("particle/sizeVariation", n(40.0))], 0.8, 0.0);
    }
    // Nothing born yet: the layer as is.
    case("ec.sim.ccparticleworld", &[("birthRate", n(0.0))], 0.0, 0.0);
}

#[test]
fn particle_systems_ii_types_and_modes() {
    for kind in 0..9 {
        case("ec.sim.ccparticlesystems2", &[("birthRate", n(3.0)), ("particle/particleType", e(kind)), ("particle/transferMode", e(kind % 4))], 1.0, 0.0);
    }
    case("ec.sim.ccparticlesystems2", &[("physics/animation", e(4)), ("producer/radiusX", n(8.0)), ("physics/direction", n(70.0))], 1.3, 0.0);
}

/// A Particle Playground grid (cannon off) plus `more`.
fn grid(more: &[(&'static str, Value)]) -> Vec<(&'static str, Value)> {
    let mut g = vec![
        ("cannonEnabled", off()),
        ("gridEnabled", on()),
        ("grid/gridPosition", v2(35.0, 22.0)),
        ("grid/gridWidth", n(40.0)),
        ("grid/gridHeight", n(20.0)),
        ("grid/particlesAcross", n(5.0)),
        ("grid/particlesDown", n(3.0)),
        ("grid/gridParticleRadius", n(3.0)),
        ("gravity/gravityForce", n(20.0)),
    ];
    g.extend(more.iter().cloned());
    g
}

#[test]
fn particle_playground() {
    case("ec.sim.particleplayground", &[], 1.0, 0.0);
    case(
        "ec.sim.particleplayground",
        &[("cannon/cannonPosition", v2(20.0, 40.0)), ("cannon/particlesPerSecond", n(150.0)), ("cannon/velocity", n(90.0)), ("gravity/gravityForce", n(40.0))],
        1.2,
        0.0,
    );
    case("ec.sim.particleplayground", &grid(&[]), 0.7, 0.0);
    case(
        "ec.sim.particleplayground",
        &grid(&[("options/gridText", Value::Str("Ab C".into())), ("grid/gridParticleRadius", n(12.0)), ("options/autoOrientRotation", on())]),
        0.6,
        0.0,
    );
    case("ec.sim.particleplayground", &grid(&[("repel/repelForce", n(5.0)), ("repel/repelForceRadius", n(12.0))]), 0.9, 0.0);
    composited(
        "ec.sim.particleplayground",
        &[
            ("cannonEnabled", off()),
            ("exploderEnabled", on()),
            ("layerExploder/explodeLayer", Value::Layer(Some(0))),
            ("layerExploder/radiusOfNewParticles", n(3.0)),
        ],
        0.5,
        0.0,
    );
    // Layer Map: every grid particle shows the other layer, at frames set by the offset type.
    for (kind, offset) in [(0, 0.0), (2, 0.5), (3, 1.0)] {
        composited(
            "ec.sim.particleplayground",
            &grid(&[
                ("layerMapEnabled", on()),
                ("layerMap/layerMapLayer", Value::Layer(Some(0))),
                ("layerMap/timeOffsetType", e(kind)),
                ("layerMap/timeOffset", n(offset)),
                ("grid/particlesAcross", n(3.0)),
                ("grid/particlesDown", n(2.0)),
            ]),
            0.8,
            0.0,
        );
    }
    composited(
        "ec.sim.particleplayground",
        &grid(&[
            ("mapperEnabled", on()),
            ("persistentPropertyMapper/useLayerAsMap", Value::Layer(Some(0))),
            ("persistentPropertyMapper/mapRedTo", e(8)),
            ("persistentPropertyMapper/redMin", n(-50.0)),
            ("persistentPropertyMapper/redMax", n(50.0)),
        ]),
        0.7,
        0.0,
    );
}

#[test]
fn ball_action_pixel_polly_scatterize() {
    case("ec.sim.ccballaction", &[], 0.0, 0.0);
    case(
        "ec.sim.ccballaction",
        &[("scatter", n(20.0)), ("rotationAxis", e(6)), ("rotation", n(40.0)), ("twistProperty", e(3)), ("twistAngle", n(90.0))],
        0.0,
        0.0,
    );
    case("ec.sim.ccballaction", &[("gridSpacing", n(2.0)), ("ballSize", n(80.0)), ("instabilityState", n(45.0)), ("twistProperty", e(2))], 0.0, 0.0);
    for object in 0..4 {
        case("ec.sim.ccpixelpolly", &[("object", e(object)), ("gridSpacing", n(6.0))], 0.6, 0.0);
    }
    case("ec.sim.ccpixelpolly", &[("enableDepthSort", off()), ("spinning", n(90.0)), ("gravity", n(2.0))], 1.0, 0.0);
    case("ec.sim.ccpixelpolly", &[("startTime", n(1.0))], 0.5, 0.0);
    case("ec.sim.ccscatterize", &[("scatter", n(10.0))], 0.0, 0.0);
    case("ec.sim.ccscatterize", &[("rightTwist", n(60.0)), ("leftTwist", n(-30.0)), ("transferMode", e(1)), ("scatter", n(4.0))], 0.0, 0.0);
    case("ec.sim.ccscatterize", &[], 0.0, 0.0);
}

#[test]
fn curl_noise() {
    case("ec.noise.curlnoise", &[], 0.0, 0.0);
    case("ec.noise.curlnoise", &[("view", e(1))], 0.0, 0.0);
    case(
        "ec.noise.curlnoise",
        &[("edgeBehavior", e(1)), ("steps", n(8.0)), ("displacementAmount", n(60.0)), ("scale", n(20.0)), ("rotation", n(30.0)), ("evolution", n(90.0))],
        0.0,
        0.0,
    );
    case("ec.noise.curlnoise", &[("displacementAmount", n(0.0))], 0.0, 0.0);
}

#[test]
fn particle_world_active_camera_matches_cpu() {
    use crate::tests::{Scene, check, diff, gpu, set};
    use effectcraft_project::{BitDepth, Comp, LayerSource, build};
    use effectcraft_render::{Accelerator, Backend, NoFootage, RenderOpts, Renderer};
    use effectcraft_time::{FrameRate, Tick};
    let Some(gpu) = gpu() else { return };
    assert!(Accelerator::supports_effect(gpu, "ec.sim.ccparticleworld"));
    let mut s = Scene::new(BitDepth::Bpc32);
    s.comp = Comp::new(96, 64, FrameRate::FPS_30, Tick::from_seconds_f64(2.5));
    *s.p.comp_mut(s.cid).unwrap() = s.comp.clone();
    let mut particles = s.solid([0.0; 3], 96, 64);
    s.effect(
        &mut particles,
        "ec.sim.ccparticleworld",
        &[
            ("birthRate", n(2.0)),
            ("longevity", n(3.0)),
            ("producer/producerX", n(0.12)),
            ("producer/producerY", n(0.08)),
            ("producer/producerZ", n(0.25)),
            ("producer/radiusX", n(0.07)),
            ("producer/radiusY", n(0.06)),
            ("producer/radiusZ", n(0.13)),
            ("physics/velocity", n(0.0)),
            ("physics/gravity", n(0.0)),
            ("particle/particleType", e(1)),
            ("particle/birthSize", n(0.2)),
            ("particle/deathSize", n(0.2)),
        ],
    );
    s.push(particles);
    let mut camera = build::layer(&mut s.p, &s.comp, "Camera", LayerSource::Camera, (96, 64), None);
    set(&mut camera, "cameraOptions/zoom", n(192.0));
    let camera_id = s.push(camera);
    let time = Tick::from_seconds_f64(1.0);
    for (x, rotation, enabled) in [(48.0, 0.0, true), (65.0, 0.0, true), (48.0, 35.0, true), (48.0, 0.0, false)] {
        let camera = s.p.comp_mut(s.cid).unwrap().layer_mut(camera_id).unwrap();
        camera.switches.video = enabled;
        set(camera, "transform/position", Value::Vec3([x, 32.0, -192.0]));
        set(camera, "transform/poi", Value::Vec3([x, 32.0, 0.0]));
        set(camera, "transform/rotation", n(rotation));
        let options = RenderOpts { motion_blur: false, ..Default::default() };
        let cpu = Renderer::new(&s.p, &NoFootage, RenderOpts { backend: Backend::Cpu, ..options }).comp_frame_cpu(s.cid, time);
        let mut renderer = Renderer::new(&s.p, &NoFootage, RenderOpts { backend: Backend::Gpu, ..options });
        renderer.accel = Some(gpu);
        let actual = gpu.render(&renderer, s.cid, time).expect("active camera GPU compositor");
        for image in [&cpu, &actual] {
            assert!(image.data.iter().flatten().all(|v| v.is_finite()));
            assert!(image.data.iter().any(|p| p[0] + p[1] + p[2] > 0.05), "generated particles must be visible");
        }
        check(&format!("Particle World camera x={x} rotation={rotation} enabled={enabled}"), Some(diff(&cpu, &actual, 1e-3)), 0.0);
    }
}
