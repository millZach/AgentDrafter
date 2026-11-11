//! Text through the command: building text, the font folders and measuring.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-text")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `agentdrafter` with no font folders from the environment and
/// returns its exit code and the JSON it printed.
fn agentdrafter(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args(args)
        .current_dir(cwd)
        .env_remove("AGENTDRAFTER_FONTS")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

#[track_caller]
fn close(value: &Value, expected: f64) {
    let number = value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is a number"));
    assert!(
        (number - expected).abs() < 1e-9,
        "{number} is not {expected}"
    );
}

#[test]
fn the_text_example_builds_and_reports_its_look_alikes() {
    let dir = scratch("example");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/text.yaml"),
        dir.join("text.yaml"),
    )
    .unwrap();
    fs::create_dir(dir.join("fonts")).unwrap();

    let (code, json) = agentdrafter(&["build", "text.yaml", "--fonts", "fonts"], &dir);

    assert_eq!(code, 0, "{json}");
    let result = &json["result"];
    assert_eq!(result["objects"]["text"], 14);
    assert_eq!(result["objects"]["mtext"], 10);
    let substitutions = result["font_substitutions"].as_array().unwrap();
    let fonts: Vec<&str> = substitutions
        .iter()
        .map(|s| s["font"].as_str().unwrap())
        .collect();
    assert_eq!(fonts, ["txt.shx", "romans.shx", "arial.ttf"]);
    for s in substitutions {
        assert_eq!(s["used"], "Hershey Roman Simplex", "{s}");
        assert_eq!(s["why"], "it is not in the font folders", "{s}");
    }
    assert!(dir.join("text.dwg").exists());
}

#[test]
fn measure_text_prints_the_measurement_and_the_font_used() {
    let dir = scratch("measure");
    let (code, json) = agentdrafter(
        &[
            "measure-text",
            "HI",
            "--font",
            "romans.shx",
            "--height",
            "21",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    let result = &json["result"];
    // In the look-alike, H strokes from 4 and I at 22 + 4, 21 units being the height.
    close(&result["left"], 4.0);
    close(&result["right"], 26.0);
    close(&result["width"], 22.0);
    assert_eq!(result["font"]["requested"], "romans.shx");
    assert_eq!(result["font"]["used"], "Hershey Roman Simplex");
    assert_eq!(result["font"]["substituted"], true);
    assert_eq!(result["font"]["why"], "no font folder is set");

    let (code, json) = agentdrafter(
        &[
            "measure-text",
            "HI",
            "--font",
            "romans.shx",
            "--height",
            "21",
            "--width-factor",
            "0.5",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    close(&json["result"]["width"], 11.0);
}

#[test]
fn a_measurement_that_cannot_be_made_is_reported_as_json() {
    let dir = scratch("measure-errors");
    let (code, json) = agentdrafter(
        &["measure-text", "A", "--font", "arial.otf", "--height", "1"],
        &dir,
    );
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "bad_value");
    assert_eq!(json["error"]["field"], "font");

    for args in [
        &["measure-text"][..],
        &["measure-text", "A", "--font", "a.shx"][..],
        &["measure-text", "A", "--height", "1"][..],
        &["measure-text", "A", "B", "--font", "a.shx", "--height", "1"][..],
        &["measure-text", "A", "--font", "a.shx", "--height", "tall"][..],
        &[
            "measure-text",
            "A",
            "--font",
            "a.shx",
            "--height",
            "1",
            "--slant",
            "5",
        ][..],
        &["build", "a.yaml", "--fonts"][..],
    ] {
        let (code, json) = agentdrafter(args, &dir);
        assert_eq!(code, 2, "{args:?}: {json}");
        assert_eq!(json["error"]["code"], "usage", "{args:?}");
    }
}

#[test]
fn help_names_measure_text_and_the_font_folders() {
    let dir = scratch("help");
    let (_, json) = agentdrafter(&["--help"], &dir);
    let usage = json["result"]["commands"]["measure-text"]["usage"]
        .as_str()
        .unwrap();
    assert!(usage.starts_with("agentdrafter measure-text"), "{usage}");
    assert!(json["result"]["fonts"]
        .as_str()
        .unwrap()
        .contains("AGENTDRAFTER_FONTS"));
}
