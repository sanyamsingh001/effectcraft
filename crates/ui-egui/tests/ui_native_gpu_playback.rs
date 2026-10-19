//! Plain native views can retain GPU textures (#304), with explicit readback for consumers.
//! EFFECTCRAFT_TEST_REQUIRE_GPU=1 makes missing adapters fail the proving run.

#![cfg(not(target_arch = "wasm32"))]

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui_kittest::Harness;
use serde_json::json;

fn available_gpu() -> bool {
    match effectcraft_gpu::Gpu::try_headless() {
        Ok(gpu) => {
            eprintln!("proving adapter: {}", effectcraft_engine::render::Accelerator::name(&gpu));
            true
        }
        Err(reason) => {
            assert!(std::env::var_os("EFFECTCRAFT_TEST_REQUIRE_GPU").is_none(), "GPU proving run needs an adapter: {reason}");
            eprintln!("no GPU adapter: {reason}; skipped");
            false
        }
    }
}

fn session() -> Session {
    let mut session = Session::default();
    session.execute("comp.new", json!({"name": "Native Texture", "width": 96, "height": 64, "frameRate": 25, "duration": 0.04})).unwrap();
    session.execute("layer.newSolid", json!({"color": "#402080", "width": 96, "height": 64})).unwrap();
    session.execute("layer.select", json!({"layers": []})).unwrap();
    session
}

