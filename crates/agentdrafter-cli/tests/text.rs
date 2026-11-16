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
        .env("AGENTDRAFTER_SYSTEM_FONTS", "0")
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

/// Measures `font`, in no folder of its own, with the environment's
/// `AGENTDRAFTER_SYSTEM_FONTS` set to `system` or left out, and `extra`
/// options, and gives the reply.
fn measure_with_system(font: &str, system: Option<&str>, extra: &[&str]) -> Value {
    let dir = scratch("system");
    let mut command = Command::new(env!("CARGO_BIN_EXE_agentdrafter"));
    command
        .args(["measure-text", "HI", "--font", font, "--height", "21"])
        .args(extra)
        .current_dir(&dir)
        .env_remove("AGENTDRAFTER_FONTS")
        .env_remove("AGENTDRAFTER_SYSTEM_FONTS");
    if let Some(system) = system {
        command.env("AGENTDRAFTER_SYSTEM_FONTS", system);
    }
    let out = command.output().unwrap();
    assert!(out.status.success(), "{out:?}");
    serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap()
}

#[test]
fn the_system_font_folders_are_searched_for_truetype_unless_left_out() {
    // A made-up font no system has: with the system's folders searched it
    // is not in them; with them left out, no font folder is set at all.
    let font = "agentdrafter-made-up-sans.ttf";
    let searched = measure_with_system(font, None, &[]);
    assert_eq!(
        searched["result"]["font"]["why"], "it is not in the font folders",
        "{searched}"
    );
    for (system, extra) in [
        (None, &["--no-system-fonts"][..]),
        (Some("0"), &[][..]),
        (Some("off"), &[][..]),
    ] {
        let left_out = measure_with_system(font, system, extra);
        assert_eq!(
            left_out["result"]["font"]["why"], "no font folder is set",
            "{system:?} {extra:?}: {left_out}"
        );
        // Either way a look-alike stood in, and the reply warns of it first.
        assert_eq!(
            left_out["warnings"],
            serde_json::json!([{
                "code": "font_look_alike",
                "font": font,
                "look_alike": "Hershey Roman Simplex",
                "sheets": [],
            }])
        );
    }
    // An SHX font is never looked for in the system's folders.
    let shx = measure_with_system("romans.shx", None, &[]);
    assert_eq!(
        shx["result"]["font"]["why"], "no font folder is set",
        "{shx}"
    );
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
