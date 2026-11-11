//! A sheet description builds to a DWG, and the DWG reads back the same.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{build, read_dwg, BuildRequest, ErrorCode, Line, Object, Point3, Unit};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-build")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn request(description: &Path) -> BuildRequest {
    BuildRequest {
        description: description.to_owned(),
        output: None,
        overwrite: false,
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_one_line_example_builds_to_a_2018_dwg_that_reads_back_the_same_line() {
    let dir = scratch("one-line-example");
    let description = dir.join("one-line.yaml");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/one-line.yaml"),
        &description,
    )
    .unwrap();

    let report = build(&request(&description)).unwrap();

    let dwg = dir.join("one-line.dwg");
    assert_eq!(report.dwg, dwg.display().to_string());
    assert_eq!(report.dwg_version, "AC1032");
    assert_eq!(report.units, Unit::Mm);
    assert_eq!(report.objects.get("line"), Some(&1));
    let bytes = fs::read(&dwg).unwrap();
    assert_eq!(report.bytes, bytes.len());
    assert_eq!(
        &bytes[..6],
        b"AC1032",
        "the file starts with the 2018 version string"
    );

    let read = read_dwg(&dwg).unwrap();
    assert_eq!(read.dwg_version, "AC1032");
    assert_eq!(read.units, Some(Unit::Mm));
    assert_eq!(
        read.objects,
        vec![Object::Line(Line {
            from: Point3::new(0.0, 0.0, 0.0),
            to: Point3::new(100.0, 50.0, 0.0),
            layer: "0".to_owned(),
        })]
    );
    assert_eq!(read.objects_not_read, 0);
    assert_eq!(files_in(&dir), ["one-line.dwg", "one-line.yaml"]);
}

#[test]
fn a_3d_line_keeps_every_coordinate_exactly() {
    let dir = scratch("three-d");
    let description = dir.join("three-d.yaml");
    fs::write(
        &description,
        "units: in\nobjects:\n  - line: {from: [12.5, -3.25, 7], to: [0.1, 1e-3, -2.5]}\n",
    )
    .unwrap();
    let output = dir.join("out").join("custom name.dwg");
    fs::create_dir_all(output.parent().unwrap()).unwrap();

    let report = build(&BuildRequest {
        output: Some(output.clone()),
        ..request(&description)
    })
    .unwrap();

    assert_eq!(report.dwg, output.display().to_string());
    let read = read_dwg(&output).unwrap();
    assert_eq!(read.units, Some(Unit::In));
    let [Object::Line(line)] = read.objects.as_slice() else {
        panic!("expected one line, read {:?}", read.objects);
    };
    let bits = |p: Point3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    assert_eq!(bits(line.from), bits(Point3::new(12.5, -3.25, 7.0)));
    assert_eq!(bits(line.to), bits(Point3::new(0.1, 1e-3, -2.5)));
}

#[test]
fn every_unit_reaches_the_dwg() {
    let dir = scratch("units");
    for unit in Unit::ALL {
        let description = dir.join(format!("{unit}.yaml"));
        fs::write(&description, format!("units: {unit}\nobjects: []\n")).unwrap();
        build(&request(&description)).unwrap();
        let read = read_dwg(&dir.join(format!("{unit}.dwg"))).unwrap();
        assert_eq!(read.units, Some(unit));
        assert!(read.objects.is_empty());
    }
}

#[test]
fn an_existing_file_is_never_replaced_unless_asked() {
    let dir = scratch("existing");
    let description = dir.join("sheet.yaml");
    fs::write(&description, "units: mm\nobjects: []\n").unwrap();
    let dwg = dir.join("sheet.dwg");
    fs::write(&dwg, b"not ours").unwrap();

    let error = build(&request(&description)).unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputExists);
    assert_eq!(error.file, Some(dwg.display().to_string()));
    assert_eq!(fs::read(&dwg).unwrap(), b"not ours");
}

#[test]
fn overwrite_replaces_only_a_dwg_agentdrafter_built() {
    let dir = scratch("overwrite");
    let description = dir.join("sheet.yaml");
    fs::write(&description, "units: mm\nobjects: []\n").unwrap();
    let dwg = dir.join("sheet.dwg");
    let overwrite = BuildRequest {
        overwrite: true,
        ..request(&description)
    };

    // A file that cannot be read as a DWG cannot show that AgentDrafter built it.
    fs::write(&dwg, b"not ours").unwrap();
    let error = build(&overwrite).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotABuild);
    assert_eq!(error.file, Some(dwg.display().to_string()));
    assert!(
        error.message.contains("cannot be read as a DWG"),
        "{error:?}"
    );
    assert!(error.message.contains("never overwritten"), "{error:?}");
    assert_eq!(fs::read(&dwg).unwrap(), b"not ours");
    assert_eq!(files_in(&dir), ["sheet.dwg", "sheet.yaml"]);

    // A DWG this build wrote is replaced.
    fs::remove_file(&dwg).unwrap();
    build(&request(&description)).unwrap();
    fs::write(&description, "units: in\nobjects: []\n").unwrap();
    build(&overwrite).unwrap();
    assert_eq!(read_dwg(&dwg).unwrap().units, Some(Unit::In));
    assert_eq!(files_in(&dir), ["sheet.dwg", "sheet.yaml"]);
}

#[test]
fn the_sheet_description_is_never_the_output() {
    let dir = scratch("self");
    let description = dir.join("sheet.yaml");
    let text = "units: mm\nobjects: []\n";
    fs::write(&description, text).unwrap();

    let error = build(&BuildRequest {
        output: Some(description.clone()),
        overwrite: true,
        ..request(&description)
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage);
    assert_eq!(fs::read_to_string(&description).unwrap(), text);
}

#[test]
fn a_failed_build_writes_nothing_and_names_the_file_and_field() {
    let dir = scratch("failed");
    let description = dir.join("sheet.yaml");
    fs::write(
        &description,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], too: [1, 1, 0]}\n",
    )
    .unwrap();

    let error = build(&request(&description)).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownKey);
    assert_eq!(error.field.as_deref(), Some("objects[0].line.too"));
    assert_eq!(error.file, Some(description.display().to_string()));
    assert_eq!((error.line, error.column), (Some(3), Some(29)));
    assert_eq!(files_in(&dir), ["sheet.yaml"]);
}

#[test]
fn a_missing_sheet_description_is_a_read_error() {
    let dir = scratch("missing");
    let error = build(&request(&dir.join("absent.yaml"))).unwrap_err();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert!(files_in(&dir).is_empty());
}
