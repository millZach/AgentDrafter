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
        text.contains(&format!("\nsha256: \"{}\"\n", fingerprint(&before))),
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
fn a_mirror_replaces_only_a_mirror_and_never_the_original_symlink_or_folder() {
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

    // A folder at the mirror path is refused.
    let folder = dir.join("a-folder");
    fs::create_dir(&folder).unwrap();
    let onto_folder = ReadRequest {
        mirror: Some(folder.clone()),
        ..request(&original)
    };
    let error = read_original(&onto_folder).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotAnOutput, "{error:?}");
    assert!(folder.is_dir());

    // A symlink at the mirror path is refused, even if it points to a valid
    // mirror or to the original DWG.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let link_to_original = dir.join("link-orig.yaml");
        symlink(&original, &link_to_original).unwrap();
        let err = read_original(&ReadRequest {
            mirror: Some(link_to_original.clone()),
            ..request(&original)
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotAnOutput, "{err:?}");
        assert!(fs::symlink_metadata(&link_to_original)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&original).unwrap(), before);

        let real_mirror = dir.join("real.mirror.yaml");
        read_original(&ReadRequest {
            mirror: Some(real_mirror.clone()),
            ..request(&original)
        })
        .unwrap();
        let mirror_before = fs::read_to_string(&real_mirror).unwrap();
        let link_to_mirror = dir.join("link-mirror.yaml");
        symlink(&real_mirror, &link_to_mirror).unwrap();
        let err = read_original(&ReadRequest {
            mirror: Some(link_to_mirror.clone()),
            ..request(&original)
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotAnOutput, "{err:?}");
        assert!(fs::symlink_metadata(&link_to_mirror)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real_mirror).unwrap(), mirror_before);

        let dangling = dir.join("dangling.yaml");
        symlink(dir.join("nowhere.yaml"), &dangling).unwrap();
        let err = read_original(&ReadRequest {
            mirror: Some(dangling.clone()),
            ..request(&original)
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotAnOutput, "{err:?}");
        assert!(!dir.join("nowhere.yaml").exists());
    }

    let onto_itself = ReadRequest {
        mirror: Some(original.clone()),
        ..request(&original)
    };
    let error = read_original(&onto_itself).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
    assert_eq!(fs::read(&original).unwrap(), before);
    let as_a_dwg = ReadRequest {
        mirror: Some(dir.join("new.dwg")),
        ..request(&original)
    };
    let error = read_original(&as_a_dwg).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
    assert!(!dir.join("new.dwg").exists());

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

    // Files named .dwg that are not DWGs always fail with DwgRead, never 0 objects.
    for (name, bytes) in [
        ("empty.dwg", &b""[..]),
        ("plain.dwg", b"plain text pretending to be a dwg\n"),
        (
            "garbage.dwg",
            &[0xff, 0x00, 0xfe, 0x01, 0xfd, 0x02, 0xfc, 0x03],
        ),
    ] {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        let error = read_original(&ReadRequest {
            no_mirror: true,
            ..request(&path)
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::DwgRead, "{name}: {error:?}");
    }
}

/// Parses a single-key YAML document `k: <value>\n` and returns the scalar string.
fn parse_yaml_scalar(yaml_value: &str) -> String {
    let doc = format!("k: {yaml_value}\n");
    let mut saw_key = false;
    for event in saphyr_parser::Parser::new_from_str(&doc) {
        let (event, _) = event.unwrap_or_else(|e| panic!("YAML error on `{doc}`: {e}"));
        if let saphyr_parser::Event::Scalar(value, style, _, _) = event {
            if !saw_key {
                assert_eq!(value, "k");
                saw_key = true;
            } else {
                assert!(
                    matches!(
                        style,
                        saphyr_parser::ScalarStyle::Plain
                            | saphyr_parser::ScalarStyle::DoubleQuoted
                    ),
                    "{doc}"
                );
                return value.into_owned();
            }
        }
    }
    panic!("no scalar value in `{doc}`")
}

