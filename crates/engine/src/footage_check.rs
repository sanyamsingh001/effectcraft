//! Lazy project open: footage is checked in the background after File ▸ Open instead of while
//! the project loads (M13.14).
//!
//! A project file already carries every footage item's metadata, so opening it never touches
//! the media: decoding starts when a frame is first needed, thumbnails when the Project panel
//! shows one, expressions compile on first evaluation and system fonts are scanned only when a
//! text layer asks for a family that isn't bundled. What open used to skip entirely is noticing
//! that files went missing (or that an item was saved without metadata). `footage.check` does
//! that as a background task listed in the Progress panel ("Checking footage"): it looks for
//! every file (each frame of an image sequence), probes items that have no metadata, and then
//! flags missing items (`missing`, File ▸ Dependencies ▸ Find Missing Footage). The desktop app
//! starts it after every open ([`Session::check_footage_on_open`]); agents and the CLI run it
//! with `wait: true`.

use effectcraft_project::{Footage, FootageKind, ItemId, ItemKind};
use serde_json::{Value, json};

use crate::Session;

/// One item to check.
struct Job {
    id: ItemId,
    footage: Footage,
    refresh: u64,
}

/// What the check found for one item.
enum Found {
    Missing(bool),
    Probed(Box<Footage>),
}

/// The source a background result checked. Edits may replace it before completion.
struct Checked {
    id: ItemId,
    refresh: u64,
    path: String,
    kind: FootageKind,
    width: u32,
    height: u32,
    timing: (effectcraft_time::FrameRate, Option<i64>),
    found: Found,
}

/// Metadata refresh keeps the user's interpretation. A conformed video's native timing
/// comes from the fresh probe so restoring the file rate follows any changes on disk.
pub(crate) fn preserve_interpretation(fresh: &mut Footage, previous: &Footage) {
    fresh.alpha = previous.alpha;
    fresh.premul_color = previous.premul_color;
    fresh.invert_alpha = previous.invert_alpha;
    fresh.fields = previous.fields;
    fresh.linear_light = previous.linear_light;
    fresh.color_profile = previous.color_profile;
    fresh.loop_count = previous.loop_count;
    fresh.pixel_aspect = previous.pixel_aspect;
    if !fresh.missing
        && fresh.kind == FootageKind::Video
        && previous.native_rate.is_some()
        && let Some(frames) = fresh.source_frame_count()
    {
        fresh.native_rate = Some(fresh.frame_rate);
        fresh.frame_rate = previous.frame_rate;
        fresh.duration = fresh.frame_rate.tick_of(frames);
    }
}

/// File timing stays stable while conforming or looping. Sequence file count is
/// independent of the user's missing-frame policy and source-frame labels.
fn source_timing(f: &Footage) -> (effectcraft_time::FrameRate, Option<i64>) {
    let frames = match f.kind {
        FootageKind::Video => f.source_frame_count(),
        FootageKind::Sequence => Some(i64::try_from(f.sequence.len()).unwrap_or(i64::MAX)),
        _ => None,
    };
    (f.native_rate.unwrap_or(f.frame_rate), frames)
}

/// Advance only a source refresh, never an interpretation edit. This runtime identity
/// also covers no-op reloads, whose serialized metadata cannot distinguish the newer source.
pub(crate) fn refresh(s: &mut Session, item: ItemId) -> u64 {
    let generation = s.footage_refresh.entry(item).or_default();
    *generation = generation.wrapping_add(1);
    *generation
}

fn exists(s: &dyn crate::Services, path: &str) -> bool {
    !path.is_empty() && s.exists(path)
}

/// Start the check (all footage, or `items`). `wait` runs it to completion first (always on
/// wasm32); the result is then `{checked, missing, probed}`.
pub fn start(s: &mut Session, items: Option<Vec<ItemId>>, wait: bool) -> crate::Result<Value> {
    let mut jobs: Vec<Job> = s
        .project
        .items
        .values()
        .filter(|i| items.as_ref().is_none_or(|v| v.contains(&i.id)))
        .filter_map(|i| match &i.kind {
            ItemKind::Footage(f) if f.kind != FootageKind::Data => Some(Job { id: i.id, footage: f.clone(), refresh: 0 }),
            _ => None,
        })
        .collect();
    if jobs.is_empty() {
        return Ok(json!({"checked": 0, "missing": 0, "probed": 0}));
    }
    for job in &mut jobs {
        job.refresh = refresh(s, job.id);
    }
    let services = s.services.clone();
    let importer = s.importer.clone();
    let n = jobs.len();
    s.spawn_task("footageCheck", format!("Checking footage ({n} items)"), wait, move |ctl| {
        let mut found: Vec<Checked> = Vec::with_capacity(jobs.len());
        let total = jobs.len() as u64;
        for (k, j) in jobs.into_iter().enumerate() {
            if !ctl.progress(k as u64, total) {
                return Err(crate::render_queue::CANCELLED.into());
            }
            let f = &j.footage;
            let present = if f.sequence.is_empty() {
                exists(services.as_ref(), &f.path)
            } else {
                // A sequence is there while any of its frames is (missing frames render as
                // placeholders, as in After Effects).
                exists(services.as_ref(), &f.path) || f.sequence.iter().any(|p| exists(services.as_ref(), p))
            };
            // Saved without metadata (a hand-written or converted project): probe it now.
            let needs_probe = present && f.width == 0 && f.height == 0 && f.kind != FootageKind::Audio;
            let result = match importer.as_ref().filter(|_| needs_probe).map(|imp| imp.probe(&f.path)) {
                Some(Ok(nf)) => Found::Probed(Box::new(nf)),
                _ => Found::Missing(!present),
            };
            found.push(Checked {
                id: j.id,
                refresh: j.refresh,
                path: f.path.clone(),
                kind: f.kind,
                width: f.width,
                height: f.height,
                timing: source_timing(f),
                found: result,
            });
        }
        ctl.progress(total, total);
        let apply: crate::jobs::Apply = Box::new(move |s: &mut Session| Ok(apply(s, found)));
        Ok(apply)
    })
}

