//! Mesh Warp acceleration shares the CPU grid model, with a bounded native index fallback.
//! EFFECTCRAFT_TEST_REQUIRE_GPU=1 turns missing adapters into failures for proving runs.

#![cfg(not(target_arch = "wasm32"))]

use effectcraft_effects::{Buf, EffectCtx, EffectEnv, Params, WarpMesh};
use effectcraft_gpu::Gpu;
use effectcraft_keyframe::{TextDoc, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{BitDepth, Comp, ItemId, ItemKind, LayerSource, Project, Solid};
use effectcraft_raster::Image;
use effectcraft_render::{Accelerator, Backend, FxStep, NoFootage, RenderOpts, Renderer};
use effectcraft_time::{FrameRate, Tick};
use std::sync::OnceLock;
const MESH: &str = "ec.distort.meshwarp";
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| match Gpu::try_headless() {
        Ok(gpu) => {
            eprintln!("proving adapter: {}", gpu.name());
            assert!(gpu.supports_effect(MESH));
            Some(gpu)
        }
        Err(reason) => {
            assert!(std::env::var_os("EFFECTCRAFT_TEST_REQUIRE_GPU").is_none(), "REQUIRE_GPU: {reason}");
            eprintln!("no GPU adapter: {reason}; skipping comparisons");
            None
        }
    })
    .as_ref()
}

