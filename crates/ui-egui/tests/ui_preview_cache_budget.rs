//! Preview cache limits keep the displayed frame and permit resolution/budget recovery.

mod cache {
    //! Small, actual public Frames cache boundaries; no large frame allocation.
    use effectcraft_engine::{Session, project::ItemId, render::RenderOpts, time::Tick};
    use effectcraft_ui_egui::frames::{FrameImage, FrameKey, Frames, RemoteDone, RemoteFrame, RemoteFrames, RemoteJob, RenderSource};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct QueueOnly {
        pending: Mutex<Vec<(RemoteJob, RemoteDone)>>,
        started: Mutex<Vec<Tick>>,
    }
    impl RemoteFrames for QueueOnly {
        fn slots(&self) -> usize {
            1
        }
        fn start(&self, job: RemoteJob, done: RemoteDone) {
            self.started.lock().unwrap().push(job.t);
            self.pending.lock().unwrap().push((job, done));
        }
    }
    impl QueueOnly {
        fn complete(&self) -> usize {
            let pending = std::mem::take(&mut *self.pending.lock().unwrap());
            let n = pending.len();
            for (job, done) in pending {
                let comp = job.project.comp(job.comp).unwrap();
                assert_eq!((comp.width, comp.height), (4, 4), "only a tiny real composition is requested");
                done(Ok(RemoteFrame { width: 4, height: 4, rgba: [255, 0, 0, 255].repeat(16), ms: 0.1 }));
            }
            n
        }
    }
    fn setup(budget: usize) -> (Session, Frames, Arc<QueueOnly>, ItemId, RenderSource, FrameKey) {
        let mut session = Session::default();
        let comp = ItemId(session.execute("comp.new", json!({"width": 4, "height": 4, "frameRate": 30, "duration": 1})).unwrap()["comp"].as_u64().unwrap());
        session.execute("layer.newSolid", json!({"width": 4, "height": 4, "color": "#ff0000"})).unwrap();
        let mut frames = Frames::default();
        let queue = Arc::new(QueueOnly::default());
        frames.set_remote(Some(queue.clone()));
        frames.set_budget(budget);
        let source = RenderSource {
            project: session.project.clone(),
            footage: session.footage.clone(),
            expr: session.expr.clone(),
            layer_cache: session.layer_cache.clone(),
            gpu: None,
            gpu_display: false,
            disk: None,
        };
        let key = FrameKey {
            revision: session.revision,
            content: frames.content_of(&session.project, session.revision, comp),
            comp: comp.0,
            frame: 0,
            scale: 1000,
            view: 0,
            opts: 0,
        };
        (session, frames, queue, comp, source, key)
    }
    #[test]
    fn cpu_frame_larger_than_ram_cache_reports_zero_preview_capacity() {
        let (_, frames, queue, comp, source, key) = setup(32);
        let capacity = frames.preview_capacity(4, 4, 1.0, false);
        let gpu_eligible_capacity = frames.preview_capacity(4, 4, 1.0, true);
        let mut completed = 0;
        for _ in 0..2 {
            frames.request_urgent(&source, key, comp, Tick::ZERO, RenderOpts::default());
            assert_eq!(frames.dispatch_remote(), 1);
            completed += queue.complete();
            assert_eq!(frames.inflight(), 0);
            assert!(!frames.is_cached(&key), "the RAM cap must remain enforced for a 64-byte frame");
        }
        eprintln!(
            "TINY_RAM capacity={capacity} gpu_eligible_capacity={gpu_eligible_capacity} cached={} completed={completed} started={:?}",
            frames.is_cached(&key),
            queue.started.lock().unwrap()
        );
        assert_eq!(frames.budget(), 32);
        assert_eq!(completed, 2, "completed frames are repeatedly discarded; this is not a worker timeout");
        assert_eq!(capacity, 0, "a cache that cannot retain one frame must not promise one preview frame");
        assert_eq!(gpu_eligible_capacity, 0, "GPU fallback cannot bypass the independent RAM cap");
    }
    #[test]
    fn exact_fit_and_larger_ram_budgets_retain_cpu_frames() {
        for budget in [64, 128] {
            let (_, frames, queue, comp, source, key) = setup(budget);
            assert!(frames.preview_capacity(4, 4, 1.0, false) >= 1);
            frames.request_urgent(&source, key, comp, Tick::ZERO, RenderOpts::default());
            assert_eq!(frames.dispatch_remote(), 1);
            assert_eq!(queue.complete(), 1);
            assert_eq!(frames.inflight(), 0);
            let Some(FrameImage::Cpu(image)) = frames.get(&key) else {
                panic!("a fitting completed frame must remain available");
            };
            assert_eq!(image.size, [4, 4]);
            assert!(image.pixels.iter().all(|p| p.to_array() == [255, 0, 0, 255]));
            assert_eq!(frames.budget(), budget);
            frames.request(&source, key, comp, Tick::ZERO, RenderOpts::default());
            assert_eq!(frames.dispatch_remote(), 0, "a cached frame does not need to render again");
        }
    }
}

