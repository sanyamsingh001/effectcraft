use effectcraft_keyframe::{Keyframe, ShapePath, Value};
use effectcraft_time::{FrameRate, Tick};

use crate::build::{self, Ids};
use crate::{Comp, ItemKind, LayerSource, MaskMode, Project, Solid};

fn project_with_layer() -> (Project, crate::ItemId) {
    let mut p = Project::default();
    let comp = Comp::new(1920, 1080, FrameRate::FPS_29_97, Tick::from_seconds_f64(10.0));
    let cid = p.add_item("Main", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let sid = p.add_item(
        "Red Solid",
        effectcraft_color::Label::Red,
        None,
        ItemKind::Solid(Solid { color: [1.0, 0.0, 0.0], width: 1920, height: 1080, pixel_aspect: 1.0 }),
    );
    let layer = build::layer(&mut p, &comp, "Red Solid 1", LayerSource::Solid { item: sid }, (1920, 1080), None);
    p.comp_mut(cid).unwrap().layers.push(layer);
    (p, cid)
}

#[test]
fn paths_resolve() {
    let (p, cid) = project_with_layer();
    let l = &p.comp(cid).unwrap().layers[0];
    assert_eq!(l.props.prop("transform/position").unwrap().value, Value::Vec3([960.0, 540.0, 0.0]));
    assert_eq!(l.props.prop("Transform/Opacity").unwrap().value, Value::Scalar(100.0));
    assert_eq!(l.props.prop("transform/#7").unwrap().match_id, "rotation");
    let uid = l.props.prop("transform/scale").unwrap().uid;
    let path = l.props.path_of(uid).unwrap();
    assert_eq!(l.props.prop(&path).unwrap().match_id, "scale");
    assert_eq!(l.props.name_path_of(uid).unwrap(), "Transform/Scale");
}

#[test]
fn masks_and_occurrence_paths() {
    let (mut p, cid) = project_with_layer();
    let mut next = p.next_id;
    let m1 = build::mask(&mut Ids(&mut next), "Mask 1", ShapePath::rect([0.0, 0.0], 10.0, 10.0), MaskMode::Add, [255, 0, 0]);
    let m2 = build::mask(&mut Ids(&mut next), "Mask 2", ShapePath::ellipse([0.0, 0.0], 10.0, 10.0), MaskMode::Subtract, [0, 255, 0]);
    p.next_id = next;
    let l = &mut p.comp_mut(cid).unwrap().layers[0];
    let masks = l.props.sub_mut("masks").unwrap();
    masks.children.push(m1.into());
    masks.children.push(m2.into());
    assert_eq!(l.props.group("masks/mask#2").unwrap().name, "Mask 2");
    assert_eq!(l.props.group("masks/Mask 2").unwrap().name, "Mask 2");
    assert!(l.props.prop("masks/mask#2/feather").is_some());
}

#[test]
fn animate_and_roundtrip_json() {
    let (mut p, cid) = project_with_layer();
    {
        let l = &mut p.comp_mut(cid).unwrap().layers[0];
        let op = l.props.prop_mut("transform/opacity").unwrap();
        op.set_animated(true, Tick::ZERO);
        op.set_value_at(Tick::from_seconds_f64(1.0), Value::Scalar(0.0));
        assert_eq!(op.keys.len(), 2);
        assert_eq!(op.value_at(Tick::from_seconds_f64(0.5)), Value::Scalar(50.0));
    }
    let json = p.to_json();
    let q = Project::from_json(&json).unwrap();
    assert_eq!(p, q);
    let _ = Keyframe::new(Tick::ZERO, Value::Scalar(0.0));
}

#[test]
fn unique_names_and_cycles() {
    let (p, cid) = project_with_layer();
    let c = p.comp(cid).unwrap();
    assert_eq!(c.unique_layer_name("Red Solid 1"), "Red Solid 2");
    assert_eq!(c.unique_layer_name("Shape Layer 1"), "Shape Layer 1");
    assert!(p.comp_contains(cid, cid));
}

#[test]
fn camera_and_light_layers_build() {
    let (mut p, cid) = project_with_layer();
    let comp = p.comp(cid).unwrap().clone();
    let cam = build::layer(&mut p, &comp, "Camera 1", LayerSource::Camera, (0, 0), None);
    assert!(cam.props.prop("cameraOptions/zoom").is_some());
    assert!(cam.props.prop("transform/poi").is_some());
    let light = build::layer(&mut p, &comp, "Light 1", LayerSource::Light { kind: crate::LightKind::Spot }, (0, 0), None);
    assert!(light.props.prop("lightOptions/coneAngle").is_some());
}

#[test]
fn same_content_sees_allocated_ids_and_comp_edits() {
    let (p, cid) = project_with_layer();
    let mut q = p.clone();
    assert!(q.same_content(&p));
    q.alloc();
    assert!(!q.same_content(&p), "an allocated id stays taken");
    let mut q = p.clone();
    // A comp copied on write but left equal is the same; a changed one is not.
    let _ = q.comp_mut(cid).unwrap();
    assert!(q.same_content(&p));
    q.comp_mut(cid).unwrap().layers[0].name = "Renamed".into();
    assert!(!q.same_content(&p));
    let mut r = p.clone();
    r.item_mut(cid).unwrap().name = "Other".into();
    assert!(!r.same_content(&p));
    let mut r = p.clone();
    r.settings.bit_depth = crate::BitDepth::Bpc16;
    assert!(!r.same_content(&p));
}

#[test]
fn project_files_name_their_version_and_always_read_back() {
    let (mut p, cid) = project_with_layer();
    let text = p.to_file_json().unwrap();
    assert_eq!(crate::saved_by(&text).as_deref(), Some(crate::APP_VERSION));
    assert!(text.trim_start().starts_with("{\n  \"savedBy\""), "the version comes first");
    assert_eq!(Project::from_json(&text).unwrap(), p);
    assert_eq!(crate::saved_by(&p.to_json()), None, "plain JSON (worker hand-offs) has none");
    // JSON can't hold NaN: saving refuses instead of writing a file that can't be opened.
    p.comp_mut(cid).unwrap().layers[0].props.prop_mut("transform/opacity").unwrap().value = Value::Scalar(f64::NAN);
    let e = p.to_file_json().unwrap_err().to_string();
    assert!(e.contains("can't be saved"), "{e}");
    assert!(crate::is_newer_version("0.2.0", "0.1.9") && crate::is_newer_version("1.0.0-rc.1", "0.9.9"));
    assert!(!crate::is_newer_version("0.1.1", "0.1.1") && !crate::is_newer_version("0.1.0", "0.1.1"));
}

fn finite_footage(kind: crate::FootageKind, rate: FrameRate, duration: Tick, loops: u32) -> crate::Item {
    crate::Item {
        id: crate::ItemId(1),
        name: "Fractional clip".into(),
        label: effectcraft_color::Label::Lavender,
        comment: String::new(),
        parent: None,
        proxy: None,
        kind: ItemKind::Footage(crate::Footage { kind, duration, frame_rate: rate, loop_count: loops, ..Default::default() }),
    }
}

#[test]
fn video_source_span_rounds_fractional_half_frame_duration_exactly() {
    let rate = FrameRate::new(2469, 200);
    let duration = Tick(30_864_641_556);
    for loops in [1, 3] {
        let clip = finite_footage(crate::FootageKind::Video, rate, duration, loops);
        let frames = 2 * i64::from(loops);
        assert_eq!(clip.source_span(), Some((rate.tick_of(frames), rate.tick_of(frames - 1))));
    }
    let before = finite_footage(crate::FootageKind::Video, rate, duration - Tick(1), 1);
    assert_eq!(before.source_span(), Some((rate.tick_of(1), Tick::ZERO)), "the preceding tick still rounds to one frame");
}

#[test]
fn footage_source_span_changes_only_at_exact_rational_rounding_boundaries() {
    for rate in [FrameRate::new(2469, 200), FrameRate::FPS_24, FrameRate::FPS_30, FrameRate::FPS_23_976, FrameRate::FPS_29_97] {
        let period = i128::from(effectcraft_time::TICKS_PER_SECOND) * i128::from(rate.den);
        let denominator = 2 * i128::from(rate.num);
        for n in (1..=8).chain([1_000_000]) {
            let numerator = i128::from(2 * n + 1) * period;
            let threshold = Tick(i64::try_from((numerator + denominator - 1) / denominator).unwrap());
            for kind in [crate::FootageKind::Video, crate::FootageKind::Audio] {
                for loops in [1, 3] {
                    for (duration, per_loop) in [(threshold - Tick(1), n), (threshold, n + 1)] {
                        let total = per_loop * i64::from(loops);
                        let clip = finite_footage(kind, rate, duration, loops);
                        let ItemKind::Footage(f) = &clip.kind else { panic!("footage") };
                        assert_eq!(f.source_frame_count(), Some(per_loop), "the source count excludes loops and keeps one-tick boundaries exact");
                        assert_eq!(
                            clip.source_span(),
                            Some((rate.tick_of(total), rate.tick_of(total - 1))),
                            "{rate:?} {kind:?} duration={duration:?} loops={loops}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn sequence_source_span_uses_file_count_at_fractional_rate() {
    let rate = FrameRate::new(2469, 200);
    let mut clip = finite_footage(crate::FootageKind::Sequence, rate, Tick(30_864_641_556), 2);
    let ItemKind::Footage(f) = &mut clip.kind else { panic!("footage") };
    f.sequence = vec!["frame1.png".into(), "frame2.png".into(), "frame3.png".into()];
    assert_eq!(f.source_frame_count(), Some(3));
    assert_eq!(clip.source_span(), Some((rate.tick_of(6), rate.tick_of(5))), "sequence files determine the count even when metadata rounds to two frames");
}
