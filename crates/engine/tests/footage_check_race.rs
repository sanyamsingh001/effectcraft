//! Background metadata checking respects edits completed while its probe was running.
use effectcraft_engine::{
    Importer, Services, Session,
    project::{Footage, FootageKind, ItemId, ItemKind},
    time::{FrameRate, Tick},
};
use serde_json::json;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::Duration,
};
struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn release(&self) {
        let (lock, cv) = &*self.0;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
fn wait(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, cv) = &**gate;
    let mut open = lock.lock().unwrap();
    while !*open {
        open = cv.wait(open).unwrap();
    }
}
fn metadata(path: &str, kind: FootageKind, changed: bool) -> Footage {
    let rate = if changed { FrameRate::FPS_24 } else { FrameRate::FPS_30 };
    Footage {
        path: path.into(),
        kind,
        width: if changed { 8 } else { 4 },
        height: if changed { 8 } else { 4 },
        frame_rate: rate,
        duration: rate.tick_of(if changed { 48 } else { 60 }),
        has_video: true,
        ..Default::default()
    }
}
struct BlockProbe {
    entered: SyncSender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    calls: AtomicUsize,
    next_kind: FootageKind,
}
impl Importer for BlockProbe {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            self.entered.send(()).unwrap();
            wait(&self.gate);
        }
        Ok(metadata(path, if first { FootageKind::Video } else { self.next_kind }, !first))
    }
}
fn marker(name: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("effectcraft-check-race-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("{}-{name}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, b"public Importer supplies metadata").unwrap();
    path.to_string_lossy().into()
}
fn get(s: &Session, item: ItemId) -> &Footage {
    let ItemKind::Footage(f) = &s.project.item(item).unwrap().kind else { panic!("footage fixture") };
    f
}
fn setup(kind: FootageKind) -> (Session, ItemId, Gate, std::sync::mpsc::Receiver<()>) {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let importer = Arc::new(BlockProbe { entered, gate: gate.0.clone(), calls: AtomicUsize::new(0), next_kind: kind });
    let mut s = Session { importer: Some(importer), ..Default::default() };
    let mut f = metadata(&marker("source.mp4"), FootageKind::Video, false);
    f.width = 0;
    f.height = 0;
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(f));
    s.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":15})).unwrap();
    s.execute("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    rx.recv_timeout(Duration::from_secs(10)).expect("probe entered");
    (s, item, gate, rx)
}
#[test]
fn background_probe_uses_the_current_interpretation() {
    let (mut s, item, gate, _) = setup(FootageKind::Video);
    s.execute("file.interpretFootage",json!({"items":[item.0],"frameRate":12,"alpha":"premultiplied","matteColor":[0.2,0.3,0.4],"invertAlpha":true,"fields":"upper","colorProfile":"rec2020","linearLight":true,"pixelAspect":1.3,"loop":2})).unwrap();
    let current = get(&s, item).clone();
    let undo = s.history.undo.len();
    gate.release();
    s.wait_jobs();
    let f = get(&s, item);
    assert_eq!(f.frame_rate, current.frame_rate, "background metadata must retain the newer12fps interpretation");
    assert_eq!(f.native_rate, Some(FrameRate::FPS_30));
    assert_eq!(f.duration, FrameRate::new(12, 1).tick_of(60));
    assert_eq!((f.width, f.height), (4, 4));
    assert_eq!(
        (f.alpha, f.premul_color, f.invert_alpha, f.fields, f.color_profile, f.linear_light, f.pixel_aspect, f.loop_count),
        (
            current.alpha,
            current.premul_color,
            current.invert_alpha,
            current.fields,
            current.color_profile,
            current.linear_light,
            current.pixel_aspect,
            current.loop_count
        )
    );
    assert_eq!(s.history.undo.len(), undo);
    assert!(s.is_dirty());
}
#[test]
fn background_probe_respects_a_newer_use_file_rate() {
    let (mut s, item, gate, _) = setup(FootageKind::Video);
    s.execute("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
    gate.release();
    s.wait_jobs();
    let f = get(&s, item);
    assert_eq!(f.frame_rate, FrameRate::FPS_30);
    assert_eq!(f.native_rate, None);
    assert_eq!(f.duration, Tick::from_seconds_f64(2.0));
}
#[test]
fn background_probe_cannot_restore_a_replaced_source_path() {
    let (mut s, item, gate, _) = setup(FootageKind::Video);
    let path = marker("replacement.mp4");
    s.execute("file.replaceFootage", json!({"items":[item.0],"path":path})).unwrap();
    let current = get(&s, item).clone();
    let undo = s.history.undo.len();
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &current, "old probe cannot overwrite a relink");
    assert_eq!(s.history.undo.len(), undo);
}
#[test]
fn background_probe_cannot_restore_a_replaced_source_kind() {
    let (mut s, item, gate, _) = setup(FootageKind::Still);
    let path = get(&s, item).path.clone();
    s.execute("file.replaceFootage", json!({"items":[item.0],"path":path})).unwrap();
    let current = get(&s, item).clone();
    assert_eq!(current.kind, FootageKind::Still);
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &current, "old Video probe cannot overwrite a new Still at the same path");
}
struct BlockExists {
    entered: SyncSender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}