fn settle(h: &mut Harness<'_, EffectcraftApp>) {
    for _ in 0..600 {
        h.step();
        if h.state().frames.inflight() == 0 && h.state().frames.last_ms.lock().is_ok_and(|ms| *ms > 0.0) {
            h.run_steps(4);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the native viewer frame did not finish");
}

fn sample(h: &mut Harness<'_, EffectcraftApp>) -> [u8; 4] {
    let comp = h.state().auto.find("viewer.comp").expect("composition rectangle");
    let point = [comp.rect[0] + comp.rect[2] * 0.65, comp.rect[1] + comp.rect[3] * 0.65];
    let image = h.render().expect("rendered viewer");
    image.get_pixel(point[0].round() as u32, point[1].round() as u32).0
}

fn assert_pixels_close(a: &egui::ColorImage, b: &egui::ColorImage) {
    assert_eq!(a.size, b.size);
    for (a, b) in a.pixels.iter().zip(&b.pixels) {
        assert!(a.to_array().into_iter().zip(b.to_array()).all(|(a, b)| a.abs_diff(b) <= 2), "GPU and CPU pixels disagree: {a:?}, {b:?}");
    }
}

#[test]
fn native_plain_view_uses_gpu_textures_and_keeps_display_controls_correct() {
    if !available_gpu() {
        return;
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| EffectcraftApp::new(session()));
    settle(&mut h);
    assert!(h.state().gpu_adapter().is_some(), "the egui device must support the compositor");
    assert!(h.state().viewer_on_gpu(), "plain native view should keep its GPU texture without mandatory CPU transfer");
    let gpu = h.state().render_source().gpu.expect("native compositor device");
    assert_eq!(gpu.context().transfer_stats().readbacks, 0, "a plain solid viewer frame should not transfer compositor pixels to the CPU");
    let gpu_pixels = h.state_mut().viewer_pixels().expect("on-demand native sampling readback");
    assert_eq!(gpu.context().transfer_stats().readbacks, 1, "explicit sampling reads the GPU texture once");
    for (channel, stops) in [("rgb", 0), ("red", 0), ("rgb", 1)] {
        h.state_mut().session.execute("view.channel", json!({"channel": channel, "colorized": false})).unwrap();
        h.state_mut().session.execute("view.exposure", json!({"stops": stops})).unwrap();
        h.run_steps(3);
        let texture_sample = sample(&mut h);
        h.state_mut().session.execute("render.backend", json!({"backend": "cpu"})).unwrap();
        settle(&mut h);
        assert!(!h.state().viewer_on_gpu(), "Software Only stays on the CPU");
        let cpu_pixels = h.state_mut().viewer_pixels().expect("CPU pixels");
        assert_pixels_close(&gpu_pixels, &cpu_pixels);
        let cpu_sample = sample(&mut h);
        assert!(
            texture_sample.into_iter().zip(cpu_sample).all(|(a, b)| a.abs_diff(b) <= 2),
            "{channel}, exposure {stops}: {texture_sample:?} vs {cpu_sample:?}"
        );
        h.state_mut().session.execute("render.backend", json!({"backend": "gpu"})).unwrap();
        settle(&mut h);
    }
    h.state_mut().session.execute("view.channel", json!({"channel": "rgb"})).unwrap();
    h.state_mut().session.execute("view.exposure", json!({"stops": 0})).unwrap();
    h.state_mut().session.execute("view.setRegionOfInterest", json!({"rect": [24, 16, 48, 32]})).unwrap();
    settle(&mut h);
    assert!(!h.state().viewer_on_gpu(), "ROI keeps its existing CPU texture placement");
    let cropped = h.state_mut().viewer_pixels().expect("ROI pixels");
    assert_eq!(cropped.size, [gpu_pixels.size[0] / 2, gpu_pixels.size[1] / 2]);
    h.state_mut().session.execute("view.setRegionOfInterest", json!({"rect": null})).unwrap();
    settle(&mut h);
    assert!(h.state().viewer_on_gpu(), "full frame restores the cached GPU texture");
    assert_pixels_close(&gpu_pixels, &h.state_mut().viewer_pixels().expect("restored full frame"));
}

#[test]
fn native_disk_cache_keeps_its_frame_persistence_path() {
    if !available_gpu() {
        return;
    }
    use effectcraft_engine::render::disk_cache::{self, Kind};
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-out/native-gpu-disk-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let disk = effectcraft_engine::render::disk_cache::DiskCache::open(&directory, 1 << 20).unwrap();
    assert!(disk.keys(Kind::Frame).is_empty(), "this test needs a fresh frame cache");
    let mut session = session();
    session.disk_cache = Some(disk.clone());
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| EffectcraftApp::new(session));
    settle(&mut h);
    assert!(h.state().gpu_adapter().is_some(), "the egui device must support the compositor");
    assert!(!h.state().viewer_on_gpu(), "enabled native disk persistence retains CPU frames");
    let comp = h.state().session.active_comp_id().unwrap();
    let series = h.state().shown_series(comp);
    let content = disk_cache::comp_content_key(&h.state().session.project, comp);
    let key = disk_cache::frame_key(content, series.frame, series.scale, series.view ^ series.opts);
    let pixels = h.state_mut().viewer_pixels().expect("current CPU viewer frame");
    let expected = pixels.pixels.iter().flat_map(|p| p.to_array()).collect::<Vec<_>>();
    for _ in 0..200 {
        if let Some(frame) = disk.get_frame(key) {
            assert!(disk.keys(Kind::Frame).contains(&key), "the current frame must be indexed");
            assert_eq!([frame.width as usize, frame.height as usize], pixels.size);
            assert_eq!(frame.rgba, expected, "the freshly persisted frame must match the viewer pixels");
            drop(h);
            disk.flush();
            drop(disk);
            std::fs::remove_dir_all(directory).unwrap();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the current viewer frame was not persisted");
}

#[test]
fn changing_native_disk_policy_separates_cached_output_and_persists_the_current_frame() {
    if !available_gpu() {
        return;
    }
    use effectcraft_engine::render::disk_cache::{self, Kind};
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-out/native-gpu-toggle-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| EffectcraftApp::new(session()));
    settle(&mut h);
    assert!(h.state().viewer_on_gpu(), "the initial frame must exercise native texture caching");
    let comp = h.state().session.active_comp_id().unwrap();
    let texture_key = h.state().shown_series(comp);
    let texture_pixels = h.state_mut().viewer_pixels().expect("initial GPU pixels");
    h.state_mut().session.execute_checked("prefs.set", json!({"values": {"disk.diskCacheFolder": directory, "disk.diskCacheEnabled": true}})).unwrap();
    let disk = h.state().session.disk_cache.clone().expect("newly enabled native disk cache");
    assert!(disk.keys(Kind::Frame).is_empty(), "the newly enabled cache starts without frames");
    settle(&mut h);
    assert!(!h.state().viewer_on_gpu(), "enabling disk persistence must select its CPU frame path even when the GPU frame was cached");
    let disk_key = h.state().shown_series(comp);
    assert_ne!(disk_key, texture_key, "different output policies must not share a cached frame or an in-flight result");
    let cpu_pixels = h.state_mut().viewer_pixels().expect("current CPU frame");
    assert_pixels_close(&texture_pixels, &cpu_pixels);
    let content = disk_cache::comp_content_key(&h.state().session.project, comp);
    let key = disk_cache::frame_key(content, disk_key.frame, disk_key.scale, disk_key.view ^ disk_key.opts);
    disk.flush();
    let stored = disk.get_frame(key).expect("the current frame must be persisted after enabling disk cache");
    assert!(disk.keys(Kind::Frame).contains(&key));
    assert_eq!(stored.rgba, cpu_pixels.pixels.iter().flat_map(|p| p.to_array()).collect::<Vec<_>>());
    h.state_mut().session.execute_checked("prefs.set", json!({"key": "disk.diskCacheEnabled", "value": false})).unwrap();
    settle(&mut h);
    assert!(h.state().viewer_on_gpu(), "disabling disk persistence can restore the cached native GPU texture");
    assert_eq!(h.state().shown_series(comp), texture_key);
    assert_pixels_close(&texture_pixels, &h.state_mut().viewer_pixels().expect("restored GPU pixels"));
    drop(h);
    disk.flush();
    drop(disk);
    std::fs::remove_dir_all(directory).unwrap();
}
