//! `agentdrafter render` and `agentdrafter stale` print one line of JSON.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-render")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn agentdrafter(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args(args)
        .current_dir(cwd)
        .env_remove("AGENTDRAFTER_FONTS")
        .env_remove("AGENTDRAFTER_PLOT_STYLES")
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

#[test]
fn render_writes_a_stamped_png_and_stale_checks_it() {
    let dir = scratch("render");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/text.yaml"),
        dir.join("text.yaml"),
    )
    .unwrap();

    let (code, json) = agentdrafter(
        &["render", "text.yaml", "--dpi", "100", "--scale", "1"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    let result = &json["result"];
    assert_eq!(result["png"], "text.png");
    assert_eq!(result["dpi"], 100.0);
    assert_eq!(result["scale"], 1.0);
    assert_eq!(result["objects"]["mtext"], 10);
    assert_eq!(result["stamp"]["sources"][0]["path"], "text.yaml");
    // No font folder: every style's font is replaced by a look-alike, and said so.
    assert_eq!(result["font_substitutions"].as_array().unwrap().len(), 3);
    assert_eq!(
        result["bytes"],
        fs::read(dir.join("text.png")).unwrap().len()
    );

    let (code, json) = agentdrafter(&["stale", "text.png"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["stale"], false);

    fs::write(dir.join("text.yaml"), "units: mm\nobjects: []\n").unwrap();
    let (code, json) = agentdrafter(&["stale", "text.png"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["stale"], true);

    let (code, json) = agentdrafter(&["render", "text.yaml"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "output_exists");
    let (code, json) = agentdrafter(
        &["render", "text.yaml", "--overwrite", "--width", "300"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["width"], 300);
}

#[test]
fn render_draws_through_a_plot_style_from_the_folders_it_is_given_or_monochrome() {
    let dir = scratch("plot-style");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 50, 0], color: 1}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("styles")).unwrap();
    // The folder --plot-styles names is where the plot style is looked for.
    let (code, json) = agentdrafter(
        &[
            "render",
            "sheet.yaml",
            "--plot-style",
            "nowhere.ctb",
            "--plot-styles",
            "styles",
        ],
        &dir,
    );
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "read_failed");
    assert_eq!(json["error"]["field"], "plot_style");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("none of the plot style folders: styles"),
        "{message}"
    );
    assert!(!dir.join("sheet.png").exists());

    let (code, json) = agentdrafter(&["render", "sheet.yaml", "--monochrome"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["drawn_through"], "monochrome");
    let (code, json) = agentdrafter(&["render", "sheet.yaml", "--overwrite"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["drawn_through"], "colour");
}

#[test]
fn render_and_stale_refuse_wrong_arguments() {
    let dir = scratch("usage");
    for args in [
        &["render"][..],
        &["render", "a.yaml", "--dpi"],
        &["render", "a.yaml", "--dpi", "many"],
        &["render", "a.yaml", "--colour"],
        &["render", "a.yaml", "--plot-style"],
        &[
            "render",
            "a.yaml",
            "--plot-style",
            "a.ctb",
            "--plot-style",
            "b.ctb",
        ],
        &["render", "a.yaml", "--monochrome", "--plot-style", "a.ctb"],
        &["render", "a.yaml", "b.yaml"],
        &["stale"],
        &["stale", "a.png", "--source"],
    ] {
        let (code, json) = agentdrafter(args, &dir);
        assert_eq!(code, 2, "{args:?}: {json}");
        assert_eq!(json["error"]["code"], "usage", "{args:?}");
    }
    let (code, json) = agentdrafter(&["render", "a.yaml", "--scale", "1", "--width", "10"], &dir);
    assert_eq!(code, 2, "{json}");
}