impl Services for BlockExists {
    fn read_file(&self, p: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(p)
    }
    fn write_file(&self, p: &str, data: &[u8]) -> std::io::Result<()> {
        std::fs::write(p, data)
    }
    fn exists(&self, _: &str) -> bool {
        self.entered.send(()).unwrap();
        wait(&self.gate);
        false
    }
}
struct DirectProbe;
impl Importer for DirectProbe {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        Ok(metadata(path, FootageKind::Video, true))
    }
}
#[test]
fn a_stale_missing_result_cannot_mark_a_relinked_file_missing() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let mut s = Session { importer: Some(Arc::new(DirectProbe)), services: Arc::new(BlockExists { entered, gate: gate.0.clone() }), ..Default::default() };
    let item =
        Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata("missing-old.mp4", FootageKind::Video, false)));
    s.execute("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    rx.recv_timeout(Duration::from_secs(10)).expect("exists entered");
    s.execute("file.replaceFootage", json!({"items":[item.0],"path":marker("present-new.mp4")})).unwrap();
    let current = get(&s, item).clone();
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &current, "missing status belongs to the captured source path");
}

#[test]
fn background_probe_cannot_overwrite_new_metadata_at_the_same_path() {
    let (mut s, item, gate, _) = setup(FootageKind::Video);
    let path = get(&s, item).path.clone();
    s.execute("file.replaceFootage", json!({"items":[item.0],"path":path})).unwrap();
    let current = get(&s, item).clone();
    assert_eq!(current.kind, FootageKind::Video);
    assert_eq!((current.width, current.height), (8, 8));
    assert_eq!(current.frame_rate, FrameRate::FPS_24);
    let undo = s.history.undo.len();
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &current, "a completed replacement supersedes the pending metadata probe");
    assert_eq!(s.history.undo.len(), undo);
}

#[test]
fn a_stale_missing_result_cannot_mark_same_path_new_metadata_missing() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let mut s = Session { importer: Some(Arc::new(DirectProbe)), services: Arc::new(BlockExists { entered, gate: gate.0.clone() }), ..Default::default() };
    let path = marker("same-path.mp4");
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
    s.execute("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    rx.recv_timeout(Duration::from_secs(10)).expect("exists entered");
    s.execute("file.replaceFootage", json!({"items":[item.0],"path":path})).unwrap();
    let current = get(&s, item).clone();
    assert_eq!((current.width, current.height), (8, 8));
    assert!(!current.missing);
    let undo = s.history.undo.len();
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &current, "replacement metadata also supersedes an older same-path missing result");
    assert_eq!(s.history.undo.len(), undo);
}

