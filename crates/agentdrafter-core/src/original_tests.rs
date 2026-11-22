//! Reading an original: made-up drawings only, built by AgentDrafter and
//! made to look like someone else's (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::build::{build, BuildRequest};
use crate::error::ErrorCode;

/// An empty folder for one test under the system's temporary folder.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-original-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up original in `dir`, `P-100.dwg`: a line, a circle and a
/// polyline on a layer, built by AgentDrafter, then stripped of its build
/// mark and of every ID AgentDrafter wrote, as someone else's drawing has
/// none. Also gives the DWG as built, with its IDs.
fn made_up_original(dir: &Path) -> (PathBuf, PathBuf) {
    let description = dir.join("built.yaml");
    fs::write(
        &description,
        "units: mm\n\
         layers:\n  E-WIRE: {color: 1}\n\
         objects:\n\
         \x20 - line: {from: [0, 0, 0], to: [100, 0, 0], layer: E-WIRE}\n\
         \x20 - circle: {center: [50, 50, 0], radius: 10}\n\
         \x20 - polyline: {points: [[0, 0, 0], [10, 10, 0], [20, 0, 0]]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let built = dir.join("built.dwg");
    let original = dir.join("P-100.dwg");
    dwg::as_someone_elses(&built, &original);
    (original, built)
}

fn request(source: &Path) -> ReadRequest {
    ReadRequest {
        source: source.to_path_buf(),
        mirror: None,
        no_mirror: false,
    }
}

/// Whether `text` reads as YAML, to its end.
fn reads_as_yaml(text: &str) -> Result<(), String> {
    for event in saphyr_parser::Parser::new_from_str(text) {
        event.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[test]
fn an_original_is_read_whole_never_written_and_mirrored_beside_it() {
    let dir = scratch("whole");
    let (original, _) = made_up_original(&dir);
    let before = fs::read(&original).unwrap();
    let report = read_original(&request(&original)).unwrap();
    assert_eq!(
        fs::read(&original).unwrap(),
        before,
        "the original is never written"
    );

    assert_eq!(report.sheet, "P-100");
    assert_eq!(report.dwg_version, "AC1032");
    assert_eq!(report.units, Some(Unit::Mm));
    assert!(report.decoded, "{report:?}");
    assert!(report.not_decoded.is_empty());
    assert!(report.not_read.is_empty());
    assert_eq!(
        report.objects,
        BTreeMap::from([
            ("circle".to_owned(), 1),
            ("line".to_owned(), 1),
            ("polyline".to_owned(), 1)
        ])
    );
    let mirror = dir.join("P-100.mirror.yaml");
    assert_eq!(
        report.mirror.as_deref(),
        Some(mirror.display().to_string().as_str())
    );
    assert!(
        report.message.contains("decoded every record") && report.message.contains("3 objects"),
        "{}",
        report.message
    );

    let text = fs::read_to_string(&mirror).unwrap();
    assert!(text.starts_with(MIRROR_MARK), "{text}");
    reads_as_yaml(&text).unwrap();
    assert!(text.contains("\nmirror_of: P-100.dwg\n"), "{text}");
    assert!(
        text.contains(&format!("\nsha256: {}\n", fingerprint(&before))),
        "{text}"
    );
    assert!(text.contains("\ndecoded: true\n"), "{text}");
    assert!(text.contains("\n  - {name: E-WIRE, "), "{text}");
    // Each object on a line of its own, with its ID from its handle.
    let objects: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "objects:")
        .skip(1)
        .collect();
    assert_eq!(objects.len(), 3, "{text}");
    let drawing = dwg::read(&original).unwrap();
    for (line, handle) in objects.iter().zip(&drawing.handles) {
        assert!(
            line.starts_with(&format!(
                "  - {{id: P-100/handle:{handle}, handle: \"{handle}\", type: "
            )),
            "{line}"
        );
    }
    assert!(objects[0].contains("type: line, "), "{}", objects[0]);
    assert!(objects[0].contains("layer: E-WIRE"), "{}", objects[0]);

    // Read again, the same file gives the same IDs and the same mirror.
    let again = read_original(&request(&original)).unwrap();
    assert_eq!(again, report);
    assert_eq!(fs::read_to_string(&mirror).unwrap(), text);
}

#[test]
fn an_object_takes_the_id_agentdrafter_wrote_on_it_and_otherwise_its_handle() {
    let dir = scratch("ids");
    let (original, built) = made_up_original(&dir);
    let from_build = dwg::read(&built).unwrap();
    assert_eq!(
        ids_of(&built, &from_build)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["built/objects[0]", "built/objects[1]", "built/objects[2]"]
    );
    let from_original = dwg::read(&original).unwrap();
    let ids: Vec<String> = ids_of(&original, &from_original)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let by_handle: Vec<String> = from_original
        .handles
        .iter()
        .map(|h| format!("P-100/handle:{h}"))
        .collect();
    assert_eq!(ids, by_handle);
    // The handles are the drawing's own, the same as built.
    assert_eq!(from_original.handles, from_build.handles);
}

#[test]
fn a_record_the_dwg_library_cannot_decode_is_named_in_the_reply_and_the_mirror() {
    let dir = scratch("not-decoded");
    let (original, _) = made_up_original(&dir);
    let custom = dir.join("P-101.dwg");
    let handle = dwg::with_a_made_up_custom_object(&original, &custom, "MADE_UP_DEVICE");
    let report = read_original(&request(&custom)).unwrap();
    assert!(!report.decoded);
    let expected = vec![RecordsOfKind {
        kind: "MADE_UP_DEVICE".into(),
        count: 1,
        handles: vec![handle.clone()],
    }];
    assert_eq!(report.not_decoded, expected);
    assert_eq!(report.not_read, expected, "nothing draws it");
    assert!(
        report.message.contains("could not decode 1 MADE_UP_DEVICE")
            && report.message.contains("1 the core has no model for"),
        "{}",
        report.message
    );
    let text = fs::read_to_string(dir.join("P-101.mirror.yaml")).unwrap();
    reads_as_yaml(&text).unwrap();
    assert!(text.contains("\ndecoded: false\n"), "{text}");
    assert!(
        text.contains(&format!(
            "\nnot_decoded: [{{kind: MADE_UP_DEVICE, count: 1, handles: [\"{handle}\"]}}]\n"
        )),
        "{text}"
    );
}

#[test]
fn a_mirror_replaces_only_a_mirror_and_never_the_original() {
    let dir = scratch("replace");
    let (original, _) = made_up_original(&dir);
    let before = fs::read(&original).unwrap();

    let someone_elses = dir.join("P-100.mirror.yaml");
    fs::write(&someone_elses, "my own notes\n").unwrap();
    let error = read_original(&request(&original)).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput, "{error:?}");
    assert_eq!(
        fs::read_to_string(&someone_elses).unwrap(),
        "my own notes\n"
    );

    let onto_itself = ReadRequest {
        mirror: Some(original.clone()),
        ..request(&original)
    };
    let error = read_original(&onto_itself).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
    assert_eq!(fs::read(&original).unwrap(), before);

    let elsewhere = dir.join("elsewhere.yaml");
    let at = ReadRequest {
        mirror: Some(elsewhere.clone()),
        ..request(&original)
    };
    read_original(&at).unwrap();
    // Its own mirror it replaces.
    read_original(&at).unwrap();
    assert!(fs::read_to_string(&elsewhere)
        .unwrap()
        .starts_with(MIRROR_MARK));
    assert_eq!(fs::read(&original).unwrap(), before);
}

#[test]
fn with_no_mirror_a_read_writes_nothing() {
    let dir = scratch("no-mirror");
    let (original, _) = made_up_original(&dir);
    let listed = |dir: &Path| {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = listed(&dir);
    let report = read_original(&ReadRequest {
        no_mirror: true,
        ..request(&original)
    })
    .unwrap();
    assert_eq!(report.mirror, None);
    assert_eq!(listed(&dir), before);

    let both = ReadRequest {
        no_mirror: true,
        mirror: Some(dir.join("m.yaml")),
        ..request(&original)
    };
    assert_eq!(read_original(&both).unwrap_err().code, ErrorCode::Usage);
}

#[test]
fn only_a_dwg_is_read_as_an_original() {
    let dir = scratch("not-a-dwg");
    let description = dir.join("sheet.yaml");
    fs::write(&description, "units: mm\nobjects: []\n").unwrap();
    let error = read_original(&request(&description)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
}

#[test]
fn a_string_is_written_plain_only_where_yaml_reads_it_back_the_same() {
    assert_eq!(scalar("E-WIRE"), "E-WIRE");
    assert_eq!(scalar("P-100/handle:2F"), "P-100/handle:2F");
    for quoted in [
        "", "2F", "0", "true", "No", "null", "a b", "x: y", "#1", "-1", "é",
    ] {
        assert!(scalar(quoted).starts_with('"'), "{quoted}");
        reads_as_yaml(&format!("k: {}\n", scalar(quoted))).unwrap();
    }
}
