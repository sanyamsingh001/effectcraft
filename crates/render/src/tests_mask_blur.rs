//! Mask motion blur follows the renderer's motion blur gate: the render options (Render
//! Settings ▸ Motion Blur, previews), the comp's switch and the precomp layers above.

use effectcraft_color::Label;
use effectcraft_keyframe::{Keyframe, ShapePath, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Comp, GroupKind, ItemId, ItemKind, LayerSource, MaskMode, MaskMotionBlur, Node, Project, Solid};
use effectcraft_time::{FrameRate, Tick};

use crate::{NoFootage, RenderOpts, Renderer, masks};
use effectcraft_effects::Buf;
use effectcraft_raster::Image;

/// A white 100×100 solid whose rect mask jumps from x 10 (frame 0) to x 50 (frame 1), Mask
/// Motion Blur On, the layer's own switch off. Rendered half way, unblurred the mask covers x
/// 30..50; blurred over the shutter it reaches down to x ≈ 20.
fn project(comp_blur: bool) -> (Project, ItemId) {
    let mut p = Project::default();
    let mut comp = Comp::new(100, 100, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    comp.enable_motion_blur = comp_blur;
    let sid = p.add_item("S", Label::Red, None, ItemKind::Solid(Solid { color: [1.0; 3], width: 100, height: 100, pixel_aspect: 1.0 }));
    let mut l = build::layer(&mut p, &comp, "S", LayerSource::Solid { item: sid }, (100, 100), None);
    let mut next = p.next_id;
    let mut m = build::mask(&mut Ids(&mut next), "Mask 1", ShapePath::rect([20.0, 50.0], 20.0, 100.0), MaskMode::Add, [255, 0, 0]);
    p.next_id = next;
    if let GroupKind::Mask { motion_blur, .. } = &mut m.kind {
        *motion_blur = MaskMotionBlur::On;
    }
    let path = m.get_mut("path").unwrap();
    path.keys = vec![
        Keyframe::new(Tick::ZERO, Value::Path(ShapePath::rect([20.0, 50.0], 20.0, 100.0))),
        Keyframe::new(FrameRate::FPS_30.tick_of(1), Value::Path(ShapePath::rect([60.0, 50.0], 20.0, 100.0))),
    ];
    l.props.sub_mut("masks").unwrap().children.push(Node::Group(m));
    l.switches.motion_blur = false;
    let cid = p.add_item("C", Label::Sandstone, None, ItemKind::Comp(comp.into()));
    p.comp_mut(cid).unwrap().layers.push(l);
    (p, cid)
}

fn alpha_at_25(p: &Project, cid: ItemId, motion_blur: bool) -> f32 {
    let half = Tick::from_seconds_f64(0.5 / 30.0);
    Renderer::new(p, &NoFootage, RenderOpts { motion_blur, ..Default::default() }).comp_frame(cid, half).get(25, 50)[3]
}

#[test]
fn mask_motion_blur_follows_the_render_and_the_comp() {
    let (p, cid) = project(true);
    let a = alpha_at_25(&p, cid, true);
    assert!(a > 0.05 && a < 0.95, "blurred: {a}");
    assert!(alpha_at_25(&p, cid, false) < 0.01, "Render Settings ▸ Motion Blur off");
    let (p, cid) = project(false);
    assert!(alpha_at_25(&p, cid, true) < 0.01, "the comp's switch off");
}

fn frame_mask(mode: MaskMode, inverted: bool, extra: bool, blur_off: bool) -> Buf {
    let mut project = Project::default();
    let mut comp = Comp::new(100, 100, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    comp.shutter_angle = 360.0;
    comp.shutter_phase = -360.0;
    let solid = project.add_item("White", Default::default(), None, ItemKind::Solid(Solid { width: 100, height: 100, color: [1.0; 3], pixel_aspect: 1.0 }));
    let mut layer = build::layer(&mut project, &comp, "White", LayerSource::Solid { item: solid }, (100, 100), None);
    let mut next = project.next_id;
    let closed = ShapePath::rect([50.0, 50.0], 40.0, 40.0);
    let mut open = closed.clone();
    open.closed = false;
    let mut mask = build::mask(&mut Ids(&mut next), "Closing", open.clone(), mode, [255, 255, 0]);
    if let GroupKind::Mask { motion_blur, inverted: inv, .. } = &mut mask.kind {
        *motion_blur = if blur_off { MaskMotionBlur::Off } else { MaskMotionBlur::On };
        *inv = inverted;
    }
    mask.get_mut("path").unwrap().keys = vec![Keyframe::new(Tick::ZERO, Value::Path(open)), Keyframe::new(FrameRate::FPS_30.tick_of(1), Value::Path(closed))];
    layer.props.sub_mut("masks").unwrap().children.push(mask.into());
    if extra {
        let mut additional = build::mask(&mut Ids(&mut next), "Always Subtract", ShapePath::rect([50.0, 50.0], 20.0, 20.0), MaskMode::Subtract, [0, 255, 255]);
        if let GroupKind::Mask { motion_blur, .. } = &mut additional.kind {
            *motion_blur = MaskMotionBlur::Off;
        }
        additional.get_mut("path").unwrap().keys = vec![
            Keyframe::new(Tick::ZERO, Value::Path(ShapePath::rect([10.0, 10.0], 20.0, 20.0))),
            Keyframe::new(FrameRate::FPS_30.tick_of(1), Value::Path(ShapePath::rect([50.0, 50.0], 20.0, 20.0))),
        ];
        layer.props.sub_mut("masks").unwrap().children.push(additional.into());
    }
    let comp_id = project.add_item("Comp", Default::default(), None, ItemKind::Comp(comp.clone().into()));
    let ctx = crate::EvalCtx::new(&project, comp_id, &comp, FrameRate::FPS_30.tick_of(1));
    let mut image = Image::new(100, 100);
    image.data.fill([1.0; 4]);
    let mut buf = Buf { img: image, offset: [0.0; 2], scale: 1.0 };
    assert!(masks::apply(&ctx, &layer, &mut buf, masks::MaskBlur { allowed: true, layer: false, samples: 2 }));
    buf
}
#[test]
fn closing_add_mask_preserves_inside_and_averages_outside() {
    let buf = frame_mask(MaskMode::Add, false, false, false);
    assert_eq!(buf.img.get(50, 50)[3], 1.0, "open and covered closed samples both preserve this pixel");
    assert_eq!(buf.img.get(10, 10)[3], 0.5, "only the open shutter sample preserves this outside pixel");
}
#[test]
fn closing_inverted_add_mask_averages_inside_and_preserves_outside() {
    let buf = frame_mask(MaskMode::Add, true, false, false);
    assert_eq!(buf.img.get(50, 50)[3], 0.5);
    assert_eq!(buf.img.get(10, 10)[3], 1.0);
}
#[test]
fn closing_subtract_mask_preserves_outside_and_averages_cutout() {
    let buf = frame_mask(MaskMode::Subtract, false, false, false);
    assert_eq!(buf.img.get(50, 50)[3], 0.5);
    assert_eq!(buf.img.get(10, 10)[3], 1.0);
}
#[test]
fn closing_add_mask_keeps_non_blurred_subtract_at_current_time() {
    let buf = frame_mask(MaskMode::Add, false, true, false);
    assert_eq!(buf.img.get(50, 50)[3], 0.0);
    assert_eq!(buf.img.get(35, 50)[3], 1.0);
    assert_eq!(buf.img.get(10, 10)[3], 0.5);
}
#[test]
fn mask_motion_blur_off_uses_only_the_current_closed_path() {
    let buf = frame_mask(MaskMode::Add, false, false, true);
    assert_eq!(buf.img.get(50, 50)[3], 1.0);
    assert_eq!(buf.img.get(10, 10)[3], 0.0);
}
