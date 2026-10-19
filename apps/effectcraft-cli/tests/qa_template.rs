//! M13.15 scenario 5 — Essential Graphics through the CLI: a MOGRT-style template built by an
//! After Effects-style script (`effectcraft-cli script`), its controls changed with commands on
//! an instance, rendered, exported as a template and imported into a fresh project. Direct
//! template rendering uses the same original, procedural fixture under the project licence.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn cli(dir: &PathBuf, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_effectcraft-cli")).current_dir(dir).args(args).arg("--json").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}: {stdout} {}", String::from_utf8_lossy(&out.stderr)));
    (out.status.code().unwrap_or(-1), v)
}

fn ok(dir: &PathBuf, args: &[&str]) -> Value {
    let (code, v) = cli(dir, args);
    assert_eq!(code, 0, "{args:?}: {v}");
    v
}

const BUILD: &str = r#"
app.beginUndoGroup("Build Template");
var comp = app.project.items.addComp("Lower Third", 320, 90, 1, 2, 12);
var bar = comp.layers.addSolid([1, 1, 1], "Bar", 320, 40, 1, 2);
bar.property("ADBE Transform Group").property("ADBE Position").setValue([160, 65]);
var t = comp.layers.addText("Jane Doe");
t.name = "Name";
t.property("ADBE Transform Group").property("ADBE Position").setValue([160, 30]);
var fill = bar.property("ADBE Effect Parade").addProperty("ADBE Fill");
t.property("ADBE Text Properties").property("ADBE Text Document").addToMotionGraphicsTemplateAs(comp, "Title");
fill.property("ADBE Fill-0002").addToMotionGraphicsTemplateAs(comp, "Bar Color");
bar.property("ADBE Transform Group").property("ADBE Opacity").addToMotionGraphicsTemplateAs(comp, "Bar Opacity");
comp.motionGraphicsTemplateName = "Lower Third";
app.endUndoGroup();
writeLn("controls: " + comp.motionGraphicsTemplateControllerCount);
comp.motionGraphicsTemplateName;
"#;