/// Write the findings into the project. Not an undo step: it records the state of the disk,
/// not an edit (the same as After Effects marking footage missing at open).
fn apply(s: &mut Session, found: Vec<Checked>) -> Value {
    let checked = found.len();
    let (mut missing, mut probed, mut changed) = (0, 0, false);
    let p = std::sync::Arc::make_mut(&mut s.project);
    for checked in found {
        if s.footage_refresh.get(&checked.id).copied().unwrap_or(0) != checked.refresh {
            continue;
        }
        let Some(ItemKind::Footage(cur)) = p.item_mut(checked.id).map(|i| &mut i.kind) else { continue };
        // A replacement or reload can supply newer metadata while this check runs.
        if cur.path != checked.path
            || cur.kind != checked.kind
            || cur.width != checked.width
            || cur.height != checked.height
            || source_timing(cur) != checked.timing
        {
            continue;
        }
        match checked.found {
            Found::Missing(m) => {
                missing += usize::from(m);
                if cur.missing != m {
                    cur.missing = m;
                    changed = true;
                }
            }
            Found::Probed(mut nf) => {
                preserve_interpretation(&mut nf, cur);
                probed += 1;
                *cur = *nf;
                changed = true;
            }
        }
    }
    if changed {
        let dirty = s.is_dirty();
        s.bump();
        if !dirty {
            // Only the disk changed: the project isn't modified.
            s.mark_saved();
        }
        s.events.push(crate::Event::PurgeCaches);
    }
    if missing > 0 {
        s.toast(format!("{missing} footage item(s) missing (File ▸ Dependencies ▸ Find Missing Footage)"));
    }
    json!({"checked": checked, "missing": missing, "probed": probed})
}

pub(crate) fn command(s: &mut Session, p: &Value) -> crate::Result<Value> {
    let items = p.get("items").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(ItemId).collect());
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false);
    start(s, items, wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with(paths: &[&str]) -> Session {
        let mut s = Session::default();
        for (i, p) in paths.iter().enumerate() {
            let f = Footage { path: p.to_string(), width: 64, height: 48, has_video: true, ..Default::default() };
            std::sync::Arc::make_mut(&mut s.project).add_item(&format!("f{i}"), crate::color::Label::None, None, ItemKind::Footage(f));
        }
        s
    }

    #[test]
    fn check_flags_missing_footage_without_an_undo_step_or_dirtying() {
        let dir = std::env::temp_dir().join(format!("ec-footage-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let there = dir.join("there.png");
        std::fs::write(&there, b"x").unwrap();
        let mut s = session_with(&[there.to_str().unwrap(), "/nonexistent/ec/gone.png"]);
        s.mark_saved();
        let r = s.execute("footage.check", json!({"wait": true})).unwrap();
        assert_eq!(r, json!({"checked": 2, "missing": 1, "probed": 0}));
        let missing: Vec<bool> = s
            .project
            .items
            .values()
            .filter_map(|i| match &i.kind {
                ItemKind::Footage(f) => Some(f.missing),
                _ => None,
            })
            .collect();
        assert_eq!(missing, [false, true]);
        assert!(s.history.undo.is_empty() && !s.is_dirty());
        // In the background: listed as a job until polled.
        std::fs::remove_file(&there).unwrap();
        let r = s.execute("footage.check", json!({})).unwrap();
        assert_eq!(r["started"], true);
        assert!(s.jobs().iter().any(|j| j.kind == "footageCheck"));
        let t0 = std::time::Instant::now();
        while !s.tasks.is_empty() && t0.elapsed().as_secs() < 10 {
            s.poll_jobs();
            std::thread::yield_now();
        }
        assert!(s.project.items.values().all(|i| matches!(&i.kind, ItemKind::Footage(f) if f.missing)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_starts_the_check_when_asked() {
        let mut s = session_with(&["/nonexistent/ec/a.mov"]);
        let path = std::env::temp_dir().join(format!("ec-footage-open-{}.ecproj", std::process::id()));
        s.execute("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
        let mut o = Session { check_footage_on_open: true, ..Default::default() };
        o.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(o.jobs().iter().any(|j| j.kind == "footageCheck"), "{:?}", o.jobs());
        let mut plain = Session::default();
        plain.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(plain.jobs().is_empty());
        let _ = std::fs::remove_file(path);
    }
}