fn image(w: u32, h: u32) -> Image {
    let mut image = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            image.set(x, y, [x as f32 / w as f32, y as f32 / h as f32, 0.25, 1.0]);
        }
    }
    image
}
fn diff(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.data.iter().zip(&b.data).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0, f32::max)
}
fn params(rows: f64, columns: f64, quality: f64, raw: String) -> Params {
    let spec = effectcraft_effects::find(MESH).unwrap();
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_owned(), p.default.clone())).collect() };
    for (id, value) in [("rows", rows), ("columns", columns), ("quality", quality)] {
        params.values.insert(id.into(), Value::Scalar(value));
    }
    params.values.insert("mesh".into(), Value::Str(raw));
    params
}
fn case(name: &str, params: Params, centered: bool, scale: f64) {
    let spec = effectcraft_effects::find(MESH).unwrap();
    let origin = if centered { [-16.0, -12.0] } else { [0.0; 2] };
    let ctx = EffectCtx {
        params: &params,
        time: 0.0,
        layer_size: [32.0, 24.0],
        seed: 1,
        adjustment: false,
        env: EffectEnv { bounds_origin: origin, ..Default::default() },
    };
    let buf = Buf { img: image((32.0 * scale) as u32, (24.0 * scale) as u32), offset: [-origin[0] * scale, -origin[1] * scale], scale };
    let cpu = effectcraft_effects::apply(spec, &ctx, buf.clone());
    let steps = [FxStep { spec, ctx }];
    let output = gpu().unwrap().effects(&steps, &buf, None).expect("supported bounded Mesh Warp chain");
    let d = diff(&cpu.img, &output.img);
    eprintln!("{name}: centered={centered} scale={scale} max_diff={d}");
    assert!(d <= 1e-3, "{name}: CPU/GPU mismatch {d}");
}
fn control_mesh() -> WarpMesh {
    let mut mesh = WarpMesh::identity(1, 1);
    mesh.controls[0][0] = [4.0, 2.0];
    mesh.controls[1][0] = [-3.0, 4.0];
    mesh.controls[2][0] = [2.0, -3.0];
    mesh.controls[0][2] = [0.0, 9.0];
    mesh.controls[1][1] = [0.0, -9.0];
    mesh.controls[0][4] = [6.0, 0.0];
    mesh
}
#[test]
fn legacy_mesh_cpu_gpu_parity() {
    if gpu().is_none() {
        return;
    }
    for centered in [false, true] {
        case("legacy", params(2.0, 3.0, 1.0, "0,0 0,0 0,0 0,0 0,0 6,-4 -5,3 0,0 0,0 0,0 0,0 0,0".into()), centered, 1.0);
    }
}
#[test]
fn versioned_coons_vertices_and_tangents_match_cpu() {
    if gpu().is_none() {
        return;
    }
    for quality in [1.0, 8.0] {
        for centered in [false, true] {
            for scale in [1.0, 0.5] {
                case("coons", params(1.0, 1.0, quality, control_mesh().format()), centered, scale);
            }
        }
    }
}
#[test]
fn opposing_tangents_are_visible_at_quality_one() {
    if gpu().is_none() {
        return;
    }
    let mut mesh = WarpMesh::identity(1, 1);
    mesh.controls[0][2] = [0.0, 12.0];
    mesh.controls[1][1] = [0.0, -12.0];
    assert!(mesh.sample(1.0 / 3.0, 0.0, [32.0, 24.0])[1] > 2.0);
    case("opposing tangents", params(1.0, 1.0, 1.0, mesh.format()), false, 1.0);
}
#[test]
fn invalid_dimensions_and_malformed_data_match_cpu_repairs() {
    if gpu().is_none() {
        return;
    }
    let mut mesh = WarpMesh::identity(1, 1);
    for vertex in &mut mesh.controls {
        vertex[0] = [4.0, 0.0];
    }
    case("nonfinite dimensions", params(f64::NAN, f64::INFINITY, f64::NAN, mesh.format()), false, 1.0);
    for raw in ["mw1;99999999999999999999,1;", "mw1;0,0;", "mw1;1,1;NaN,0"] {
        case("malformed", params(1.0, 1.0, 1.0, raw.into()), false, 1.0);
    }
}
fn scene(kind: &str) -> (Project, ItemId) {
    let mut project = Project::default();
    project.settings.bit_depth = BitDepth::Bpc32;
    let comp = Comp::new(97, 61, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    let source = match kind {
        "shape" => LayerSource::Shape,
        "text" => LayerSource::Text,
        _ => {
            let solid = project.add_item(
                "Orange",
                Default::default(),
                None,
                ItemKind::Solid(Solid { width: 97, height: 61, color: [0.9, 0.3, 0.1], pixel_aspect: 1.0 }),
            );
            LayerSource::Solid { item: solid }
        }
    };
    let mut layer = build::layer(&mut project, &comp, "Mesh", source, (97, 61), None);
    let mut next = project.next_id;
    if kind == "shape" {
        let mut ids = Ids(&mut next);
        let rect = build::shape_rect(&mut ids, [90.0, 54.0], [0.0, 0.0], 0.0);
        let fill = build::shape_fill(&mut ids, [0.9, 0.3, 0.1, 1.0]);
        let group = build::shape_group(&mut ids, "Rectangle", vec![rect, fill]);
        layer.props.sub_mut("contents").unwrap().children.push(group.into());
    } else if kind == "text" {
        layer.props.prop_mut("text/sourceText").unwrap().value = Value::Text(Box::new(TextDoc { text: "HI".into(), size: 30.0, ..Default::default() }));
        layer.props.prop_mut("transform/position").unwrap().value = Value::Vec3([20.0, 45.0, 0.0]);
    }
    for id in [MESH, "ec.channel.invert"] {
        let spec = effectcraft_effects::find(id).unwrap();
        let mut group = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), spec.name, [97.0, 61.0]);
        if id == MESH {
            for id in ["rows", "columns"] {
                group.get_mut(id).unwrap().value = Value::Scalar(1.0);
            }
            group.get_mut("mesh").unwrap().value = Value::Str(control_mesh().format());
        }
        layer.props.sub_mut("effects").unwrap().children.push(group.into());
    }
    project.next_id = next;
    let mut comp = comp;
    comp.layers.push(layer);
    let cid = project.add_item("Mesh", Default::default(), None, ItemKind::Comp(comp.into()));
    (project, cid)
}
#[test]
fn full_gpu_compositor_matches_solid_shape_and_text() {
    if gpu().is_none() {
        return;
    }
    for kind in ["solid", "shape", "text"] {
        let (project, cid) = scene(kind);
        let opts = RenderOpts { motion_blur: false, backend: Backend::Cpu, ..Default::default() };
        let cpu = Renderer::new(&project, &NoFootage, opts).comp_frame_cpu(cid, Tick::ZERO);
        assert!(cpu.data.iter().any(|p| p[3] > 0.5), "{kind} must contain visible authored content");
        let mut renderer = Renderer::new(&project, &NoFootage, RenderOpts { backend: Backend::Gpu, ..opts });
        renderer.accel = gpu().map(|g| g as &dyn Accelerator);
        let output = gpu().unwrap().render(&renderer, cid, Tick::ZERO).expect("GPU compositor");
        let d = diff(&cpu, &output);
        eprintln!("full {kind}: max_diff={d}");
        assert!(d <= 1e-3, "{kind} CPU/GPU mismatch {d}");
    }
}
#[test]
fn folded_maximum_mesh_rejects_row_index_growth() {
    if gpu().is_none() {
        return;
    }
    let mut mesh = WarpMesh::identity(31, 31);
    for (index, vertex) in mesh.controls.iter_mut().enumerate() {
        vertex[0][1] = if (index / 32) % 2 == 0 { -1000.0 } else { 1000.0 };
    }
    let params = params(31.0, 31.0, 10.0, mesh.format());
    let spec = effectcraft_effects::find(MESH).unwrap();
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [64.0, 512.0], seed: 1, adjustment: false, env: EffectEnv::default() };
    let buf = Buf { img: image(64, 512), offset: [0.0; 2], scale: 1.0 };
    assert!(gpu().unwrap().effects(&[FxStep { spec, ctx }], &buf, None).is_none(), "oversized row indexing must request the renderer's bounded CPU fallback");
}

