//! Real-pointer regressions for Project multi-drag and painted shape snapping.
use effectcraft_engine::Session;
use effectcraft_engine::project::{ItemId, LayerId};
use effectcraft_engine::render::{Backend, RenderOpts};
use effectcraft_engine::time::Tick;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::panels::viewer::comp_to_screen;
use egui::{Event, Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let entry = h.state().auto.find(id).unwrap_or_else(|| panic!("missing visible {id}"));
    let rect = Rect::from_min_size(pos2(entry.rect[0], entry.rect[1]), vec2(entry.rect[2], entry.rect[3]));
    assert!(rect.is_positive() && rect.center().x >= 0.0 && rect.center().x < 1600.0 && rect.center().y >= 0.0 && rect.center().y < 1000.0);
    rect
}
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for index in 1..=12 {
        h.event(Event::PointerMoved(from + (to - from) * (index as f32 / 12.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}
fn project_harness() -> (Harness<'static, EffectcraftApp>, [u64; 2]) {
    let mut s = Session::default();
    let first = s.execute("comp.new", json!({"name":"First Clip","width":160,"height":90,"frameRate":30,"duration":1})).unwrap()["comp"].as_u64().unwrap();
    let second = s.execute("comp.new", json!({"name":"Second Clip","width":160,"height":90,"frameRate":30,"duration":1})).unwrap()["comp"].as_u64().unwrap();
    s.execute("comp.new", json!({"name":"Main","width":320,"height":180,"frameRate":30,"duration":4})).unwrap();
    for name in ["Bottom", "Middle", "Top"] {
        s.execute("layer.newSolid", json!({"name":name,"color":"#406080"})).unwrap();
    }
    s.execute("time.set", json!({"time":1.0})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, [first, second])
}
fn drop_on_timeline(h: &mut Harness<'_, EffectcraftApp>, item: u64) {
    let from = rect(h, &format!("project.item.{item}.name")).center();
    let bottom = h.state().session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Bottom").unwrap().id;
    let row = rect(h, &format!("timeline.layer.{}.row", bottom.0));
    let bar = rect(h, &format!("timeline.layer.{}.bar", bottom.0));
    drag(h, from, pos2(bar.min.x + bar.width() * 0.5, row.min.y + 2.0));
}
fn clips(h: &Harness<'_, EffectcraftApp>, ids: &[u64]) -> Vec<u64> {
    h.state()
        .session
        .active_comp()
        .unwrap()
        .layers
        .iter()
        .filter_map(|layer| layer.source.item())
        .map(|item| item.0)
        .filter(|item| ids.contains(item))
        .collect()
}
#[test]
fn project_single_item_drop_keeps_existing_placement() {
    let (mut h, ids) = project_harness();
    h.state_mut().session.state.project_selection = vec![ItemId(ids[0])];
    h.run_steps(2);
    drop_on_timeline(&mut h, ids[0]);
    assert_eq!(clips(&h, &ids), vec![ids[0]], "single project item control");
    let comp = h.state().session.active_comp().unwrap();
    let layer = comp.layers.iter().find(|layer| layer.source.item() == Some(ItemId(ids[0]))).unwrap();
    assert!((layer.in_point.seconds() - 2.0).abs() <= 1.0 / 30.0 + 1e-9);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 3);
}
#[test]
fn selected_project_items_drop_together_in_order_and_one_undo() {
    let (mut h, ids) = project_harness();
    let locked = h.state().session.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Bottom").unwrap().id;
    h.state_mut().session.execute("layer.setSwitch", json!({"layers":[locked.0],"switch":"locked","value":true})).unwrap();
    let original = h.state().session.active_comp().unwrap().layer(locked).unwrap().clone();
    h.state_mut().session.state.project_selection = ids.map(ItemId).to_vec();
    h.run_steps(2);
    assert_eq!(h.state().session.state.project_selection, ids.map(ItemId));
    drop_on_timeline(&mut h, ids[0]);
    let added = clips(&h, &ids);
    eprintln!("selected project IDs={ids:?}; actual added source IDs={added:?}");
    assert_eq!(added, ids, "every highlighted project item must be added in selection order");
    let comp = h.state().session.active_comp().unwrap();
    assert_eq!(comp.layers.len(), 5);
    assert_eq!(comp.layer(locked).unwrap(), &original, "insertion does not edit the locked target");
    for layer in comp.layers.iter().filter(|layer| layer.source.item().is_some_and(|item| ids.contains(&item.0))) {
        assert!((layer.in_point.seconds() - 2.0).abs() <= 1.0 / 30.0 + 1e-9, "both clips share drop time");
    }
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 3, "one gesture removes both added clips");
}
#[test]
fn dragging_an_unselected_project_item_does_not_add_other_selected_items() {
    let (mut h, ids) = project_harness();
    h.state_mut().session.state.project_selection = vec![ItemId(ids[1])];
    h.run_steps(2);
    drop_on_timeline(&mut h, ids[0]);
    assert_eq!(clips(&h, &ids), vec![ids[0]], "only the grabbed unselected item is dragged");
}
#[test]
fn dragging_a_project_folder_into_the_timeline_keeps_the_composition() {
    let (mut h, _) = project_harness();
    let folder = h.state_mut().session.execute("project.newFolder", json!({"name":"Folder"})).unwrap()["item"].as_u64().unwrap();
    h.state_mut().session.state.project_selection = vec![ItemId(folder)];
    h.run_steps(2);
    drop_on_timeline(&mut h, folder);
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 3);
}
fn snap_harness(shape: bool) -> (Harness<'static, EffectcraftApp>, LayerId) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Snapping","width":640,"height":360,"duration":4})).unwrap();
    let command = if shape { "layer.newShape" } else { "layer.newSolid" };
    for (name, position, color) in [("Target", [500.0, 200.0], "#00ff00"), ("Moving", [150.0, 200.0], "#ff0000")] {
        let params = if shape {
            json!({"kind":"rect","name":name,"size":[100,100],"position":position,"fill":color,"stroke":false,"strokeWidth":0})
        } else {
            json!({"name":name,"width":100,"height":100,"color":color})
        };
        let layer = s.execute(command, params).unwrap()["layer"].as_u64().unwrap();
        if !shape {
            s.execute("prop.set", json!({"layer":layer,"path":"transform/position","value":[position[0],position[1],0]})).unwrap();
        }
    }
    let moving = s.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Moving").unwrap().id;
    s.execute("edit.deselectAll", json!({})).unwrap();
    s.execute("view.snapping", json!({"value":true})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, moving)
}
fn check_snapped_painted_edges(shape: bool) {
    let (mut h, moving) = snap_harness(shape);
    let from = comp_to_screen(&h.ctx, [198.0, 152.0]).unwrap();
    let to = comp_to_screen(&h.ctx, [445.0, 152.0]).unwrap();
    drag(&mut h, from, to);
    let s = &h.state().session;
    let comp = s.active_comp_id().unwrap();
    let position = s.active_comp().unwrap().layer(moving).unwrap().props.prop("transform/position").unwrap().value.as_vec3();
    let image = s.render(comp, Tick::ZERO, RenderOpts { backend: Backend::Cpu, ..Default::default() });
    assert_eq!((image.width, image.height), (640, 360));
    let blank: Vec<usize> = (360..540)
        .filter(|x| {
            let pixel = image.data[200 * image.width as usize + *x];
            pixel[0] < 0.5 && pixel[1] < 0.5
        })
        .collect();
    eprintln!("shape={shape}; moving_position={position:?}; independent painted gap columns={blank:?}");
    assert!((position[0] - 400.0).abs() < 0.01, "the two actual 100px content edges should meet: {position:?}");
    assert!(blank.is_empty(), "snapped filled objects must meet without an unpainted gap: {blank:?}");
}
#[test]
fn solid_corner_snapping_keeps_painted_edges_aligned() {
    check_snapped_painted_edges(false);
}
#[test]
fn filled_shape_corner_snapping_does_not_leave_an_antialias_padding_gap() {
    check_snapped_painted_edges(true);
}