#[test]
fn essential_graphics_template_via_script_and_commands() {
    let dir = std::env::temp_dir().join(format!("ec-qa-template-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("build.jsx"), BUILD).unwrap();

    let r = ok(&dir, &["script", "build.jsx", "--save-as", "t.ecproj"]);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["output"], "controls: 3");
    let list = ok(&dir, &["exec", "essential.list", r##"{"comp":"Lower Third"}"##, "t.ecproj"]);
    let controls: Vec<(String, String)> =
        list["controls"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["type"].as_str().unwrap().to_string())).collect();
    assert_eq!(
        controls,
        [("Title".into(), "text".into()), ("Bar Color".into(), "color".into()), ("Bar Opacity".into(), "slider".into())],
        "ADBE Fill-0002 is the Fill's Color: {list}"
    );

    // An edit comp with an instance; controls addressed by name.
    let r = ok(
        &dir,
        &[
            "run",
            "t.ecproj",
            "comp.new",
            r##"{"name":"Edit","width":320,"height":90,"frameRate":12,"duration":2}"##,
            "layer.addItem",
            r##"{"item":"Lower Third"}"##,
            "comp.new",
            r##"{"name":"Plain","width":320,"height":90,"frameRate":12,"duration":2}"##,
            "layer.addItem",
            r##"{"item":"Lower Third"}"##,
            "comp.open",
            r##"{"comp":"Edit"}"##,
            "essential.set",
            r##"{"layer":"#1","control":"Title","value":"John Smith"}"##,
            "essential.set",
            r##"{"layer":"#1","control":"Bar Color","value":"#00c080"}"##,
            "essential.set",
            r##"{"layer":"#1","control":"bar opacity","value":50}"##,
            "--save",
        ],
    );
    let inst = ok(&dir, &["exec", "essential.instance", r##"{"comp":"Edit","layer":"#1"}"##, "t.ecproj"]);
    assert!(inst["controls"].as_array().unwrap().iter().all(|c| c["overridden"] == true), "{inst}");
    let (code, e) = cli(&dir, &["exec", "essential.set", r##"{"comp":"Edit","layer":"#1","control":"Subtitle","value":"x"}"##, "t.ecproj"]);
    assert_eq!(code, 1);
    assert!(e["error"].as_str().unwrap().contains("controls: Title, Bar Color, Bar Opacity"), "{e} ({r})");

    // Render both instances and compare.
    ok(&dir, &["render-frame", "t.ecproj", "--comp", "Edit", "--time", "1", "--out", "edit.png"]);
    ok(&dir, &["render-frame", "t.ecproj", "--comp", "Plain", "--time", "1", "--out", "plain.png"]);
    let edit = image::open(dir.join("edit.png")).unwrap().to_rgba8();
    let plain = image::open(dir.join("plain.png")).unwrap().to_rgba8();
    let bar = edit.get_pixel(10, 75).0;
    assert!(bar[0] < 15 && (bar[1] as i32 - 96).abs() < 8 && (bar[2] as i32 - 64).abs() < 8, "50 % of #00c080 over black: {bar:?}");
    let pbar = plain.get_pixel(10, 75).0;
    assert!(pbar[0] > 240 && pbar[1] < 15, "the template's own red: {pbar:?}");
    let diff = |y0: u32, y1: u32| -> u32 {
        (y0..y1).flat_map(|y| (0..320).map(move |x| (x, y))).filter(|(x, y)| edit.get_pixel(*x, *y).0 != plain.get_pixel(*x, *y).0).count() as u32
    };
    assert!(diff(10, 45) > 100, "the title text differs");

    // Export the template and bring it into a fresh project.
    let x = ok(&dir, &["exec", "essential.exportTemplate", r##"{"comp":"Lower Third","path":"lt.ectemplate"}"##, "t.ecproj"]);
    assert_eq!(x["controls"], 3, "{x}");
    let info = ok(&dir, &["exec", "essential.templateInfo", r##"{"path":"lt.ectemplate"}"##, "--empty"]);
    assert_eq!(info["name"], "Lower Third", "{info}");
    let imp = ok(&dir, &["run", "--empty", "essential.importTemplate", r##"{"path":"lt.ectemplate"}"##, "--save-as", "fresh.ecproj"]);
    assert!(imp["saved"].is_string(), "{imp}");
    let list = ok(&dir, &["exec", "essential.list", r##"{"comp":"Lower Third"}"##, "fresh.ecproj"]);
    assert_eq!(list["controls"].as_array().unwrap().len(), 3, "{list}");

    // One-shot rendering applies the JSON values to an instance, preserving the template defaults.
    let original_template = std::fs::read(dir.join("lt.ectemplate")).unwrap();
    std::fs::write(dir.join("values.json"), r##"{"Title":"John Smith","Bar Color":"#00c080","bar opacity":50}"##).unwrap();
    ok(&dir, &["render-frame", "--template", "lt.ectemplate", "--values", "values.json", "--time", "1", "--out", "direct.png"]);
    assert_eq!(image::open(dir.join("direct.png")).unwrap().to_rgba8(), edit);
    ok(&dir, &["frame", "--template", "lt.ectemplate", "--frame", "12", "--out", "defaults.png"]);
    assert_eq!(image::open(dir.join("defaults.png")).unwrap().to_rgba8(), plain);

    let output = dir.join("sequence/direct_[#####].png");
    let rendered = ok(
        &dir,
        &[
            "render",
            "--template",
            "lt.ectemplate",
            "--values",
            "values.json",
            "--out",
            output.to_str().unwrap(),
            "--format",
            "png",
            "--channels",
            "rgb",
            "--start",
            "1",
            "--end",
            "1.0833333333333333",
        ],
    );
    assert_eq!(rendered["rendered"].as_array().unwrap().len(), 1, "{rendered}");
    let frames: Vec<_> = std::fs::read_dir(dir.join("sequence"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .collect();
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(image::open(&frames[0]).unwrap().to_rgba8(), edit);
    assert_eq!(std::fs::read(dir.join("lt.ectemplate")).unwrap(), original_template);

    // Invalid values fail before the queue creates an output directory, or frame rendering
    // overwrites an existing output. A JSON object and existing control names are required.
    for (name, values, expected) in [
        ("malformed", "{", "invalid JSON"),
        ("array", "[]", "JSON object"),
        ("unknown", r#"{"Subtitle":"x"}"#, "controls: Title, Bar Color, Bar Opacity"),
        ("invalid-value", r#"{"Bar Opacity":"bad"}"#, "can't use"),
    ] {
        let values_file = format!("{name}.json");
        std::fs::write(dir.join(&values_file), values).unwrap();
        let output_dir = dir.join(format!("failed-{name}"));
        let out = output_dir.join("frame_[#####].png");
        let (code, error) = cli(&dir, &["render", "--template", "lt.ectemplate", "--values", &values_file, "--out", out.to_str().unwrap()]);
        assert_eq!(code, 1, "{name}: {error}");
        assert!(error["error"].as_str().unwrap().contains(expected), "{name}: {error}");
        assert!(!output_dir.exists(), "{name}: invalid input created render output");
        std::fs::write(dir.join("untouched.png"), b"existing output").unwrap();
        let (code, error) = cli(&dir, &["render-frame", "--template", "lt.ectemplate", "--values", &values_file, "--out", "untouched.png"]);
        assert_eq!(code, 1, "{name}: {error}");
        assert_eq!(std::fs::read(dir.join("untouched.png")).unwrap(), b"existing output");
    }
    let (code, error) = cli(&dir, &["render-frame", "--template", "lt.ectemplate", "--values", "missing-values.json", "--out", "missing.png"]);
    assert_eq!(code, 1, "{error}");
    assert!(error["error"].as_str().unwrap().contains("cannot read values"), "{error}");
    assert!(!dir.join("missing.png").exists());
    std::fs::write(dir.join("large-values.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    let (code, error) = cli(&dir, &["render-frame", "--template", "lt.ectemplate", "--values", "large-values.json", "--out", "large.png"]);
    assert_eq!(code, 1, "{error}");
    assert!(error["error"].as_str().unwrap().contains("exceeds 1 MiB"), "{error}");
    assert!(!dir.join("large.png").exists());

    for args in [
        vec!["render-frame", "--values", "values.json"],
        vec!["info", "--template", "lt.ectemplate"],
        vec!["render-frame", "--template", "lt.ectemplate", "--comp", "Lower Third"],
        vec!["render-frame", "t.ecproj", "--template", "lt.ectemplate"],
        vec!["render", "--template", "lt.ectemplate", "--queue"],
        vec!["render-frame", "--template", "lt.ectemplate", "--bridge", "1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_effectcraft-cli")).current_dir(&dir).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    // File-loaded templates can use exact rational rates that comp.new's float parser rounds.
    // Preserve that rate, Tick duration, work area, background and locked source layers.
    let mut project = effectcraft_engine::project::Project::from_json(&std::fs::read_to_string(dir.join("t.ecproj")).unwrap()).unwrap();
    let source = project.find_by_name("Lower Third").unwrap().id;
    let rate = effectcraft_time::FrameRate::new(24001, 1001);
    let comp = project.comp_mut(source).unwrap();
    comp.frame_rate = rate;
    comp.duration = rate.tick_of(24);
    comp.work_area = (rate.tick_of(3), rate.tick_of(7));
    comp.background = [0.08, 0.16, 0.24];
    comp.layers.first_mut().unwrap().switches.locked = true;
    std::fs::write(dir.join("rational.ecproj"), project.to_json()).unwrap();
    ok(&dir, &["exec", "essential.exportTemplate", r#"{"comp":"Lower Third","path":"rational.ectemplate"}"#, "rational.ecproj"]);
    let rational_template = std::fs::read(dir.join("rational.ectemplate")).unwrap();
    let rational = ok(&dir, &["render-frame", "--template", "rational.ectemplate", "--frame", "5", "--out", "rational.png"]);
    assert!((rational["time"].as_f64().unwrap() - 5.0 / rate.as_f64()).abs() < 1e-9, "{rational}");
    assert_eq!((rational["width"].as_u64().unwrap(), rational["height"].as_u64().unwrap()), (320, 90));
    let output = dir.join("rational-work-area/frame_[#####].png");
    ok(&dir, &["render", "--template", "rational.ectemplate", "--work-area", "--out", output.to_str().unwrap(), "--format", "png", "--resolution", "0.05"]);
    let count = std::fs::read_dir(dir.join("rational-work-area"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "png"))
        .count();
    assert_eq!(count, 4, "the template's exact work area contains frames 3 through 6");
    assert_eq!(std::fs::read(dir.join("rational.ectemplate")).unwrap(), rational_template);
    assert_eq!(std::fs::read(dir.join("lt.ectemplate")).unwrap(), original_template);
    let _ = std::fs::remove_dir_all(&dir);
}
