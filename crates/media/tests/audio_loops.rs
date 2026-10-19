//! Footage loop playback through the real native audio provider and mixdown.
#![cfg(not(target_arch = "wasm32"))]
use effectcraft_media::{MediaPool, probe_bytes};
use effectcraft_project::{Comp, Footage, FootageKind, ItemId, ItemKind, LayerSource, Project, build};
use effectcraft_render::audio::mix_comp;
use effectcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};
use std::sync::Arc;

const RATE: u32 = 8000;
fn wav() -> (Arc<[u8]>, Vec<f32>) {
    let samples: Vec<f32> = (0..2 * RATE)
        .flat_map(|i| {
            let left = if i < RATE { 0.25 } else { 0.75 };
            [left, -left * 0.5]
        })
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 8).to_le_bytes());
    bytes.extend(8u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    (bytes.into(), samples)
}
fn fixture(kind: FootageKind, loops: u32) -> (MediaPool, Footage, Vec<f32>) {
    let (bytes, samples) = wav();
    let mut footage = probe_bytes("synthetic-loop.wav", bytes.clone()).unwrap();
    assert_eq!(footage.duration, Tick::from_seconds_f64(2.0));
    assert!(footage.has_audio);
    footage.kind = kind;
    footage.loop_count = loops;
    let pool = MediaPool::new();
    pool.add_bytes(&footage.path, bytes);
    (pool, footage, samples)
}
fn expected(samples: &[f32], loops: usize) -> Vec<f32> {
    let mut expected = Vec::new();
    for _ in 0..loops {
        expected.extend_from_slice(samples);
    }
    expected.resize(5 * RATE as usize * 2, 0.0);
    expected
}
fn scene(footage: Footage) -> (Project, ItemId) {
    let mut project = Project::default();
    let item = project.add_item("Audio", Default::default(), None, ItemKind::Footage(footage));
    let mut comp = Comp::new(16, 16, FrameRate::FPS_30, Tick::from_seconds_f64(5.0));
    let mut layer = build::layer(&mut project, &comp, "Audio", LayerSource::Footage { item }, (16, 16), None);
    layer.out_point = comp.duration;
    comp.layers.push(layer);
    let id = project.add_item("Audio", Default::default(), None, ItemKind::Comp(comp.into()));
    (project, id)
}
#[test]
fn native_audio_loop_repeats_twice_and_stops_at_the_limit() {
    for kind in [FootageKind::Audio, FootageKind::Video] {
        let (control_pool, control, samples) = fixture(kind, 1);
        let unlooped = control_pool.audio_samples(&control, Tick::ZERO, 5 * RATE as usize, RATE);
        assert_eq!(unlooped, expected(&samples, 1), "native unlooped PCM control");
        let (pool, footage, _) = fixture(kind, 2);
        let looped = pool.audio_samples(&footage, Tick::ZERO, 5 * RATE as usize, RATE);
        assert!(looped.iter().all(|sample| sample.is_finite()));
        assert_eq!(&looped[..samples.len()], samples, "first native loop must decode");
        assert_eq!(&looped[samples.len()..samples.len() + 2], &samples[..2], "the second loop must restart the native PCM stream");
        assert!(looped == expected(&samples, 2), "{kind:?} must repeat the native PCM stream twice, then fall silent");
        eprintln!("{kind:?}: first and second 2s streams equal; tail after 4s silent; nonfinite=0");
    }
}
#[test]
fn repeated_native_audio_matches_mixdown_blocks_and_negative_preroll() {
    let (pool, footage, samples) = fixture(FootageKind::Audio, 2);
    let (project, comp) = scene(footage.clone());
    let frames = 5 * RATE as usize;
    let long = mix_comp(&project, &pool, None, comp, Tick::ZERO, frames, RATE);
    assert_eq!(long, expected(&samples, 2));
    let mut blocks = Vec::new();
    for at in (0..frames).step_by(257) {
        let start = Tick(at as i64 * (TICKS_PER_SECOND / i64::from(RATE)));
        blocks.extend(mix_comp(&project, &pool, None, comp, start, 257.min(frames - at), RATE));
    }
    assert_eq!(blocks, long, "audio-device blocks must agree across loop and stop boundaries");
    let preroll = pool.audio_samples(&footage, Tick(-TICKS_PER_SECOND / 4), (RATE as usize * 9) / 2, RATE);
    let mut wanted = vec![0.0; RATE as usize / 4 * 2];
    wanted.extend_from_slice(&samples);
    wanted.extend_from_slice(&samples);
    wanted.resize(preroll.len(), 0.0);
    assert_eq!(preroll, wanted, "negative source time and time after the final loop remain silent");
    eprintln!("mixdown: 257-frame blocks match whole mix; finite repeated audio and silent preroll/tail");
}
#[test]
fn cached_native_pcm_uses_the_same_loop_boundaries() {
    let directory = std::env::temp_dir().join(format!("effectcraft-audio-loops-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("cached-loop.wav");
    let (bytes, samples) = wav();
    std::fs::write(&path, &bytes).unwrap();
    let mut footage = probe_bytes(path.to_str().unwrap(), bytes).unwrap();
    footage.loop_count = 2;
    let pool = MediaPool::new();
    pool.set_conform_folder(Some(directory.join("conformed")));
    let cached = pool.conformed_path(&footage.path, RATE).unwrap();
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    let raw: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
    std::fs::write(&cached, raw).unwrap();
    let output = pool.audio_samples(&footage, Tick::ZERO, 5 * RATE as usize, RATE);
    assert_eq!(output, expected(&samples, 2), "native conformed PCM must repeat through the same provider boundary");
    assert!(output.iter().all(|sample| sample.is_finite()));
    std::fs::remove_dir_all(directory).unwrap();
    eprintln!("cached native PCM: two loops, finite samples, silent after 4s");
}

#[test]
fn audio_requests_reject_unrepresentable_stereo_buffers() {
    for loops in [1, 2] {
        let (pool, footage, _) = fixture(FootageKind::Audio, loops);
        for frames in [usize::MAX, usize::MAX / 2, isize::MAX as usize / 8 + 1] {
            assert!(pool.audio_samples(&footage, Tick::ZERO, frames, RATE).is_empty());
        }
    }
}
#[test]
fn invalid_and_subsample_durations_keep_native_fallback_bounded() {
    let (pool, mut footage, native) = fixture(FootageKind::Audio, 2);
    for duration in [Tick::ZERO, Tick(-TICKS_PER_SECOND), Tick(1), Tick::from_seconds_f64(f64::NAN), Tick::from_seconds_f64(f64::NEG_INFINITY)] {
        footage.duration = duration;
        let output = pool.audio_samples(&footage, Tick::ZERO, 8, RATE);
        assert_eq!(output, native[..16]);
        assert!(output.iter().all(|v| v.is_finite()));
    }
    assert!(pool.audio_samples(&footage, Tick::ZERO, 0, RATE).is_empty());
    assert_eq!(pool.audio_samples(&footage, Tick::ZERO, 8, 0), vec![0.0; 16]);
    footage.duration = Tick::from_seconds_f64(f64::INFINITY);
    assert_eq!(footage.duration, Tick::MAX);
    assert_eq!(pool.audio_samples(&footage, Tick::MAX, 8, RATE), native[..16]);
    assert!(pool.audio_samples(&footage, Tick::MIN, 8, RATE).iter().all(|v| *v == 0.0));
}
#[test]
fn short_native_loops_round_at_requested_sample_rates() {
    for rate in [8000, 11025, 22050, 44100, 48000, 96000, 12345] {
        let (pool, mut footage, _) = fixture(FootageKind::Audio, 2);
        // Four and a half requested frames round down to four complete frames.
        footage.duration = Tick(9 * TICKS_PER_SECOND / (2 * i64::from(rate)));
        let mut unlooped = footage.clone();
        unlooped.loop_count = 1;
        let native = pool.audio_samples(&unlooped, Tick::ZERO, 4, rate);
        let mut expected = native.clone();
        expected.extend_from_slice(&native);
        expected.resize(24, 0.0);
        assert_eq!(pool.audio_samples(&footage, Tick::ZERO, 12, rate), expected, "sample rate {rate}");
        // Start one frame into the second loop; flooring twice would lose a frame at 12345Hz.
        let start = Tick((5 * TICKS_PER_SECOND + i64::from(rate) - 1) / i64::from(rate));
        assert_eq!(pool.audio_samples(&footage, start, 7, rate), expected[10..24], "sample rate {rate} at a native offset");
        footage.duration = Tick((TICKS_PER_SECOND + i64::from(rate) - 1) / i64::from(rate));
        let first = pool.audio_samples(&unlooped, Tick::ZERO, 1, rate);
        let mut one_frame = first.clone();
        one_frame.extend_from_slice(&first);
        one_frame.resize(12, 0.0);
        assert_eq!(pool.audio_samples(&footage, Tick::ZERO, 6, rate), one_frame, "one-frame period at {rate}");
    }
}
