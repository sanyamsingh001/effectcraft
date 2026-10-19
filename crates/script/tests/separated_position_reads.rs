//! Script values of separated Position agree with actual follower edits and rendering.
use effectcraft_engine::{Session, project::LayerId, render::EvalCtx, time::Tick};
use serde_json::{Value, json};
use std::sync::Arc;

fn setup(three_d: bool) -> (Session, u64) {
    let mut session = Session { expr: Some(Arc::new(effectcraft_expr::Expressions)), expr_check: Some(effectcraft_expr::check_syntax), ..Default::default() };
    effectcraft_script::install(&mut session);
    session.execute_checked("comp.new", json!({"name":"Separated","width":100,"height":80,"duration":4,"frameRate":30})).unwrap();
    let layer = session.execute_checked("layer.newSolid", json!({"name":"Box","width":10,"height":10})).unwrap()["layer"].as_u64().unwrap();
    if three_d {
        session.execute_checked("layer.setSwitch", json!({"layer":layer,"switch":"threeD","value":true})).unwrap();
    }
    session.execute_checked("prop.separateDimensions", json!({"layer":layer})).unwrap();
    (session, layer)
}
fn assert_script_values(mut session: Session, layer: u64, time: f64, raw: Value, evaluated: Value) {
    session.set_time(Tick::from_seconds_f64(time));
    let control = session.execute_checked("prop.get", json!({"layer":layer,"path":"transform/position","time":time})).unwrap();
    assert_eq!(control["value"], raw, "public property reads combine the written followers");
    let comp_id = session.active_comp_id().unwrap();
    let comp = session.project.comp(comp_id).unwrap();
    let owner = comp.layer(LayerId(layer)).unwrap();
    let property = owner.props.prop("transform/position").unwrap();
    let mut context = EvalCtx::new(&session.project, comp_id, comp, Tick::from_seconds_f64(time));
    context.expr = session.expr.as_deref();
    assert_eq!(context.value(owner, property).to_json(), evaluated, "the renderer reads the real follower tracks");
    let before = session.project.clone();
    let history = session.history.undo.len();
    let code = format!(
        "var p=app.project.activeItem.layer(1).property('ADBE Transform Group').property('ADBE Position'); [p.value,p.valueAtTime({time},true),p.valueAtTime({time},false)]"
    );
    let outcome = effectcraft_script::run_code(&mut session, &code, "separated-reads.jsx");
    assert!(outcome.error.is_none(), "public script query failed: {:?}; output={:?}", outcome.error, outcome.output);
    eprintln!("script={}; property={control}; expected-raw={raw}; expected-evaluated={evaluated}", outcome.result);
    let rows = outcome.result.as_array().expect("the real script returns three value rows");
    assert_eq!(rows.len(), 3);
    for (actual, expected) in rows.iter().zip([&evaluated, &raw, &evaluated]) {
        let actual = actual.as_array().expect("script Position is a vector");
        let expected = expected.as_array().expect("the fixture expects a vector");
        assert_eq!(actual.len(), expected.len());
        for (component, expected) in actual.iter().zip(expected) {
            assert_eq!(component.as_f64(), expected.as_f64(), "script .value and both valueAtTime modes must observe separated followers");
        }
    }
    assert!(Arc::ptr_eq(&session.project, &before), "reading script values does not edit the project");
    assert_eq!(session.history.undo.len(), history);
}
#[test]
fn script_reads_separated_position_after_a_static_leader_api_write() {
    let (mut session, layer) = setup(false);
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[12,23]})).unwrap();
    assert_script_values(session, layer, 0.0, json!([12.0, 23.0, 0.0]), json!([12.0, 23.0, 0.0]));
}
#[test]
fn script_reads_separated_position_after_animated_3d_leader_api_writes() {
    let (mut session, layer) = setup(true);
    for (time, value) in [(0.0, [10, 20, 30]), (1.0, [30, 40, 50])] {
        session.execute_checked("prop.addKey", json!({"layer":layer,"path":"transform/position","time":time,"value":value})).unwrap();
    }
    assert_script_values(session, layer, 0.5, json!([20.0, 30.0, 40.0]), json!([20.0, 30.0, 40.0]));
}
#[test]
fn script_pre_expression_position_keeps_raw_axes_while_value_evaluates_them() {
    let (mut session, layer) = setup(false);
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[12,23]})).unwrap();
    session.execute_checked("prop.setExpression", json!({"layer":layer,"path":"transform/positionX","expression":"100"})).unwrap();
    assert_script_values(session, layer, 0.0, json!([12.0, 23.0, 0.0]), json!([100.0, 23.0, 0.0]));
}

