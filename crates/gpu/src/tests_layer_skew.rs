//! Actual GPU placement for the new 2D layer transform, with independent pixel samples.
use crate::tests::{Scene, gpu, set};
use effectcraft_keyframe::Value;
use effectcraft_project::BitDepth;
use effectcraft_render::{NoFootage, Renderer};
use effectcraft_time::Tick;
#[test]
fn two_d_layer_skew_and_axis_reach_native_gpu_placement() {
    let Some(g) = gpu() else {
        assert_ne!(std::env::var("EFFECTCRAFT_TEST_REQUIRE_GPU").ok().as_deref(), Some("1"), "a native GPU adapter is required for this proof");
        return;
    };
    eprintln!("proving layer Skew adapter: {}", g.ctx.name);
    for (skew, axis, inside, outside) in [
        (45.0, 0.0, [(41, 29), (22, 34)], [(20, 29), (42, 34)]),
        (-45.0, 0.0, [(22, 29), (41, 34)], [(42, 29), (20, 34)]),
        (45.0, 450.0, [(26, 23), (37, 39)], [(25, 35), (38, 24)]),
    ] {
        let mut s = Scene::new(BitDepth::Bpc8);
        let mut l = s.solid([0.0, 1.0, 0.0], 16, 8);
        set(&mut l, "transform/position", Value::Vec3([32.0, 32.0, 0.0]));
        set(&mut l, "transform/skew", Value::Scalar(skew));
        set(&mut l, "transform/skewAxis", Value::Scalar(axis));
        s.push(l);
        let r = Renderer::new(&s.p, &NoFootage, Default::default());
        let cpu = r.comp_frame_cpu(s.cid, Tick::ZERO);
        let actual = g.render(&r, s.cid, Tick::ZERO).expect("the actual GPU must render layer Skew without a CPU fallback");
        assert_eq!((actual.width, actual.height), (97, 61));
        assert!(actual.data.iter().flatten().all(|c| c.is_finite()));
        for (x, y) in inside {
            let pixel = actual.get(x, y);
            assert!(pixel[1] > 0.9 && pixel[3] > 0.9 && pixel[0] < 0.01 && pixel[2] < 0.01, "independent inside ({x},{y}): {pixel:?}");
        }
        for (x, y) in outside {
            assert!(actual.get(x, y)[3] < 0.01, "independent outside ({x},{y})");
        }
        let error = actual.data.iter().zip(&cpu.data).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0_f32, f32::max);
        assert!(error <= 1.0 / 255.0 + 1e-5, "actual GPU/CPU placement error {error} for skew {skew} axis {axis}");
    }
}
