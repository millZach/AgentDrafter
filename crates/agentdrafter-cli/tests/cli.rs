//! The command prints one line of JSON on success and on failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `agentdrafter` and returns its exit code and the JSON it printed.
fn agentdrafter(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.stderr.is_empty(),
        "nothing on standard error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    let json: Value = serde_json::from_str(&stdout).unwrap();
    (out.status.code().unwrap(), json)
}

fn copy_example(dir: &Path) {
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/one-line.yaml"),
        dir.join("one-line.yaml"),
    )
    .unwrap();
}

#[test]
fn build_prints_json_on_success() {
    let dir = scratch("success");
    copy_example(&dir);

    let (code, json) = agentdrafter(&["build", "one-line.yaml"], &dir);

    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let result = &json["result"];
    assert_eq!(result["description"], "one-line.yaml");
    assert_eq!(result["dwg"], "one-line.dwg");
    assert_eq!(result["dwg_version"], "AC1032");
    assert_eq!(result["units"], "mm");
    assert_eq!(result["objects"]["line"], 1);
    let bytes = fs::read(dir.join("one-line.dwg")).unwrap();
    assert_eq!(result["bytes"], bytes.len());
    assert!(json.get("error").is_none());
}

#[test]
fn build_writes_to_the_output_path_and_replaces_only_when_asked() {
    let dir = scratch("output");
    copy_example(&dir);

    let (code, json) = agentdrafter(&["build", "one-line.yaml", "--output", "a.dwg"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["dwg"], "a.dwg");

    let (code, json) = agentdrafter(&["build", "one-line.yaml", "-o", "a.dwg"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "output_exists");
    assert_eq!(json["error"]["file"], "a.dwg");

    let (code, json) = agentdrafter(
        &["build", "--overwrite", "one-line.yaml", "-o", "a.dwg"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
}

#[test]
fn build_prints_json_on_failure_naming_the_field() {
    let dir = scratch("failure");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nobjects:\n  - line:\n      from: [0, 0]\n      to: [1, 1, 0]\n",
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    assert_eq!(json["ok"], false);
    assert!(json.get("result").is_none());
    let error = &json["error"];
    assert_eq!(error["code"], "wrong_type");
    assert_eq!(error["field"], "objects[0].line.from");
    assert_eq!(error["line"], 4);
    assert_eq!(error["column"], 13);
    assert_eq!(error["file"], "sheet.yaml");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("three coordinates"));
    assert!(!dir.join("sheet.dwg").exists());
}

#[test]
fn a_missing_unit_is_reported_as_json() {
    let dir = scratch("missing-unit");
    fs::write(dir.join("sheet.yaml"), "objects: []\n").unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "missing_unit");
    assert_eq!(json["error"]["field"], "units");
}

#[test]
fn a_missing_file_is_reported_as_json() {
    let dir = scratch("missing-file");
    let (code, json) = agentdrafter(&["build", "absent.yaml"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "read_failed");
    assert_eq!(json["error"]["file"], "absent.yaml");
}

#[test]
fn wrong_arguments_are_reported_as_json() {
    let dir = scratch("usage");
    for args in [
        &[][..],
        &["draw"][..],
        &["build"][..],
        &["build", "a.yaml", "b.yaml"][..],
        &["build", "a.yaml", "--colour"][..],
        &["build", "a.yaml", "--output"][..],
    ] {
        let (code, json) = agentdrafter(args, &dir);
        assert_eq!(code, 2, "{args:?}: {json}");
        assert_eq!(json["ok"], false, "{args:?}");
        assert_eq!(json["error"]["code"], "usage", "{args:?}");
    }
}

#[test]
fn help_and_version_are_json() {
    let dir = scratch("help");
    let (code, json) = agentdrafter(&["--help"], &dir);
    assert_eq!(code, 0);
    assert_eq!(json["ok"], true);
    assert!(json["result"]["commands"]["build"]["usage"]
        .as_str()
        .unwrap()
        .starts_with("agentdrafter build"));

    let (code, json) = agentdrafter(&["--version"], &dir);
    assert_eq!(code, 0);
    assert_eq!(json["result"]["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn schema_prints_the_sheet_descriptions_json_schema() {
    let dir = scratch("schema");
    let (code, json) = agentdrafter(&["schema"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let schema = &json["result"];
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["required"], serde_json::json!(["units", "objects"]));
    assert_eq!(
        schema["properties"]["units"]["enum"],
        serde_json::json!(["mm", "cm", "m", "in", "ft"])
    );
    for object_type in ["line", "polyline", "arc", "circle", "ellipse", "spline"] {
        assert!(
            schema["$defs"][object_type]["properties"]["layer"].is_object(),
            "{object_type} takes a layer"
        );
    }
    assert!(schema["$defs"]["layer"]["properties"]["lineweight"].is_object());
    assert!(schema["$defs"]["linetype"]["properties"]["pattern"].is_object());

    let (code, json) = agentdrafter(&["schema", "extra"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");

    let (_, json) = agentdrafter(&["--help"], &dir);
    assert_eq!(
        json["result"]["commands"]["schema"]["usage"],
        "agentdrafter schema"
    );
}

#[test]
fn the_geometry_example_builds_every_object_type() {
    let dir = scratch("geometry");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/geometry.yaml"),
        dir.join("geometry.yaml"),
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "geometry.yaml"], &dir);

    assert_eq!(code, 0, "{json}");
    let objects = &json["result"]["objects"];
    for object_type in ["line", "polyline", "arc", "circle", "ellipse", "spline"] {
        assert!(
            objects[object_type].as_u64().unwrap_or(0) > 0,
            "{object_type} in {objects}"
        );
    }
}

#[test]
fn an_unknown_layer_is_reported_with_the_choices() {
    let dir = scratch("unknown-layer");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nlayers: {outline: {}}\nobjects:\n  - circle: {center: [0, 0, 0], radius: 5, layer: hidden}\n",
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    let error = &json["error"];
    assert_eq!(error["code"], "unknown_name");
    assert_eq!(error["field"], "objects[0].circle.layer");
    assert_eq!(
        error["message"],
        "unknown layer `hidden`; the sheet's layers: 0, outline"
    );
    assert!(!dir.join("sheet.dwg").exists());
}