#[test]
fn selected_project_items_keep_their_reverse_selection_order() {
    let (mut h, ids) = project_harness();
    let selected = [ids[1], ids[0]];
    h.state_mut().session.state.project_selection = selected.map(ItemId).to_vec();
    h.run_steps(2);
    drop_on_timeline(&mut h, ids[0]);
    assert_eq!(clips(&h, &ids), selected);
    let comp = h.state().session.active_comp().unwrap();
    let names: Vec<_> = comp.layers.iter().map(|layer| layer.name.as_str()).collect();
    assert_eq!(names, ["Top", "Middle", "Second Clip", "First Clip", "Bottom"]);
    let times: Vec<_> = comp.layers.iter().filter(|layer| layer.source.item().is_some_and(|item| ids.contains(&item.0))).map(|layer| layer.in_point).collect();
    assert_eq!(times.len(), 2);
    assert_eq!(times[0], times[1]);
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 3);
}

#[test]
fn project_multi_drop_rolls_back_when_a_selected_folder_cannot_be_added() {
    let (mut h, ids) = project_harness();
    let folder = h.state_mut().session.execute("project.newFolder", json!({"name":"Invalid Folder"})).unwrap()["item"].as_u64().unwrap();
    let before = h.state().session.project.clone();
    let history = h.state_mut().session.execute("edit.history.list", json!({})).unwrap();
    h.state_mut().session.state.project_selection = vec![ItemId(ids[0]), ItemId(folder)];
    h.run_steps(2);
    drop_on_timeline(&mut h, ids[0]);
    assert_eq!(&*h.state().session.project, &*before, "an invalid multi-drop is atomic");
    let after = h.state_mut().session.execute("edit.history.list", json!({})).unwrap();
    assert_eq!(after, history, "a rolled-back drop does not create an undo entry");
}