#[test]
fn a_string_is_written_plain_only_where_yaml_reads_it_back_the_same() {
    assert_eq!(scalar("E-WIRE"), "E-WIRE");
    assert_eq!(scalar("P-100/handle:2F"), "P-100/handle:2F");
    assert_eq!(parse_yaml_scalar(&scalar("E-WIRE")), "E-WIRE");
    assert_eq!(
        parse_yaml_scalar(&scalar("P-100/handle:2F")),
        "P-100/handle:2F"
    );
    for tricky in [
        "",
        "2F",
        "0",
        "1E3",
        "true",
        "false",
        "No",
        "yes",
        "on",
        "off",
        "y",
        "n",
        "null",
        "~",
        ".inf",
        ".Inf",
        "-.inf",
        ".nan",
        ".NaN",
        "-",
        ":",
        "a:",
        "a b",
        "x: y",
        "#1",
        "-1",
        "\"quoted\"",
        "'single'",
        "a\nb",
        "a\rb",
        "a\u{2028}b",
        "a\u{2029}b",
        "a\u{0085}b",
        "é",
    ] {
        let out = scalar(tricky);
        assert!(out.starts_with('"'), "{tricky:?} -> {out}");
        assert!(
            !out.contains(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']),
            "single-line YAML scalar for {tricky:?}: {out}"
        );
        assert_eq!(parse_yaml_scalar(&out), tricky, "{tricky:?} -> {out}");
    }
}

#[test]
fn a_filename_with_newlines_or_yaml_punctuation_never_breaks_the_mirror_yaml() {
    let dir = scratch("tricky-name");
    let (original, _) = made_up_original(&dir);
    let bytes = fs::read(&original).unwrap();
    let drawing = dwg::read_bytes(&bytes, &original).unwrap();
    for tricky_name in [
        "bad\ninjected: true.dwg",
        "cr\rname.dwg",
        "sep\u{2028}line.dwg",
        "~special: #1 \"q\".dwg",
    ] {
        let fake_path = dir.join(tricky_name);
        let sheet = sheet_name(&fake_path);
        let text = mirror_text(&fake_path, &bytes, &sheet, &drawing);
        let first_line = text.lines().next().unwrap();
        assert!(first_line.starts_with(MIRROR_MARK), "{first_line}");
        assert!(
            !first_line.contains("injected: true\n"),
            "header comment must stay one line: {text}"
        );
        reads_as_yaml(&text)
            .unwrap_or_else(|e| panic!("YAML broke on {tricky_name:?}: {e}\n{text}"));
        let mirror_of_line = text.lines().find(|l| l.starts_with("mirror_of: ")).unwrap();
        assert_eq!(
            parse_yaml_scalar(mirror_of_line.strip_prefix("mirror_of: ").unwrap()),
            tricky_name
        );
    }
}

#[test]
fn carried_ids_never_collide_with_handle_ids_or_duplicate_each_other() {
    let dir = scratch("id-collisions");
    let (original, _) = made_up_original(&dir);
    let mut drawing = dwg::read(&original).unwrap();
    assert_eq!(drawing.handles.len(), 3);
    let h0 = drawing.handles[0].clone();
    let h1 = drawing.handles[1].clone();
    let h2 = drawing.handles[2].clone();
    // Object 0 carries an ID that literally equals Object 1's handle ID (in
    // different case), and Object 2 duplicates Object 0's or another ID.
    drawing.ids_and_names = vec![
        IdAndName {
            id: Some(format!("p-100/handle:{}", h1.to_lowercase())),
            name: None,
        },
        IdAndName {
            id: Some("DEMO/dup".into()),
            name: None,
        },
        IdAndName {
            id: Some("demo/DUP".into()),
            name: None,
        },
    ];
    let resolved = ids_of(&original, &drawing);
    let ids: Vec<&str> = resolved.iter().map(|(id, _)| id.as_str()).collect();
    // Object 0 could not steal Object 1's handle ID, so it gets its own handle ID.
    assert_eq!(ids[0], format!("P-100/handle:{h0}"));
    // Object 1 keeps its carried ID.
    assert_eq!(ids[1], "DEMO/dup");
    // Object 2 duplicates Object 1 case-insensitively, so it falls back to its handle ID.
    assert_eq!(ids[2], format!("P-100/handle:{h2}"));
    let unique: HashSet<String> = ids.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(unique.len(), 3, "every object gets a unique ID: {ids:?}");
}

#[test]
fn ids_stay_stable_across_path_case_and_agree_with_display_list() {
    use crate::display_list::{display_list_with, DisplayListRequest};
    use crate::settings::Settings;

    let dir = scratch("case-stability");
    let (original, _) = made_up_original(&dir);
    let report = read_original(&ReadRequest {
        no_mirror: true,
        ..request(&original)
    })
    .unwrap();
    let dl = display_list_with(&DisplayListRequest::new(&original), &Settings::default()).unwrap();
    assert_eq!(dl.sheet, report.sheet);
    let expected_ids: Vec<String> = dl.objects.iter().filter_map(|o| o.id.clone()).collect();
    assert_eq!(expected_ids.len(), 3);

    // Uppercase extension (.DWG) yields the same sheet name and IDs.
    let upper_ext = dir.join("P-100.DWG");
    if !upper_ext.exists() {
        fs::copy(&original, &upper_ext).unwrap();
    }
    let upper_report = read_original(&ReadRequest {
        no_mirror: true,
        ..request(&upper_ext)
    })
    .unwrap();
    let upper_dl =
        display_list_with(&DisplayListRequest::new(&upper_ext), &Settings::default()).unwrap();
    assert_eq!(upper_report.sheet, "P-100");
    assert_eq!(upper_dl.sheet, "P-100");
    assert_eq!(
        upper_dl
            .objects
            .iter()
            .filter_map(|o| o.id.clone())
            .collect::<Vec<_>>(),
        expected_ids
    );

    // On Unix, a differently-cased symlink to the same file resolves to the
    // on-disk filename, giving identical sheet name and IDs in both commands.
    #[cfg(unix)]
    {
        let lower_alias = dir.join("p-100.dwg");
        if !lower_alias.exists() {
            std::os::unix::fs::symlink(&original, &lower_alias).unwrap();
        }
        let alias_report = read_original(&ReadRequest {
            no_mirror: true,
            ..request(&lower_alias)
        })
        .unwrap();
        let alias_dl =
            display_list_with(&DisplayListRequest::new(&lower_alias), &Settings::default())
                .unwrap();
        assert_eq!(alias_report.sheet, "P-100");
        assert_eq!(alias_dl.sheet, "P-100");
        assert_eq!(
            alias_dl
                .objects
                .iter()
                .filter_map(|o| o.id.clone())
                .collect::<Vec<_>>(),
            expected_ids
        );
    }
}

#[test]
fn multiple_mleader_styles_produce_byte_identical_mirrors() {
    let dir = scratch("mleader-order");
    let description = dir.join("leaders.yaml");
    fs::write(
        &description,
        "units: mm\n\
         mleader_styles:\n\
         \x20 Z_STYLE: {text_height: 3.0}\n\
         \x20 A_STYLE: {text_height: 2.5}\n\
         \x20 M_STYLE: {text_height: 4.0}\n\
         objects:\n\
         \x20 - mleader: {leaders: [[[0, 0, 0], [10, 10, 0]]], content: {text: ONE}, mleader_style: Z_STYLE}\n\
         \x20 - mleader: {leaders: [[[20, 0, 0], [30, 10, 0]]], content: {text: TWO}, mleader_style: A_STYLE}\n\
         \x20 - mleader: {leaders: [[[40, 0, 0], [50, 10, 0]]], content: {text: THREE}, mleader_style: M_STYLE}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let dwg = dir.join("leaders.dwg");
    let mirror = dir.join("leaders.mirror.yaml");
    read_original(&request(&dwg)).unwrap();
    let first = fs::read_to_string(&mirror).unwrap();
    reads_as_yaml(&first).unwrap();
    for _ in 0..5 {
        read_original(&request(&dwg)).unwrap();
        assert_eq!(fs::read_to_string(&mirror).unwrap(), first);
    }
}

#[test]
fn block_definition_objects_carry_ids_and_handles_in_the_mirror() {
    let dir = scratch("block-handles");
    let library = dir.join("blocks.yaml");
    fs::write(
        &library,
        "units: mm\n\
         blocks:\n\
         \x20 MARKER:\n\
         \x20   objects:\n\
         \x20     - circle: {center: [0, 0, 0], radius: 5}\n\
         \x20     - line: {from: [-5, 0, 0], to: [5, 0, 0]}\n",
    )
    .unwrap();
    let description = dir.join("sheet.yaml");
    fs::write(
        &description,
        "units: mm\n\
         block_library: blocks.yaml\n\
         objects:\n\
         \x20 - insert: {block: MARKER, position: [20, 20, 0]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let original = dir.join("B-100.dwg");
    dwg::as_someone_elses(&dir.join("sheet.dwg"), &original);
    let report = read_original(&request(&original)).unwrap();
    assert!(report.decoded);
    let drawing = dwg::read(&original).unwrap();
    assert_eq!(drawing.blocks.len(), 1);
    assert_eq!(drawing.block_handles.len(), 1);
    assert_eq!(drawing.block_handles[0].len(), 2);

    let text = fs::read_to_string(dir.join("B-100.mirror.yaml")).unwrap();
    reads_as_yaml(&text).unwrap();
    let block_lines: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "    objects:")
        .skip(1)
        .take_while(|l| l.starts_with("      - "))
        .collect();
    assert_eq!(block_lines.len(), 2, "{text}");
    for (line, handle) in block_lines.iter().zip(&drawing.block_handles[0]) {
        assert!(
            line.starts_with(&format!(
                "      - {{id: B-100/handle:{handle}, handle: \"{handle}\", type: "
            )),
            "{line}"
        );
    }
}

#[test]
fn a_damaged_original_is_read_past_its_damage_which_the_reply_names() {
    let dir = scratch("damaged");
    let (original, _) = made_up_original(&dir);
    let damaged = dir.join("P-102.dwg");
    let handle = dwg::with_a_damaged_line(&original, &damaged);
    let report = read_original(&request(&damaged)).unwrap();
    assert!(!report.decoded);
    assert_eq!(report.skipped, 1, "{report:?}");
    assert!(report.recovered_errors >= 1, "{report:?}");
    assert!(report.read_past_damage.is_some(), "{report:?}");
    let expected = vec![RecordsOfKind {
        kind: "LINE".into(),
        count: 1,
        handles: vec![handle.clone()],
    }];
    assert_eq!(report.not_decoded, expected, "{report:?}");
    assert_eq!(
        report.not_read, expected,
        "skipped graphical object is in not_read: {report:?}"
    );
    assert_eq!(
        report.objects,
        BTreeMap::from([("circle".to_owned(), 1), ("polyline".to_owned(), 1)])
    );
    assert!(
        report.message.contains("could not read the file whole")
            && report.message.contains("1 LINE"),
        "{}",
        report.message
    );
    let text = fs::read_to_string(dir.join("P-102.mirror.yaml")).unwrap();
    reads_as_yaml(&text).unwrap();
    assert!(text.contains("\nread_past_damage: "), "{text}");
    assert!(
        text.contains(&format!(
            "\nnot_read: [{{kind: LINE, count: 1, handles: [\"{handle}\"]}}]\n"
        )),
        "{text}"
    );
}