mod playback {
    //! Public QueueOnly playback reproduction for a RAM budget below one full frame.
    //! No native/GPU rendering: completed4x4 frames go through the public remote callback.
    use effectcraft_engine::Session;
    use effectcraft_ui_egui::{
        EffectcraftApp,
        frames::{Frames, RemoteDone, RemoteFrame, RemoteFrames, RemoteJob},
    };
    use egui_kittest::Harness;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct QueueOnly {
        pending: Mutex<Vec<(RemoteJob, RemoteDone)>>,
        started: Mutex<Vec<i64>>,
    }
    impl RemoteFrames for QueueOnly {
        fn slots(&self) -> usize {
            1
        }
        fn start(&self, job: RemoteJob, done: RemoteDone) {
            let rate = job.project.comp(job.comp).unwrap().frame_rate;
            self.started.lock().unwrap().push(rate.frame_at(job.t));
            self.pending.lock().unwrap().push((job, done));
        }
    }
    impl QueueOnly {
        fn deliver(&self) -> usize {
            let pending = std::mem::take(&mut *self.pending.lock().unwrap());
            let completed = pending.len();
            for (job, done) in pending {
                let comp = job.project.comp(job.comp).unwrap();
                assert_eq!((comp.width, comp.height), (4, 4));
                assert_eq!(job.opts.scale, 1.0, "the preview requests full-resolution64-byte frames");
                done(Ok(RemoteFrame { width: 4, height: 4, rgba: [255, 0, 0, 255].repeat(16), ms: 0.1 }));
            }
            completed
        }
    }
    struct ResultState {
        playing: bool,
        caching: bool,
        capacity: i64,
        waiting: bool,
        audio_held: bool,
        completed: usize,
        frame_zero_requests: usize,
        cached: Vec<i64>,
    }
    fn cache_preview(budget: usize) -> ResultState {
        let mut session = Session::default();
        session.execute("comp.new", json!({"name":"Tiny Budget", "width":4, "height":4, "frameRate":24, "duration":0.25})).unwrap();
        session.execute("layer.newSolid", json!({"width":4, "height":4, "color":"#ff0000"})).unwrap();
        session
            .execute(
                "playback.settings.set",
                json!({
                    "shortcut":"spacebar", "cacheBeforePlayback":true, "range":"entireDuration",
                    "playFrom":"rangeStart", "resolution":"full", "includeAudio":false
                }),
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
        h.run_steps(3);
        let remote = Arc::new(QueueOnly::default());
        let mut frames = Frames::default();
        frames.set_remote(Some(remote.clone()));
        frames.set_budget(budget);
        // Replace only after startup applies normal preferences; tiny budgets model a large frame
        // exceeding a valid native Settings allowance without allocating that large frame.
        h.state_mut().frames = frames;
        let ctx = h.ctx.clone();
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "playback.toggle", json!({})).unwrap();
        if budget >= 64 {
            assert!(h.state().playback.playing && h.state().playback.caching, "a fitting cache-before-playback control enters caching");
        }
        let mut completed = 0;
        for _ in 0..24 {
            h.step();
            h.state().frames.dispatch_remote();
            completed += remote.deliver();
        }
        let app = h.state();
        let comp = app.session.active_comp_id().unwrap();
        let key = app.frame_key(comp, 0, 1.0);
        let started = remote.started.lock().unwrap();
        let result = ResultState {
            playing: app.playback.playing,
            caching: app.playback.caching,
            capacity: app.frames.preview_capacity(4, 4, 1.0, false),
            waiting: app.playback.waiting,
            audio_held: app.playback.audio_held,
            completed,
            frame_zero_requests: started.iter().filter(|frame| **frame == 0).count(),
            cached: app.frames.cached_frames(&key),
        };
        eprintln!(
            "budget={budget} capacity={} playing={} caching={} completed={} frame0_requests={} cached={:?}",
            result.capacity, result.playing, result.caching, result.completed, result.frame_zero_requests, result.cached
        );
        result
    }
    #[test]
    fn queue_only_cache_before_playback_finishes_when_one_frame_fits() {
        let result = cache_preview(128);
        assert!(result.completed > 0, "the remote callback actually completed a full frame");
        assert!(result.playing && !result.caching, "a fitting one-frame preview starts normally");
    }
    #[test]
    fn queue_only_cache_before_playback_does_not_wait_forever_below_one_frame() {
        let result = cache_preview(32);
        if result.caching {
            assert!(result.completed > 0, "a still-waiting preview actually received completed frames");
            assert!(result.frame_zero_requests >= 2, "the still-waiting preview repeated its first completed frame");
        }
        assert!(!result.playing && !result.waiting && !result.audio_held, "an impossible video preview leaves no playing or audio wait state");
        assert_eq!(result.capacity, 0);
        assert_eq!(result.completed, 0, "an impossible frame is rejected before render");
        assert_eq!(result.frame_zero_requests, 0, "paused urgent requests must not restart the failed preview");
        assert!(
            !result.caching,
            "the preview must end Cache Before Playback when no full frame fits: playing={}, completed={}, frame0_requests={}",
            result.playing, result.completed, result.frame_zero_requests
        );
    }
}