struct SameDimensionsProbe {
    rate: FrameRate,
    frames: i64,
}
impl Importer for SameDimensionsProbe {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        Ok(Footage {
            path: path.into(),
            kind: FootageKind::Video,
            width: 4,
            height: 4,
            frame_rate: self.rate,
            duration: self.rate.tick_of(self.frames),
            has_video: true,
            ..Default::default()
        })
    }
}
#[test]
fn a_stale_missing_result_cannot_overwrite_same_dimensions_native_timing() {
    let mut overwritten = Vec::new();
    for (name, rate, frames) in [("rate-and-count", FrameRate::FPS_24, 48), ("count-only", FrameRate::FPS_30, 90), ("rate-only", FrameRate::FPS_24, 60)] {
        let (entered, rx) = sync_channel(1);
        let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
        let mut s = Session {
            importer: Some(Arc::new(SameDimensionsProbe { rate, frames })),
            services: Arc::new(BlockExists { entered, gate: gate.0.clone() }),
            ..Default::default()
        };
        let path = marker(name);
        let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
        s.execute("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
        rx.recv_timeout(Duration::from_secs(10)).expect("exists entered");
        s.execute("file.replaceFootage", json!({"items":[item.0],"path":path})).unwrap();
        let current = get(&s, item).clone();
        assert_eq!((current.width, current.height), (4, 4));
        assert_eq!(current.frame_rate, rate);
        assert_eq!(current.duration, rate.tick_of(frames));
        assert!(!current.missing);
        let undo = s.history.undo.len();
        gate.release();
        s.wait_jobs();
        if get(&s, item) != &current {
            overwritten.push((name, get(&s, item).missing));
        }
        assert_eq!(s.history.undo.len(), undo);
    }
    assert!(overwritten.is_empty(), "replacement native timing supersedes older missing status: {overwritten:?}");
}

struct IdenticalProbe;
impl Importer for IdenticalProbe {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        Ok(metadata(path, FootageKind::Video, false))
    }
}
struct CaptureExists {
    entered: SyncSender<bool>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}
impl Services for CaptureExists {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, data)
    }
    fn exists(&self, path: &str) -> bool {
        let observed = std::path::Path::new(path).is_file();
        self.entered.send(observed).unwrap();
        wait(&self.gate);
        observed
    }
}
#[test]
fn completed_metadata_identical_reload_supersedes_old_missing_observation() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let path = marker("metadata-identical.mp4");
    std::fs::remove_file(&path).unwrap();
    let mut s = Session { importer: Some(Arc::new(IdenticalProbe)), services: Arc::new(CaptureExists { entered, gate: gate.0.clone() }), ..Default::default() };
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
    let initial = get(&s, item).clone();
    s.execute_checked("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    assert!(!rx.recv_timeout(Duration::from_secs(10)).expect("captured old absence"));
    std::fs::write(&path, b"restored source; public Importer supplies identical metadata").unwrap();
    assert!(std::path::Path::new(&path).is_file());
    let undo = s.history.undo.len();
    s.execute_checked("file.reloadFootage", json!({"items":[item.0]})).unwrap();
    assert_eq!(get(&s, item), &initial, "metadata is deliberately identical including path/kind/dimensions/rate/count");
    assert_eq!(s.history.undo.len(), undo, "metadata-identical reload keeps its no-op document edit behavior");
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &initial, "completed reload must supersede the older captured missing status");
    assert_eq!(s.history.undo.len(), undo);
}

#[test]
fn completed_metadata_identical_replacement_supersedes_old_missing_observation() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let path = marker("metadata-identical-replace.mp4");
    std::fs::remove_file(&path).unwrap();
    let mut s = Session { importer: Some(Arc::new(IdenticalProbe)), services: Arc::new(CaptureExists { entered, gate: gate.0.clone() }), ..Default::default() };
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
    let initial = get(&s, item).clone();
    s.execute_checked("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    assert!(!rx.recv_timeout(Duration::from_secs(10)).expect("captured old absence"));
    std::fs::write(&path, b"restored source; identical metadata").unwrap();
    assert!(std::path::Path::new(&path).is_file());
    s.execute_checked("file.replaceFootage", json!({"item":item.0,"path":path})).unwrap();
    assert_eq!(get(&s, item), &initial, "only the item name may change; source metadata is identical");
    let undo = s.history.undo.len();
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &initial, "completed replacement supersedes the older captured missing status");
    assert_eq!(s.history.undo.len(), undo);
}
struct CaptureFirstExists {
    entered: SyncSender<bool>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    calls: AtomicUsize,
}
impl Services for CaptureFirstExists {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, data)
    }
    fn exists(&self, path: &str) -> bool {
        let observed = std::path::Path::new(path).is_file();
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.send(observed).unwrap();
            wait(&self.gate);
        }
        observed
    }
}
#[test]
fn a_newer_present_check_supersedes_an_older_missing_observation() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let path = marker("metadata-identical-check.mp4");
    std::fs::remove_file(&path).unwrap();
    let mut s = Session { services: Arc::new(CaptureFirstExists { entered, gate: gate.0.clone(), calls: AtomicUsize::new(0) }), ..Default::default() };
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
    let initial = get(&s, item).clone();
    s.execute_checked("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
    assert!(!rx.recv_timeout(Duration::from_secs(10)).expect("captured old absence"));
    std::fs::write(&path, b"restored source").unwrap();
    assert!(std::path::Path::new(&path).is_file());
    let newer = s.execute_checked("footage.check", json!({"items":[item.0],"wait":true})).unwrap();
    assert_eq!(newer, json!({"checked":1,"missing":0,"probed":0}));
    assert_eq!(get(&s, item), &initial);
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &initial, "an older check cannot undo the newer present observation");
    assert!(s.history.undo.is_empty());
}