#[test]
fn ordinary_unseparated_position_script_reads_keep_their_vector_values() {
    let mut session = Session { expr: Some(Arc::new(effectcraft_expr::Expressions)), expr_check: Some(effectcraft_expr::check_syntax), ..Default::default() };
    effectcraft_script::install(&mut session);
    session.execute_checked("comp.new", json!({"width":100,"height":80,"duration":4})).unwrap();
    let layer = session.execute_checked("layer.newSolid", json!({"name":"Box"})).unwrap()["layer"].as_u64().unwrap();
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[12,23]})).unwrap();
    assert_script_values(session, layer, 0.0, json!([12.0, 23.0, 0.0]), json!([12.0, 23.0, 0.0]));
}
#[test]
fn ordinary_expression_property_script_reads_keep_raw_and_evaluated_modes() {
    let (mut session, layer) = setup(false);
    session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/opacity","value":20})).unwrap();
    session.execute_checked("prop.setExpression", json!({"layer":layer,"path":"transform/opacity","expression":"value + 5"})).unwrap();
    let property = session.execute_checked("prop.get", json!({"layer":layer,"path":"transform/opacity"})).unwrap();
    assert_eq!(property["value"].as_f64(), Some(20.0));
    assert_eq!(property["evaluated"].as_f64(), Some(25.0));
    let outcome = effectcraft_script::run_code(
        &mut session,
        "var p=app.project.activeItem.layer(1).property('ADBE Transform Group').property('ADBE Opacity'); [p.value,p.valueAtTime(0,true),p.valueAtTime(0,false)]",
        "ordinary-reads.jsx",
    );
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let rows = outcome.result.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for (actual, expected) in rows.iter().zip([25.0, 20.0, 25.0]) {
        assert_eq!(actual.as_f64(), Some(expected));
    }
}
#[test]
fn script_separated_position_reads_keep_missing_axis_fallbacks() {
    for (removed, expected) in [("positionY", [12.0, 0.0, 0.0]), ("positionX", [50.0, 40.0, 0.0])] {
        let (mut session, layer) = setup(false);
        session.execute_checked("prop.set", json!({"layer":layer,"path":"transform/position","value":[12,23]})).unwrap();
        let comp_id = session.active_comp_id().unwrap();
        let transform = Arc::make_mut(&mut session.project).comp_mut(comp_id).unwrap().layer_mut(LayerId(layer)).unwrap().props.sub_mut("transform").unwrap();
        transform.children.retain(|node| node.match_id() != removed);
        let comp = session.active_comp().unwrap();
        let owner = comp.layer(LayerId(layer)).unwrap();
        let property = owner.props.prop("transform/position").unwrap();
        let context = EvalCtx::new(&session.project, comp_id, comp, Tick::ZERO);
        assert_eq!(context.value(owner, property).as_vec3(), expected);
        assert_eq!(context.keyframed_value(owner, property, Tick::ZERO).as_vec3(), expected);
        let outcome = effectcraft_script::run_code(
            &mut session,
            "var p=app.project.activeItem.layer(1).property('ADBE Transform Group').property('ADBE Position'); [p.value,p.valueAtTime(0,true),p.valueAtTime(0,false)]",
            "missing-axes.jsx",
        );
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let rows = outcome.result.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        for row in rows {
            let components = row.as_array().unwrap();
            assert_eq!(components.len(), 3);
            for (component, expected) in components.iter().zip(expected) {
                assert_eq!(component.as_f64(), Some(expected));
            }
        }
    }
}