mod recovery {
    //! Bounded paused preview requests and actual tiny-frame recovery.
    use effectcraft_engine::{Session, render::Renderer};
    use effectcraft_ui_egui::{
        EffectcraftApp,
        frames::{Frames, RemoteDone, RemoteFrame, RemoteFrames, RemoteJob, RenderSource, to_color_image},
        state::Resolution,
    };
    use egui_kittest::Harness;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct QueueOnly {
        source: RenderSource,
        pending: Mutex<Vec<(RemoteJob, RemoteDone)>>,
        started: Mutex<Vec<(f64, Option<[f64; 4]>)>>,
        started_frames: Mutex<Vec<i64>>,
    }
    impl RemoteFrames for QueueOnly {
        fn slots(&self) -> usize {
            1
        }
        fn start(&self, job: RemoteJob, done: RemoteDone) {
            self.started.lock().unwrap().push((job.opts.scale, job.opts.roi));
            self.started_frames.lock().unwrap().push(job.project.comp(job.comp).unwrap().frame_rate.frame_at(job.t));
            self.pending.lock().unwrap().push((job, done));
        }
    }
    impl QueueOnly {
        fn deliver(&self) -> usize {
            let pending = std::mem::take(&mut *self.pending.lock().unwrap());
            let count = pending.len();
            for (job, done) in pending {
                let comp = job.project.comp(job.comp).unwrap();
                assert_eq!((comp.width, comp.height), (4, 4), "only tiny actual frames are rendered");
                let renderer = Renderer::new(&job.project, self.source.footage.as_ref(), job.opts);
                let image = renderer.comp_frame_cpu(job.comp, job.t);
                assert!(image.width <= 4 && image.height <= 4);
                let rgba = to_color_image(&image).pixels.iter().flat_map(|p| p.to_array()).collect();
                done(Ok(RemoteFrame { width: image.width, height: image.height, rgba, ms: 0.1 }));
            }
            count
        }
    }
    fn setup() -> (Harness<'static, EffectcraftApp>, Arc<QueueOnly>) {
        let mut session = Session::default();
        session.execute("comp.new", json!({"name":"Tiny paused preview", "width":4, "height":4, "frameRate":24, "duration":0.25})).unwrap();
        session.execute("layer.newSolid", json!({"width":4, "height":4, "color":"#ff0000"})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
        h.run_steps(3);
        h.state_mut().ui.viewer.res = Resolution::Full;
        let remote =
            Arc::new(QueueOnly { source: h.state().render_source(), pending: Mutex::default(), started: Mutex::default(), started_frames: Mutex::default() });
        let mut frames = Frames::default();
        frames.set_remote(Some(remote.clone()));
        frames.set_budget(32);
        h.state_mut().frames = frames;
        // A fresh green project key distinguishes recovery pixels from the old startup texture.
        h.state_mut().session.execute("layer.newSolid", json!({"width":4, "height":4, "color":"#00ff00"})).unwrap();
        pump(&mut h, &remote, 12);
        assert!(remote.started.lock().unwrap().is_empty(), "a paused viewer must not repeatedly request a frame the RAM cache will discard");
        assert_eq!(h.state().frames.inflight(), 0);
        assert_eq!(h.state().frames.budget(), 32);
        (h, remote)
    }
    fn pump(h: &mut Harness<'_, EffectcraftApp>, remote: &QueueOnly, steps: usize) -> usize {
        let mut completed = 0;
        for _ in 0..steps {
            let next = h.ctx.input(|i| i.time) + 1.0 / 24.0;
            let pending = h.input_mut().time.unwrap_or(next);
            h.input_mut().time = Some(next.max(pending));
            h.ctx.request_repaint();
            h.step();
            h.state().frames.dispatch_remote();
            completed += remote.deliver();
        }
        completed
    }
    fn visible_cached(h: &mut Harness<'_, EffectcraftApp>, remote: &QueueOnly, scale: f64, size: [usize; 2], budget: usize) {
        assert!(pump(h, remote, 8) > 0, "a fitting frame must actually render and complete");
        let app = h.state_mut();
        let comp = app.session.active_comp_id().unwrap();
        let key = app.frame_key(comp, 0, scale);
        assert!(app.frames.is_cached(&key), "the completed frame is retained at the requested view/resolution");
        assert_eq!(app.frames.budget(), budget, "recovery does not raise the configured RAM cap");
        assert_eq!(app.viewer_texture_size(), Some(size), "the completed recovery frame is displayed");
        let pixels = app.viewer_pixels().expect("a completed frame has visible pixels");
        assert_eq!(pixels.size, size);
        assert!(pixels.pixels.iter().all(|p| p.to_array() == [0, 255, 0, 255]), "actual green frame pixels replace the startup image");
        let count = remote.started.lock().unwrap().len();
        pump(h, remote, 4);
        assert_eq!(remote.started.lock().unwrap().len(), count, "a retained paused viewer frame is not requeued");
    }
    #[test]
    fn paused_preview_resumes_with_a_visible_half_resolution_frame() {
        let (mut h, remote) = setup();
        h.state_mut().ui.viewer.res = Resolution::Half;
        visible_cached(&mut h, &remote, 0.5, [2, 2], 32);
        assert!(remote.started.lock().unwrap().iter().all(|(scale, _)| *scale == 0.5));
    }
    #[test]
    fn paused_preview_resumes_when_the_ram_budget_fits_the_frame() {
        let (mut h, remote) = setup();
        h.state().frames.set_budget(64);
        visible_cached(&mut h, &remote, 1.0, [4, 4], 64);
    }
    #[test]
    fn paused_preview_resumes_with_a_visible_region_of_interest_frame() {
        let (mut h, remote) = setup();
        h.state_mut().session.state.region_of_interest = Some([0.0, 0.0, 2.0, 2.0]);
        visible_cached(&mut h, &remote, 1.0, [2, 2], 32);
        assert!(remote.started.lock().unwrap().iter().all(|(_, roi)| *roi == Some([0.0, 0.0, 2.0, 2.0])));
    }

    fn fitting_setup(budget: usize, idle: bool, outside_work_area: bool) -> (Harness<'static, EffectcraftApp>, Arc<QueueOnly>) {
        let mut session = Session::default();
        session.execute("comp.new", json!({"name":"Fitting paused cache", "width":4, "height":4, "frameRate":24, "duration":0.25})).unwrap();
        session.execute("layer.newSolid", json!({"width":4, "height":4, "color":"#ff0000"})).unwrap();
        if outside_work_area {
            session.execute("comp.workArea", json!({"start":1.0/24.0,"end":3.0/24.0})).unwrap();
            let rate = session.active_comp().unwrap().frame_rate;
            session.set_time(rate.tick_of(5));
        }
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
        h.run_steps(3);
        h.state_mut().ui.viewer.res = Resolution::Full;
        h.state_mut().ui.cache_when_idle = idle;
        let remote =
            Arc::new(QueueOnly { source: h.state().render_source(), pending: Mutex::default(), started: Mutex::default(), started_frames: Mutex::default() });
        let mut frames = Frames::default();
        frames.set_remote(Some(remote.clone()));
        frames.set_budget(budget);
        h.state_mut().frames = frames;
        h.state_mut().session.execute("layer.newSolid", json!({"width":4,"height":4,"color":"#00ff00"})).unwrap();
        (h, remote)
    }
    fn assert_paused_current(h: &mut Harness<'_, EffectcraftApp>, remote: &QueueOnly, frame: i64, marker: &str) {
        let comp = h.state().session.active_comp_id().unwrap();
        let key = h.state().frame_key(comp, frame, 1.0);
        eprintln!(
            "{marker}: requests={:?} cached={:?} time={}",
            remote.started_frames.lock().unwrap(),
            h.state().frames.cached_frames(&key),
            h.ctx.input(|i| i.time)
        );
        assert!(h.state().frames.is_cached(&key), "{marker}: the current frame must stay cached");
        let pixels = h.state_mut().viewer_pixels().unwrap();
        assert_eq!(pixels.size, [4, 4]);
        assert!(pixels.pixels.iter().all(|p| p.to_array() == [0, 255, 0, 255]), "{marker}: actual green pixels stay visible");
        let count = remote.started_frames.lock().unwrap().len();
        pump(h, remote, 4);
        assert_eq!(remote.started_frames.lock().unwrap().len(), count, "{marker}: no paused frame requeue");
    }
    #[test]
    fn fitting_paused_one_frame_cache_keeps_the_current_frame() {
        let (mut h, remote) = fitting_setup(64, false, false);
        assert!(pump(&mut h, &remote, 16) > 0);
        assert_paused_current(&mut h, &remote, 0, "FITTING_PAUSED");
    }
    #[test]
    fn fitting_paused_larger_cache_retains_the_current_frame() {
        let (mut h, remote) = fitting_setup(512, false, false);
        assert!(pump(&mut h, &remote, 16) > 0);
        assert_paused_current(&mut h, &remote, 0, "FITTING_LARGER");
    }
    #[test]
    fn two_frame_paused_cache_does_not_render_past_the_cached_neighbor() {
        let (mut h, remote) = fitting_setup(160, false, false);
        let comp = h.state().session.active_comp_id().unwrap();
        assert_eq!(h.state().frames.preview_capacity(4, 4, 1.0, false), 2);
        for frame in [0, 1] {
            h.state().request_frame(comp, frame, 1.0);
            assert_eq!(h.state().frames.dispatch_remote(), 1);
            assert_eq!(remote.deliver(), 1);
            assert!(h.state().frames.is_cached(&h.state().frame_key(comp, frame, 1.0)));
        }
        let before = remote.started_frames.lock().unwrap().len();
        pump(&mut h, &remote, 12);
        eprintln!("PAUSED_WINDOW requests={:?}", remote.started_frames.lock().unwrap());
        assert_eq!(remote.started_frames.lock().unwrap().len(), before, "PAUSED_WINDOW: cached current plus neighbor leaves no farther paused render slot");
        assert_paused_current(&mut h, &remote, 0, "PAUSED_WINDOW");
    }
    #[test]
    fn paused_two_frame_cache_preserves_the_cursor_after_work_area() {
        let (mut h, remote) = fitting_setup(160, false, true);
        pump(&mut h, &remote, 16);
        assert_paused_current(&mut h, &remote, 5, "PAUSED_OUTSIDE");
    }
    fn idle_outside(budget: usize) {
        let (mut h, remote) = fitting_setup(budget, true, true);
        pump(&mut h, &remote, 8);
        let prior = h.ctx.input(|i| i.time);
        h.input_mut().time = Some(prior + 2.0);
        pump(&mut h, &remote, 12);
        assert!(h.ctx.input(|i| i.time) >= prior + 2.0, "fixture must actually cross idle quiet threshold");
        assert_paused_current(&mut h, &remote, 5, "IDLE_OUTSIDE");
    }
    #[test]
    fn idle_one_frame_cache_preserves_the_cursor_outside_work_area() {
        idle_outside(64);
    }
    #[test]
    fn idle_larger_cache_preserves_the_cursor_outside_work_area() {
        idle_outside(512);
    }
    fn streaming(budget: usize) {
        let (mut h, remote) = fitting_setup(budget, false, false);
        h.state_mut()
            .session
            .execute(
                "playback.settings.set",
                json!({"shortcut":"spacebar","cacheBeforePlayback":false,"resolution":"full","range":"entireDuration","includeAudio":false}),
            )
            .unwrap();
        h.step();
        // Preference application precedes the injected tiny budget.
        let mut frames = Frames::default();
        frames.set_remote(Some(remote.clone()));
        frames.set_budget(budget);
        h.state_mut().frames = frames;
        let ctx = h.ctx.clone();
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "playback.toggle", json!({})).unwrap();
        let mut progress = false;
        let mut completed = 0;
        for _ in 0..24 {
            completed += pump(&mut h, &remote, 1);
            progress |= h.state().session.time() > effectcraft_engine::time::Tick::ZERO;
        }
        eprintln!("STREAMING budget={budget} completed={completed} progress={progress} requests={:?}", remote.started_frames.lock().unwrap());
        assert!(completed > 0);
        assert!(h.state().playback.playing && !h.state().playback.caching);
        assert!(progress, "STREAMING_FIT: non-cache-before playback must advance when a complete frame fits");
    }
    #[test]
    fn streaming_preview_advances_when_only_one_frame_fits() {
        streaming(64);
    }
    #[test]
    fn streaming_preview_advances_with_a_larger_cache() {
        streaming(512);
    }

    fn idle_before(budget: usize) {
        let (mut h, remote) = fitting_setup(budget, true, false);
        h.state_mut().session.execute("comp.workArea", json!({"start":3.0/24.0,"end":6.0/24.0})).unwrap();
        pump(&mut h, &remote, 8);
        let prior = h.ctx.input(|i| i.time);
        h.input_mut().time = Some(prior + 2.0);
        pump(&mut h, &remote, 12);
        assert!(h.ctx.input(|i| i.time) >= prior + 2.0, "fixture crosses idle quiet threshold");
        assert_paused_current(&mut h, &remote, 0, "IDLE_BEFORE");
    }
    #[test]
    fn idle_two_frame_cache_preserves_the_cursor_before_work_area() {
        idle_before(160);
    }
    #[test]
    fn idle_larger_cache_preserves_the_cursor_before_work_area() {
        idle_before(512);
    }
}
