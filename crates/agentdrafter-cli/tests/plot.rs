//! `agentdrafter plot` prints one line of JSON, and `agentdrafter stale`
//! checks the PDF it wrote.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-plot")
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
fn plot_writes_a_stamped_pdf_of_each_sheet_and_stale_checks_it() {
    let dir = scratch("set");
    demo_set::copy_demo_set(&dir.join("demo-set"));

    let (code, json) = agentdrafter(&["plot", "demo-set"], &dir);
    assert_eq!(code, 0, "{json}");
    let result = &json["result"];
    assert_eq!(result["plotted"], 3);
    let first = &result["sheets"][0]["result"];
    assert_eq!(first["sheet_size"]["width_mm"], 431.8);
    assert_eq!(first["sheet_size"]["height_mm"], 279.4);
    assert_eq!(first["plot_style"]["name"], "monochrome (built in)");
    assert_eq!(first["stamp"]["sources"][0]["kind"], "template");
    let pdf = dir.join("demo-set").join("dwg").join("DEMO-001.pdf");
    assert!(fs::read(&pdf).unwrap().starts_with(b"%PDF-"));

    let (code, json) = agentdrafter(&["stale", "demo-set/dwg/DEMO-001.pdf"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["stale"], false, "{json}");

    // Plotting again without --overwrite leaves each PDF as it was.
    let (code, json) = agentdrafter(&["plot", "demo-set"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "sheets_failed");
    assert_eq!(
        json["result"]["sheets"][0]["error"]["code"],
        "output_exists"
    );
    let (code, json) = agentdrafter(&["plot", "demo-set", "--overwrite"], &dir);
    assert_eq!(code, 0, "{json}");
}

#[test]
fn plot_takes_a_plot_style_from_the_folders_it_is_given() {
    let dir = scratch("sheet");
    fs::write(
        dir.join("sheet.yaml"),
        "units: in\nobjects:\n  - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true}\n",
    )
    .unwrap();
    let (code, json) = agentdrafter(&["plot", "sheet.yaml", "--plot-style", "nowhere.ctb"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "read_failed");
    assert_eq!(json["error"]["field"], "plot_style");

    let (code, json) = agentdrafter(&["plot", "sheet.yaml", "--output", "out.pdf"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["pdf"], "out.pdf");
    assert_eq!(json["result"]["sheet_size"]["width_points"], 1224.0);

    let (code, json) = agentdrafter(&["plot"], &dir);
    assert_eq!(code, 2, "{json}");
    let (code, json) = agentdrafter(&["plot", ".", "--output", "x.pdf"], &dir);
    assert_eq!(code, 2, "{json}");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("an output path (--output) is for one sheet"));
}

#[test]
fn a_plot_style_name_of_any_characters_gets_a_json_reply() {
    let dir = scratch("plot-style-name");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 50, 0]}\n",
    )
    .unwrap();
    // Its last four bytes start inside the `é`.
    let (code, json) = agentdrafter(&["plot", "sheet.yaml", "--plot-style", "éaaa"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "bad_value");
    assert_eq!(json["error"]["field"], "plot_style");
    assert!(!dir.join("sheet.pdf").exists());
}