fn check_stroked_snap_edges(group_scale: f64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name":"Stroke snapping","width":640,"height":360,"duration":4})).unwrap();
    for (name, position, color) in [("Target", [500.0, 200.0], "#00ff00"), ("Moving", [150.0, 200.0], "#ff0000")] {
        let layer = s
            .execute("layer.newShape", json!({"kind":"rect","name":name,"size":[100,100],"position":position,"fill":color,"stroke":color,"strokeWidth":10}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        if group_scale != 1.0 {
            let prop = s
                .active_comp()
                .unwrap()
                .layer(LayerId(layer))
                .unwrap()
                .props
                .sub("contents")
                .unwrap()
                .groups()
                .find(|g| g.match_id == "group")
                .unwrap()
                .sub("transform")
                .unwrap()
                .get("scale")
                .unwrap()
                .uid;
            s.execute("prop.set", json!({"layer":layer,"prop":prop,"value":[group_scale*100.0,group_scale*100.0]})).unwrap();
        }
    }
    let moving = s.active_comp().unwrap().layers.iter().find(|layer| layer.name == "Moving").unwrap().id;
    s.execute("edit.deselectAll", json!({})).unwrap();
    s.execute("view.snapping", json!({"value":true})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(s));
    h.run_steps(3);
    let half_width = 55.0 * group_scale;
    let from = comp_to_screen(&h.ctx, [(150.0 + half_width - 2.0) as f32, (200.0 - half_width + 2.0) as f32]).unwrap();
    let to = comp_to_screen(&h.ctx, [(500.0 - half_width - 5.0) as f32, (200.0 - half_width + 2.0) as f32]).unwrap();
    drag(&mut h, from, to);
    let s = &h.state().session;
    let position = s.active_comp().unwrap().layer(moving).unwrap().props.prop("transform/position").unwrap().value.as_vec3();
    let image = s.render(s.active_comp_id().unwrap(), Tick::ZERO, RenderOpts { backend: Backend::Cpu, ..Default::default() });
    let blank: Vec<_> = (360..540)
        .filter(|x| {
            let pixel = image.data[200 * image.width as usize + *x];
            pixel[0] < 0.5 && pixel[1] < 0.5
        })
        .collect();
    eprintln!("stroke scale={group_scale} moving_position={position:?} painted_gap={blank:?}");
    assert!((position[0] - (500.0 - 2.0 * half_width)).abs() < 0.01, "actual stroked edges align");
    assert!(blank.is_empty(), "rendered strokes meet without empty pixels");
}
#[test]
fn stroked_shape_snapping_uses_the_painted_outline() {
    check_stroked_snap_edges(1.0);
}
#[test]
fn shape_group_scaled_strokes_snap_at_the_scaled_painted_outline() {
    check_stroked_snap_edges(2.0);
}

fn check_scale_handle_painted_edges(shape: bool) {
    let (mut h, moving) = snap_harness(shape);
    h.state_mut().session.execute("layer.select", json!({"layers":[moving.0]})).unwrap();
    h.run_steps(2);
    let from = rect(&h, &format!("viewer.handle.{}.5", moving.0)).center();
    let to = comp_to_screen(&h.ctx, [445.0, 200.0]).unwrap();
    drag(&mut h, from, to);
    let s = &h.state().session;
    let layer = s.active_comp().unwrap().layer(moving).unwrap();
    let position = layer.props.prop("transform/position").unwrap().value.as_vec3();
    let scale = layer.props.prop("transform/scale").unwrap().value.as_vec3();
    let image = s.render(s.active_comp_id().unwrap(), Tick::ZERO, RenderOpts { backend: Backend::Cpu, ..Default::default() });
    let blank: Vec<_> = (430..540)
        .filter(|x| {
            let pixel = image.data[200 * image.width as usize + *x];
            pixel[0] < 0.5 && pixel[1] < 0.5
        })
        .collect();
    eprintln!("scale_handle shape={shape}; position={position:?}; scale={scale:?}; independent gap={blank:?}");
    assert_eq!(position, [150.0, 200.0, 0.0], "the visible handle scales without moving");
    assert!((scale[0] - 600.0).abs() < 0.05 && (scale[1] - 100.0).abs() < 0.05, "the painted right edge reaches the snap target: {scale:?}");
    assert!(blank.is_empty(), "the actual rendered resize must leave no gap: {blank:?}");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    let scale = h.state().session.active_comp().unwrap().layer(moving).unwrap().props.prop("transform/scale").unwrap().value.as_vec3();
    assert_eq!(scale, [100.0, 100.0, 100.0], "one Undo restores the complete scale gesture");
}
#[test]
fn solid_scale_handle_snaps_to_the_painted_edge() {
    check_scale_handle_painted_edges(false);
}
#[test]
fn filled_shape_scale_handle_snaps_to_the_painted_edge() {
    check_scale_handle_painted_edges(true);
}
