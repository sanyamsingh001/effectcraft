//! Interpret Footage numeric forms emit floating-point JSON, including whole loop counts.

use effectcraft_engine::{
    Session,
    project::{Footage, FootageKind, ItemKind},
};
use effectcraft_ui_egui::panels::forms::{Field, Form};
use serde_json::json;

#[test]
fn numeric_loop_form_values_apply_and_keep_existing_limits() {
    let mut session = Session::default();
    let item = std::sync::Arc::make_mut(&mut session.project).add_item(
        "Sequence",
        effectcraft_engine::color::Label::Lavender,
        None,
        ItemKind::Footage(Footage { kind: FootageKind::Sequence, sequence: vec!["frame_0001.png".into(), "frame_0002.png".into()], ..Default::default() }),
    );
    session.state.project_selection = vec![item];
    for (value, expected) in [(3.0, 3), (0.0, 1), (999999.0, 9999)] {
        let form = Form {
            command: "file.interpretFootage".into(),
            base: json!({"items": [item.0]}),
            fields: vec![Field::num("loop", "Loop", value)],
            ..Default::default()
        };
        let params = form.params();
        assert!(params["loop"].as_u64().is_none(), "numeric fields use float JSON even for whole values");
        session.execute_checked(&form.command, params).unwrap();
        let ItemKind::Footage(footage) = &session.project.item(item).unwrap().kind else { panic!("footage") };
        assert_eq!(footage.loop_count, expected);
    }
}
