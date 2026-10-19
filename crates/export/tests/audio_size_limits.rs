//! Size rejection through the production export API; no large PCM buffers are allocated.
use effectcraft_export::render_queue::{AudioFormat, OutputFormat, OutputModule, RenderSettings};
use effectcraft_export::{ExportError, Job, JobOptions, Sink, export};
use effectcraft_project::{Comp, Footage, FootageKind, ItemId, ItemKind, LayerSource, Project, build};
use effectcraft_raster::Image;
use effectcraft_render::FootageSource;
use effectcraft_time::{FrameRate, Tick};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct CountAudio(AtomicUsize);
impl FootageSource for CountAudio {
    fn frame(&self, _: ItemId, _: &Footage, _: Tick) -> Option<Arc<Image>> {
        None
    }
    fn audio(&self, _: ItemId, _: &Footage, _: Tick, _: usize, _: u32) -> Option<Vec<f32>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        None
    }
}
#[test]
fn oversized_wav_and_aiff_are_rejected_before_mixing() {
    for (format, precision, seconds, path) in
        [(OutputFormat::Wav, AudioFormat::F32, 3600.0, "large.wav"), (OutputFormat::Aiff, AudioFormat::S24, 7200.0, "large.aif")]
    {
        let mut project = Project::default();
        let duration = Tick::from_seconds_f64(seconds);
        let footage = Footage { path: "synthetic-audio.wav".into(), kind: FootageKind::Audio, duration, has_audio: true, ..Default::default() };
        let item = project.add_item("Audio", Default::default(), None, ItemKind::Footage(footage));
        let mut comp = Comp::new(16, 16, FrameRate::FPS_30, duration);
        comp.layers.push(build::layer(&mut project, &comp, "Audio", LayerSource::Footage { item }, (16, 16), None));
        let comp = project.add_item("Comp", Default::default(), None, ItemKind::Comp(comp.into()));
        let settings = RenderSettings::default();
        let mut output = OutputModule::for_format(format);
        output.audio_sample_rate = 192000;
        output.audio_channels = 2;
        output.audio_format = precision;
        let provider = CountAudio::default();
        let files: Arc<Mutex<Vec<String>>> = Default::default();
        let received = files.clone();
        let sink: Box<Sink> = Box::new(move |path: &str, _: Vec<u8>| received.lock().unwrap().push(path.into()));
        let job = Job {
            project: &project,
            footage: &provider,
            expr: None,
            accel: None,
            comp,
            settings: &settings,
            output: &output,
            path,
            sink: Some(&*sink),
            nested_switches: true,
            options: JobOptions::default(),
        };
        let error = export(&job, &mut |_| true).unwrap_err();
        assert!(matches!(error, ExportError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("size or memory limits"), "{error}");
        assert_eq!(provider.0.load(Ordering::SeqCst), 0, "oversized {format:?} must fail before requesting any audio");
        assert!(!files.lock().unwrap().iter().any(|file| file.ends_with(".wav") || file.ends_with(".aif")), "oversized output must not publish an audio file");
    }
}