#[test]
fn unknown_native_frame_count_probe_keeps_the_current_conform_and_adopts_file_metadata() {
    let mut failures = Vec::new();
    for fps in [12, 15] {
        let (entered, rx) = sync_channel(1);
        let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
        let importer = Arc::new(BlockProbe { entered, gate: gate.0.clone(), calls: AtomicUsize::new(0), next_kind: FootageKind::Video });
        let mut s = Session { importer: Some(importer), ..Default::default() };
        let initial = Footage {
            path: marker(&format!("unknown-native-{fps}.mp4")),
            kind: FootageKind::Video,
            has_video: true,
            frame_rate: FrameRate::FPS_29_97,
            width: 0,
            height: 0,
            duration: Tick::ZERO,
            ..Default::default()
        };
        assert_eq!(initial.source_frame_count(), None);
        let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(initial));
        s.execute_checked("footage.check", json!({"items":[item.0],"wait":false})).unwrap();
        rx.recv_timeout(Duration::from_secs(10)).expect("metadata probe entered");
        let rate = FrameRate::new(fps, 1);
        s.execute_checked("file.interpretFootage", json!({"items":[item.0],"frameRate":fps})).unwrap();
        assert_eq!(get(&s, item).frame_rate, rate);
        assert_eq!(get(&s, item).duration, Tick::ZERO, "unknown native count stays unknown until the metadata probe finishes");
        let undo = s.history.undo.len();
        gate.release();
        s.wait_jobs();
        let current = get(&s, item);
        if current.width != 4
            || current.height != 4
            || current.frame_rate != rate
            || current.native_rate != Some(FrameRate::FPS_30)
            || current.duration != rate.tick_of(60)
        {
            failures.push(format!("conform {fps}: {current:?}"));
        }
        assert_eq!(s.history.undo.len(), undo);
        s.execute_checked("file.interpretFootage", json!({"items":[item.0],"frameRate":"file"})).unwrap();
        let restored = get(&s, item);
        if restored.frame_rate != FrameRate::FPS_30 || restored.native_rate.is_some() || restored.duration != FrameRate::FPS_30.tick_of(60) {
            failures.push(format!("file rate {fps}: {restored:?}"));
        }
    }
    assert!(failures.is_empty(), "fresh native60-frame metadata must replace the unknown duration without losing its effective rate: {failures:?}");
}
#[test]
fn failed_atomic_batch_check_cannot_reuse_a_newer_checks_refresh_identity() {
    let (entered, rx) = sync_channel(1);
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let path = marker("failed-batch-check.mp4");
    std::fs::remove_file(&path).unwrap();
    let mut s = Session { services: Arc::new(CaptureFirstExists { entered, gate: gate.0.clone(), calls: AtomicUsize::new(0) }), ..Default::default() };
    let item = Arc::make_mut(&mut s.project).add_item("Video", Default::default(), None, ItemKind::Footage(metadata(&path, FootageKind::Video, false)));
    let initial = get(&s, item).clone();
    let result = s.execute_checked(
        "engine.batch",
        json!({"steps":[
            {"command":"footage.check","params":{"items":[item.0],"wait":false}},
            {"command":"prop.set","params":{"layer":"missing","path":"transform/opacity","value":0}}
        ]}),
    );
    assert!(result.is_err());
    assert!(!rx.recv_timeout(Duration::from_secs(10)).expect("batch worker captured absence"));
    assert_eq!(get(&s, item), &initial);
    std::fs::write(&path, b"restored after failed batch").unwrap();
    let latest = s.execute_checked("footage.check", json!({"items":[item.0],"wait":true})).unwrap();
    assert_eq!(latest, json!({"checked":1,"missing":0,"probed":0}));
    gate.release();
    s.wait_jobs();
    assert_eq!(get(&s, item), &initial, "a failed batch cannot rewind source-refresh identity and make its pending worker current again");
    assert!(s.history.undo.is_empty());
}