#[test]
fn finite_coordinates_beyond_f32_range_request_cpu_fallback() {
    if gpu().is_none() {
        return;
    }
    let spec = effectcraft_effects::find(MESH).unwrap();
    let mut mesh = WarpMesh::identity(1, 1);
    mesh.controls[0][0] = [-1e40, -1e40];
    mesh.controls[1][0] = [1e40, -1e40];
    mesh.controls[2][0] = [-1e40, 1e40];
    mesh.controls[3][0] = [1e40, 1e40];
    assert_eq!(WarpMesh::parse(&mesh.format(), 1, 1), mesh);
    let params = params(1.0, 1.0, 1.0, mesh.format());
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [32.0, 24.0], seed: 1, adjustment: false, env: EffectEnv::default() };
    let buf = Buf { img: image(32, 24), offset: [0.0; 2], scale: 1.0 };
    let cpu = effectcraft_effects::apply(spec, &ctx, buf.clone());
    assert!(cpu.img.data.iter().flatten().all(|v| v.is_finite()), "finite CPU output before any kernel runs");
    assert!(cpu.img.get(16, 12)[3] > 0.9, "the CPU mesh covers the image");
    assert!(gpu().unwrap().effects(&[FxStep { spec, ctx }], &buf, None).is_none(), "unrepresentable GPU triangle data must request CPU fallback");
}
fn extreme_mesh(amplitude: f64) -> WarpMesh {
    let mut mesh = WarpMesh::identity(1, 1);
    mesh.controls[0][0] = [-amplitude, -amplitude];
    mesh.controls[1][0] = [amplitude, -amplitude];
    mesh.controls[2][0] = [-amplitude, amplitude];
    mesh.controls[3][0] = [amplitude, amplitude];
    mesh
}

#[test]
fn cpu_extreme_finite_controls_never_emit_nonfinite_pixels() {
    let spec = effectcraft_effects::find(MESH).unwrap();
    let mesh = extreme_mesh(2e155);
    assert_eq!(WarpMesh::parse(&mesh.format(), 1, 1), mesh);
    let params = params(1.0, 1.0, 1.0, mesh.format());
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [32.0, 24.0], seed: 1, adjustment: false, env: EffectEnv::default() };
    let buf = Buf { img: image(32, 24), offset: [0.0; 2], scale: 1.0 };
    let cpu = effectcraft_effects::apply(spec, &ctx, buf);
    let nonfinite = cpu.img.data.iter().flatten().filter(|v| !v.is_finite()).count();
    eprintln!("CPU extreme controls: nonfinite={nonfinite} center={:?}", cpu.img.get(16, 12));
    assert_eq!(nonfinite, 0, "finite control data must not contaminate image pixels");
}

#[test]
fn gpu_subnormal_coefficients_match_cpu_or_request_fallback() {
    let Some(gpu) = gpu() else { return };
    let spec = effectcraft_effects::find(MESH).unwrap();
    let mesh = extreme_mesh(2e38);
    assert_eq!(WarpMesh::parse(&mesh.format(), 1, 1), mesh);
    let params = params(1.0, 1.0, 1.0, mesh.format());
    let ctx = EffectCtx { params: &params, time: 0.0, layer_size: [32.0, 24.0], seed: 1, adjustment: false, env: EffectEnv::default() };
    let buf = Buf { img: image(32, 24), offset: [0.0; 2], scale: 1.0 };
    let cpu = effectcraft_effects::apply(spec, &ctx, buf.clone());
    assert!(cpu.img.data.iter().flatten().all(|v| v.is_finite()));
    assert!(cpu.img.get(16, 12)[3] > 0.9);
    if let Some(output) = gpu.effects(&[FxStep { spec, ctx }], &buf, None) {
        assert!(output.img.data.iter().flatten().all(|v| v.is_finite()));
        let d = diff(&cpu.img, &output.img);
        eprintln!("GPU subnormal coefficients: max_diff={d} CPUcenter={:?} GPUcenter={:?}", cpu.img.get(16, 12), output.img.get(16, 12));
        assert!(d <= 1e-3, "subnormal coefficient GPU path must match CPU: {d}");
    } else {
        eprintln!("GPU subnormal coefficients: bounded CPU fallback");
    }
}
