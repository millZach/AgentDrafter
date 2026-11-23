//! Reading an original: made-up drawings only, built by AgentDrafter and
//! made to look like someone else's (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::build::{build, BuildRequest};
use crate::error::ErrorCode;
use crate::{
    check_with, query_with, Answer, CheckRequest, End, Measure, ObjectRef, Query, QueryRequest,
    Rule, Selection, Settings, SnapKind, SnapRef,
};

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
        "del\u{007F}char",
        "c1\u{0080}ctrl\u{009F}",
        "bom\u{FEFF}char",
        "\u{FEFF}leading",
        "nonchar\u{FFFE}and\u{FFFF}",
        "é",
    ] {
        let out = scalar(tricky);
        assert!(out.starts_with('"'), "{tricky:?} -> {out}");
        assert!(
            out.chars().all(is_yaml12_safe_single_line),
            "YAML 1.2 §5.1 printable scalar for {tricky:?}: {out}"
        );
        assert_eq!(parse_yaml_scalar(&out), tricky, "{tricky:?} -> {out}");
    }
    assert_eq!(scalar("bom\u{FEFF}char"), "\"bom\\uFEFFchar\"");
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
        "del\u{007F}and\u{0080}c1\u{009F}.dwg",
        "bom\u{FEFF}name.dwg",
        "nonchar\u{FFFE}and\u{FFFF}.dwg",
        "~special: #1 \"q\".dwg",
    ] {
        let fake_path = dir.join(tricky_name);
        let sheet = sheet_name(&fake_path);
        let text = mirror_text(&fake_path, &bytes, &sheet, &drawing);
        let first_line = text.lines().next().unwrap();
        assert!(first_line.starts_with(MIRROR_MARK), "{first_line}");
        assert!(
            first_line.chars().all(is_yaml12_safe_single_line),
            "header comment must hold only YAML 1.2 §5.1 printable chars: {first_line:?}"
        );
        assert!(
            text.chars()
                .all(|c| c == '\n' || is_yaml12_safe_single_line(c)),
            "mirror YAML must hold only YAML 1.2 §5.1 printable chars: {tricky_name:?}"
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
fn replacing_a_vanished_mirror_never_clobbers_a_newly_created_file() {
    let dir = scratch("vanished-mirror");
    let (original, _) = made_up_original(&dir);
    let mirror = original.with_extension(MIRROR_EXTENSION);
    let source_id = file_id_of_path(&original);
    read_original(&request(&original)).unwrap();
    // Initial check saw ReplaceExisting, then the mirror was removed and a
    // user's own file appeared at the same path before write_whole.
    fs::remove_file(&mirror).unwrap();
    fs::write(&mirror, b"user's own notes\n").unwrap();
    let err = write_whole(
        &original,
        source_id,
        &mirror,
        b"new mirror\n",
        MirrorTarget::ReplaceExisting,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotAnOutput, "{err:?}");
    assert_eq!(fs::read(&mirror).unwrap(), b"user's own notes\n");

    // If the mirror vanished and nothing else is there, ReplaceExisting routes
    // to create-new placement and writes cleanly.
    fs::remove_file(&mirror).unwrap();
    let valid_mirror = format!("{MIRROR_MARK}P-100.dwg\n");
    write_whole(
        &original,
        source_id,
        &mirror,
        valid_mirror.as_bytes(),
        MirrorTarget::ReplaceExisting,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&mirror).unwrap(), valid_mirror);
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

    // Differently-cased stem (`p-100.dwg`) for the same file: on
    // case-insensitive volumes the path directly names `P-100.dwg` and on
    // case-sensitive Unix a symlink exercises the same on-disk resolution.
    // Both IDs AND the full written text mirror (including header comment and
    // `mirror_of`) must be byte-identical.
    let canonical_mirror = dir.join("canonical.mirror.yaml");
    read_original(&ReadRequest {
        mirror: Some(canonical_mirror.clone()),
        ..request(&original)
    })
    .unwrap();
    let canonical_bytes = fs::read(&canonical_mirror).unwrap();

    let lower_stem = dir.join("p-100.dwg");
    let same_file_directly = crate::build::is_same_file(&original, &lower_stem);
    #[cfg(unix)]
    if !same_file_directly && fs::symlink_metadata(&lower_stem).is_err() {
        std::os::unix::fs::symlink(&original, &lower_stem).unwrap();
    }
    if same_file_directly || fs::symlink_metadata(&lower_stem).is_ok() {
        let alias_mirror = dir.join("alias.mirror.yaml");
        let alias_report = read_original(&ReadRequest {
            mirror: Some(alias_mirror.clone()),
            ..request(&lower_stem)
        })
        .unwrap();
        let alias_dl =
            display_list_with(&DisplayListRequest::new(&lower_stem), &Settings::default()).unwrap();
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
        assert_eq!(
            fs::read(&alias_mirror).unwrap(),
            canonical_bytes,
            "same file via case alias must produce a byte-identical mirror"
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
    assert_eq!(
        report.sha256,
        crate::stamp::fingerprint(&fs::read(&damaged).unwrap())
    );
    assert_eq!(report.skipped, 1, "{report:?}");
    assert!(report.recovered_errors >= 1, "{report:?}");
    assert!(report.read_past_damage.is_some(), "{report:?}");
    // Strict `display-list` refuses the damaged DWG by default, while explicit
    // `read_past_damage: true` draws the recoverable objects read-only (ADR-0004).
    let strict_dl = crate::display_list(&crate::DisplayListRequest::new(&damaged));
    assert!(
        strict_dl.is_err(),
        "default display-list must remain strict on damaged DWGs"
    );
    let mut recovered_req = crate::DisplayListRequest::new(&damaged);
    recovered_req.read_past_damage = true;
    let recovered_dl = crate::display_list(&recovered_req).unwrap();
    assert_eq!(
        recovered_dl.objects.len(),
        2,
        "recoverable circle + polyline must draw when read_past_damage is true"
    );
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

#[test]
fn on_disk_path_prefers_unicode_case_fold_and_breaks_hard_link_ties_deterministically() {
    let dir = scratch("on-disk-casefold");
    let strasse = dir.join("Straße.dwg");
    fs::write(&strasse, b"stub").unwrap();
    let target_id = file_id_of_path(&strasse);
    assert!(target_id.is_some());

    // Create hard links that sort before and after `Straße.dwg` (plus a
    // symlink that sorts even earlier, which must be ignored).
    let aaa = dir.join("AAA.dwg");
    let zzz = dir.join("ZZZ.dwg");
    fs::hard_link(&strasse, &aaa).unwrap();
    fs::hard_link(&strasse, &zzz).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&strasse, dir.join("000-link.dwg")).unwrap();

    // 1. Exact match wins over lexicographically smaller hard link.
    assert_eq!(
        on_disk_path(&zzz, target_id).file_name().unwrap(),
        "ZZZ.dwg"
    );

    // 2. Unicode case-fold match (`STRASSE.dwg` <-> `Straße.dwg`, where
    //    `to_lowercase()` differs but `to_uppercase()` matches) wins over
    //    lexicographically smaller hard link `AAA.dwg`.
    let upper_query = dir.join("STRASSE.dwg");
    assert_eq!(
        on_disk_path(&upper_query, target_id).file_name().unwrap(),
        "Straße.dwg"
    );

    // 3. Reverse direction: querying `Straße.dwg` against `STRASSE.dwg` + `AAA.dwg`.
    let dir2 = scratch("on-disk-casefold-rev");
    let upper_disk = dir2.join("STRASSE.dwg");
    fs::write(&upper_disk, b"stub").unwrap();
    let target_id2 = file_id_of_path(&upper_disk);
    fs::hard_link(&upper_disk, dir2.join("AAA.dwg")).unwrap();
    assert_eq!(
        on_disk_path(&dir2.join("Straße.dwg"), target_id2)
            .file_name()
            .unwrap(),
        "STRASSE.dwg"
    );

    // 4. Unrelated query falls back deterministically to the lexicographically
    //    smallest regular non-symlink same-inode entry (`AAA.dwg`, never
    //    `000-link.dwg` and independent of `readdir` order).
    let unrelated = dir.join("UNRELATED.dwg");
    assert_eq!(
        on_disk_path(&unrelated, target_id).file_name().unwrap(),
        "AAA.dwg"
    );
}

#[test]
fn query_covers_every_kind_on_an_original_dwg_with_handle_ids() {
    use crate::check::{check_with, CheckRequest};
    use crate::geometry::{ObjectRef, SnapKind, SnapRef};
    use crate::query::{
        query_with, Answer, End, Matching, Measure, Query, QueryRequest, Selection, SelectionBox,
    };
    use crate::settings::Settings;

    let dir = scratch("query-original");
    let library = dir.join("blocks.yaml");
    fs::write(
        &library,
        "units: mm\n\
         blocks:\n\
         \x20 TAG_BOX:\n\
         \x20   objects:\n\
         \x20     - circle: {center: [0, 0, 0], radius: 6}\n\
         \x20     - text: {contents: MADE UP PLANT, position: [-5, 8, 0], height: 2}\n\
         \x20     - attdef: {tag: TAG, position: [0, 0, 0], height: 3, justify: middle_center}\n",
    )
    .unwrap();
    let description = dir.join("built.yaml");
    fs::write(
        &description,
        "units: mm\n\
         block_library: blocks.yaml\n\
         layers:\n\
         \x20 E-WIRE: {color: 1}\n\
         \x20 HIDDEN: {color: 8, on: false}\n\
         objects:\n\
         \x20 - line: {from: [0, 0, 0], to: [100, 0, 0], layer: E-WIRE}\n\
         \x20 - line: {from: [100, 0, 0], to: [100, 50, 0], layer: E-WIRE}\n\
         \x20 - circle: {center: [50, 50, 0], radius: 10}\n\
         \x20 - insert: {block: TAG_BOX, position: [20, 20, 0], attributes: {TAG: P-101}}\n\
         \x20 - text: {contents: NOTE A, position: [10, 80, 0], height: 4}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let original = dir.join("P-100.dwg");
    dwg::as_someone_elses(&dir.join("built.dwg"), &original);
    let before = fs::read(&original).unwrap();

    // `read` gives the canonical sheet name and handle IDs; `query` must agree.
    let read_report = read_original(&ReadRequest {
        no_mirror: true,
        ..request(&original)
    })
    .unwrap();
    assert_eq!(read_report.sheet, "P-100");
    let drawing = dwg::read(&original).unwrap();
    let h_line1 = &drawing.handles[0];
    let h_line2 = &drawing.handles[1];
    let h_circle = &drawing.handles[2];
    let h_insert = &drawing.handles[3];
    let h_text = &drawing.handles[4];
    let id_line1 = format!("P-100/handle:{h_line1}");
    let id_line2 = format!("P-100/handle:{h_line2}");
    let id_circle = format!("P-100/handle:{h_circle}");
    let id_insert = format!("P-100/handle:{h_insert}");
    let id_text = format!("P-100/handle:{h_text}");

    let ask = |path: &Path, sheet: Option<&str>, query: Query| {
        query_with(
            &QueryRequest {
                source: path.to_owned(),
                sheet: sheet.map(str::to_owned),
                query,
            },
            &Settings::default(),
        )
        .unwrap()
    };

    // 1. find: all objects, provenance has file+handle and no YAML line/column/field.
    let all = ask(&original, None, Query::Find(Selection::default()));
    assert_eq!(all.units, Some(Unit::Mm));
    assert_eq!(all.sheets, ["P-100"]);
    assert_eq!(all.objects_not_read, 0);
    assert!(all.not_read.is_empty());
    let all_json = serde_json::to_value(&all).unwrap();
    assert!(all_json.get("objects_not_read").is_none(), "{all_json}");
    assert!(all_json.get("not_read").is_none(), "{all_json}");
    let Answer::Find { count, objects } = &all.answer else {
        panic!("find answer")
    };
    assert_eq!(*count, 5);
    assert_eq!(
        objects.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        [
            id_line1.as_str(),
            id_line2.as_str(),
            id_circle.as_str(),
            id_insert.as_str(),
            id_text.as_str()
        ]
    );
    for (found, handle) in objects.iter().zip(&drawing.handles) {
        assert!(!found.id_written);
        assert_eq!(found.written_at.file, original.display().to_string());
        assert_eq!(found.written_at.handle.as_deref(), Some(handle.as_str()));
        assert_eq!(found.written_at.field, None);
        assert_eq!(found.written_at.line, None);
        assert_eq!(found.written_at.column, None);
        let v = serde_json::to_value(found).unwrap();
        assert_eq!(v["handle"], *handle);
        assert!(v.get("field").is_none(), "{v}");
        assert!(v.get("line").is_none(), "{v}");
        assert!(v.get("column").is_none(), "{v}");
    }

    // find by handle ID (case-insensitive) + --sheet matching case-insensitively.
    let by_id = ask(
        &original,
        Some("p-100"),
        Query::Find(Selection {
            id: Some(id_line1.to_lowercase()),
            ..Selection::default()
        }),
    );
    let Answer::Find { count, objects } = &by_id.answer else {
        panic!("find")
    };
    assert_eq!(*count, 1);
    assert_eq!(objects[0].id, id_line1);

    // Wrong --sheet name returns UnknownName.
    let bad_sheet = query_with(
        &QueryRequest {
            source: original.clone(),
            sheet: Some("OTHER".into()),
            query: Query::Blocks,
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(bad_sheet.code, ErrorCode::UnknownName);
    assert!(bad_sheet.message.contains("P-100"), "{}", bad_sheet.message);

    // Uppercase .DWG extension works too.
    let upper = dir.join("P-100.DWG");
    if !upper.exists() {
        fs::copy(&original, &upper).unwrap();
    }
    let upper_find = ask(
        &upper,
        None,
        Query::Find(Selection {
            id: Some(id_circle.clone()),
            ..Selection::default()
        }),
    );
    assert_eq!(upper_find.sheets, ["P-100"]);
    assert!(matches!(upper_find.answer, Answer::Find { count: 1, .. }));

    // find by --block, --tag + --value, --text (sheet text and block text),
    // --window, --crossing, --touching.
    let by_block_text = ask(
        &original,
        None,
        Query::Find(Selection {
            text: Some("MADE UP".into()),
            matching: Matching::Contains,
            ..Selection::default()
        }),
    );
    let Answer::Find { objects, .. } = &by_block_text.answer else {
        panic!("find")
    };
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, id_insert);
    assert_eq!(objects[0].block_text.len(), 1);
    assert_eq!(
        objects[0].block_text[0].file,
        original.display().to_string()
    );
    assert_eq!(objects[0].block_text[0].field, None);
    assert_eq!(
        objects[0].block_text[0].handle.as_deref(),
        Some(drawing.block_handles[0][1].as_str())
    );
    assert_eq!(objects[0].block_text[0].contents, "MADE UP PLANT");

    let by_window = ask(
        &original,
        None,
        Query::Find(Selection {
            window: Some(SelectionBox([[35.0, 35.0], [65.0, 65.0]])),
            ..Selection::default()
        }),
    );
    let Answer::Find { objects, .. } = &by_window.answer else {
        panic!("find")
    };
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, id_circle);

    let by_crossing = ask(
        &original,
        None,
        Query::Find(Selection {
            crossing: Some(SelectionBox([[95.0, -5.0], [105.0, 5.0]])),
            ..Selection::default()
        }),
    );
    let Answer::Find { objects, .. } = &by_crossing.answer else {
        panic!("find")
    };
    assert_eq!(
        objects.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        [id_line1.as_str(), id_line2.as_str()]
    );

    let by_touching = ask(
        &original,
        None,
        Query::Find(Selection {
            touching: Some(ObjectRef {
                id: Some(id_line1.clone()),
                name: None,
            }),
            ..Selection::default()
        }),
    );
    let Answer::Find { objects, .. } = &by_touching.answer else {
        panic!("find")
    };
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, id_line2);

    // 2. blocks
    let blocks = ask(&original, None, Query::Blocks);
    let Answer::Blocks { count, blocks } = &blocks.answer else {
        panic!("blocks")
    };
    assert_eq!(*count, 1);
    assert_eq!(blocks[0].blocks[0].block, "TAG_BOX");
    assert_eq!(blocks[0].blocks[0].inserts, [id_insert.as_str()]);

    // 3. attributes
    let attrs = ask(
        &original,
        None,
        Query::Attributes(Selection {
            tag: Some("TAG".into()),
            value: Some("P-101".into()),
            ..Selection::default()
        }),
    );
    let Answer::Attributes { count, inserts } = &attrs.answer else {
        panic!("attributes")
    };
    assert_eq!(*count, 1);
    assert_eq!(inserts[0].id, id_insert);
    assert_eq!(
        inserts[0].written_at.handle.as_deref(),
        Some(h_insert.as_str())
    );
    assert_eq!(inserts[0].written_at.field, None);

    // 4. measure
    let measured = ask(
        &original,
        None,
        Query::Measure(Measure {
            from: End {
                id: Some(id_line1.clone()),
                point: Some("from".into()),
                ..End::default()
            },
            to: End {
                id: Some(id_line1.clone()),
                point: Some("to".into()),
                ..End::default()
            },
        }),
    );
    let Answer::Measure { distance, .. } = &measured.answer else {
        panic!("measure")
    };
    assert!((distance.unwrap() - 100.0).abs() < 1e-9);

    // 5. extents
    let ext = ask(&original, None, Query::Extents);
    let Answer::Extents { count, extents, .. } = &ext.answer else {
        panic!("extents")
    };
    assert_eq!(*count, 1);
    let b = extents[0].extents.unwrap();
    assert!((b.min[0] - 0.0).abs() < 1e-3 && (b.max[0] - 100.0).abs() < 1e-3);

    // 6. layers
    let layers = ask(&original, None, Query::Layers);
    let Answer::Layers { layers, .. } = &layers.answer else {
        panic!("layers")
    };
    let names: Vec<&str> = layers[0]
        .layers
        .iter()
        .map(|l| l.layer.name.as_str())
        .collect();
    assert!(names.contains(&"0") && names.contains(&"E-WIRE") && names.contains(&"HIDDEN"));
    let wire_layer = layers[0]
        .layers
        .iter()
        .find(|l| l.layer.name == "E-WIRE")
        .unwrap();
    assert_eq!(wire_layer.objects, 2);

    // 7. snap
    let snapped = ask(
        &original,
        None,
        Query::Snap(SnapRef {
            id: Some(id_circle.clone()),
            snap: Some(SnapKind::Center),
            ..SnapRef::default()
        }),
    );
    let Answer::Snap { count, points, .. } = &snapped.answer else {
        panic!("snap")
    };
    assert_eq!(*count, 1);
    assert_eq!(points[0].at, crate::model::Point3::new(50.0, 50.0, 0.0));

    // 8. geometry
    let geom = ask(
        &original,
        None,
        Query::Geometry(Selection {
            id: Some(id_line1.clone()),
            ..Selection::default()
        }),
    );
    let Answer::Geometry { count, objects } = &geom.answer else {
        panic!("geometry")
    };
    assert_eq!(*count, 1);
    assert!((objects[0].measures.length.unwrap() - 100.0).abs() < 1e-9);

    // 9. layout (model-space only)
    let lay = ask(&original, None, Query::Layout);
    let Answer::Layout { count, layouts } = &lay.answer else {
        panic!("layout")
    };
    assert_eq!(*count, 0);
    assert_eq!(layouts.len(), 1);
    assert_eq!(layouts[0].layout, None);
    assert_eq!(layouts[0].model_space, 5);

    // Clean original DWG passes `check` (no fake `paper_size` failure, no false
    // positive `text_overflow` on the DWG block's attdef).
    let checked = check_with(
        &CheckRequest {
            source: original.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);
    assert_eq!(checked.units, Some(Unit::Mm));
    assert_eq!(checked.sheets, ["P-100"]);
    assert_eq!(checked.objects_not_read, 0);
    assert!(checked.not_read.is_empty());

    assert_eq!(
        fs::read(&original).unwrap(),
        before,
        "query and check never modify an original DWG"
    );
}

#[test]
fn query_and_check_handle_layout_original_dwgs_with_space_and_paper() {
    use crate::check::{check_with, CheckRequest, Rule};
    use crate::query::{query_with, Answer, Query, QueryRequest, Selection};
    use crate::settings::Settings;

    let dir = scratch("layout-original");
    let description = dir.join("built.yaml");
    fs::write(
        &description,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Sheet1}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [420, 0, 0], [420, 297, 0], [0, 297, 0]], closed: true, space: paper}\n\
         \x20 - viewport: {center: [210, 148.5, 0], width: 200, height: 150, view_center: [50, 50, 0], scale: 1}\n\
         \x20 - circle: {center: [50, 50, 0], radius: 20}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let original = dir.join("L-200.dwg");
    dwg::as_someone_elses(&dir.join("built.dwg"), &original);

    // query layout reports layout, paper, page setup and viewport handle.
    let lay = query_with(
        &QueryRequest {
            source: original.clone(),
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Layout { count, layouts } = &lay.answer else {
        panic!("layout")
    };
    assert_eq!(*count, 1);
    assert_eq!(layouts[0].layout.as_deref(), Some("Sheet1"));
    assert_eq!(
        layouts[0].paper.as_ref().and_then(|p| p.name.as_deref()),
        Some("ISO A3 landscape")
    );
    assert!(layouts[0].page_setup.is_some());
    assert!(layouts[0].viewports[0].id.starts_with("L-200/handle:"));
    assert!(layouts[0].viewports[0].written_at.handle.is_some());
    assert_eq!(layouts[0].viewports[0].written_at.field, None);

    // --space filters model vs paper space cleanly.
    let model_only = query_with(
        &QueryRequest {
            source: original.clone(),
            sheet: None,
            query: Query::Find(Selection {
                space: Some("model".into()),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { count, objects } = &model_only.answer else {
        panic!("find")
    };
    assert_eq!(*count, 1);
    assert_eq!(objects[0].space.as_deref(), Some("model"));

    let paper_only = query_with(
        &QueryRequest {
            source: original.clone(),
            sheet: None,
            query: Query::Find(Selection {
                space: Some("paper".into()),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { count, .. } = &paper_only.answer else {
        panic!("find")
    };
    assert_eq!(*count, 2);

    // Clean layout original passes check (including paper_size and off_sheet).
    let checked = check_with(
        &CheckRequest {
            source: original.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);

    // Layout original with an object running off paper fails paper_size & off_sheet.
    let bad_desc = dir.join("off.yaml");
    fs::write(
        &bad_desc,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Sheet1}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [420, 0, 0], [420, 297, 0], [0, 297, 0]], closed: true, space: paper}\n\
         \x20 - line: {from: [410, 10, 0], to: [450, 10, 0], space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: bad_desc,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let off_orig = dir.join("L-201.dwg");
    dwg::as_someone_elses(&dir.join("off.dwg"), &off_orig);
    let off_report = check_with(
        &CheckRequest {
            source: off_orig.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(!off_report.passed);
    let off_finding = off_report
        .findings
        .iter()
        .find(|f| f.rule == Rule::OffSheet)
        .expect("off_sheet finding");
    let obj = off_finding.object.as_ref().unwrap();
    assert!(obj.id.starts_with("L-201/handle:"));
    assert!(obj.written_at.handle.is_some());
    assert_eq!(obj.written_at.field, None);
    assert!(off_report
        .findings
        .iter()
        .any(|f| f.rule == Rule::PaperSize));
}

#[test]
fn check_on_original_dwg_catches_text_on_text_text_on_line_and_table_overflow() {
    use crate::check::{check_with, CheckRequest, Rule};
    use crate::settings::Settings;

    let dir = scratch("check-faults");
    let library = dir.join("blocks.yaml");
    fs::write(
        &library,
        "units: mm\n\
         blocks:\n\
         \x20 BADGE:\n\
         \x20   objects:\n\
         \x20     - circle: {center: [0, 0, 0], radius: 10}\n\
         \x20     - attdef: {tag: TAG, position: [0, 0, 0], height: 4, justify: middle_center}\n",
    )
    .unwrap();
    let description = dir.join("faults.yaml");
    fs::write(
        &description,
        "units: mm\n\
         block_library: blocks.yaml\n\
         objects:\n\
         \x20 - text: {contents: ALPHA, position: [10, 10, 0], height: 5}\n\
         \x20 - text: {contents: BETA, position: [12, 10, 0], height: 5}\n\
         \x20 - line: {from: [5, 12.5, 0], to: [40, 12.5, 0]}\n\
         \x20 - insert: {block: BADGE, position: [80, 50, 0], attributes: {TAG: T-1}}\n\
         \x20 - line: {from: [70, 50, 0], to: [90, 50, 0]}\n\
         \x20 - table: {position: [120, 80, 0], columns: [10], rows: [5], cells: [[THIS TEXT OVERFLOWS ITS TABLE CELL]]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let original = dir.join("F-100.dwg");
    dwg::as_someone_elses(&dir.join("faults.dwg"), &original);
    let before = fs::read(&original).unwrap();

    let report = check_with(
        &CheckRequest {
            source: original.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(!report.passed);
    assert!(
        report.findings.iter().all(|f| f.rule != Rule::PaperSize),
        "no fake paper_size failure on a model-space original: {:#?}",
        report.findings
    );
    assert!(report.findings.iter().any(|f| f.rule == Rule::TextOnText));
    assert!(report.findings.iter().any(|f| f.rule == Rule::TextOnLine));
    assert!(report.findings.iter().any(|f| f.rule == Rule::TextOverflow));

    // Every finding's object has handle ID + handle provenance and no YAML field/line/column.
    for finding in &report.findings {
        let obj = finding.object.as_ref().unwrap();
        assert!(obj.id.starts_with("F-100/handle:"), "{obj:?}");
        assert_eq!(obj.written_at.file, original.display().to_string());
        assert!(obj.written_at.handle.is_some());
        assert_eq!(obj.written_at.field, None);
        assert_eq!(obj.written_at.line, None);
        assert_eq!(obj.written_at.column, None);
    }

    // The line through the insert's attribute names the block part by handle.
    let attr_hit = report
        .findings
        .iter()
        .find(|f| {
            f.rule == Rule::TextOnLine
                && f.object
                    .as_ref()
                    .and_then(|o| o.in_block.as_ref())
                    .is_some()
        })
        .expect("line through insert attribute");
    let part = attr_hit.object.as_ref().unwrap().in_block.as_ref().unwrap();
    assert_eq!(part.block, "BADGE");
    assert_eq!(part.file, original.display().to_string());
    assert_eq!(part.field, None);
    assert!(part.handle.is_some());
    assert_eq!(part.tag.as_deref(), Some("TAG"));

    // Wrong --sheet name returns UnknownName; matching case-insensitively succeeds.
    let ok_sheet = check_with(
        &CheckRequest {
            source: original.clone(),
            sheets: vec!["f-100".into()],
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(ok_sheet.count, report.count);
    let err = check_with(
        &CheckRequest {
            source: original.clone(),
            sheets: vec!["NOPE".into()],
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownName);

    assert_eq!(fs::read(&original).unwrap(), before);
}

#[test]
fn query_and_check_report_not_read_handle_unitless_dwgs_and_refuse_damaged_dwgs() {
    use crate::check::{check_with, CheckRequest};
    use crate::query::{query_with, Answer, End, Measure, Query, QueryRequest, Selection};
    use crate::settings::Settings;

    let dir = scratch("edge-dwgs");
    let (original, _) = made_up_original(&dir);

    // 1. Custom undecoded object is surfaced in `objects_not_read` and `not_read`.
    let custom = dir.join("P-101.dwg");
    let handle = dwg::with_a_made_up_custom_object(&original, &custom, "MADE_UP_DEVICE");
    let expected = vec![RecordsOfKind {
        kind: "MADE_UP_DEVICE".into(),
        count: 1,
        handles: vec![handle],
    }];
    let q = query_with(
        &QueryRequest {
            source: custom.clone(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(q.objects_not_read, 1);
    assert_eq!(q.not_read, expected);
    assert!(
        q.message
            .contains("1 object the core has no model for not read: 1 MADE_UP_DEVICE"),
        "{}",
        q.message
    );
    let c = check_with(
        &CheckRequest {
            source: custom.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(c.passed, "{:#?}", c.findings);
    assert_eq!(c.objects_not_read, 1);
    assert_eq!(c.not_read, expected);
    assert!(
        c.message
            .contains("1 object the core has no model for not checked: 1 MADE_UP_DEVICE"),
        "{}",
        c.message
    );

    // 2. Unitless DWG reports `units: null` and works without panicking.
    let unitless = dir.join("U-100.dwg");
    dwg::as_unitless_original(&dir.join("built.dwg"), &unitless);
    let before = fs::read(&unitless).unwrap();
    let u_find = query_with(
        &QueryRequest {
            source: unitless.clone(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(u_find.units, None);
    assert_eq!(serde_json::to_value(&u_find).unwrap()["units"], Value::Null);
    let Answer::Find { objects, .. } = &u_find.answer else {
        panic!("find")
    };
    let id0 = objects[0].id.clone();

    let u_ext = query_with(
        &QueryRequest {
            source: unitless.clone(),
            sheet: None,
            query: Query::Extents,
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(u_ext.units, None);
    assert!(u_ext.message.contains("drawing units"), "{}", u_ext.message);

    let u_meas = query_with(
        &QueryRequest {
            source: unitless.clone(),
            sheet: None,
            query: Query::Measure(Measure {
                from: End {
                    id: Some(id0.clone()),
                    point: Some("from".into()),
                    ..End::default()
                },
                to: End {
                    id: Some(id0.clone()),
                    point: Some("to".into()),
                    ..End::default()
                },
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(
        u_meas.message.contains("drawing units"),
        "{}",
        u_meas.message
    );

    let u_geom = query_with(
        &QueryRequest {
            source: unitless.clone(),
            sheet: None,
            query: Query::Geometry(Selection {
                id: Some(id0),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(
        u_geom.message.contains("drawing units"),
        "{}",
        u_geom.message
    );

    let u_check = check_with(
        &CheckRequest {
            source: unitless.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(u_check.passed, "{:#?}", u_check.findings);
    assert_eq!(u_check.units, None);
    assert!((u_check.tolerance - 0.01).abs() < 1e-12);
    assert_eq!(
        serde_json::to_value(&u_check).unwrap()["units"],
        Value::Null
    );
    assert_eq!(fs::read(&unitless).unwrap(), before);

    // 3. Damaged DWG is refused strictly with `dwg_read` by both `query` and `check`.
    let damaged = dir.join("P-102.dwg");
    dwg::with_a_damaged_line(&original, &damaged);
    let q_err = query_with(
        &QueryRequest {
            source: damaged.clone(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(q_err.code, ErrorCode::DwgRead, "{q_err:?}");
    let c_err = check_with(
        &CheckRequest {
            source: damaged,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(c_err.code, ErrorCode::DwgRead, "{c_err:?}");
}

#[test]
fn check_handles_layout_margins_unitless_inch_layouts_and_reports_skipped_checks() {
    let dir = scratch("check-layout-margins-and-skipped");

    // 1. ISO A3 landscape layout with non-zero printer margins (10 mm each side)
    //    and off-page geometry running past x = 410 (physical right edge in paper space).
    let a3_off_yaml = dir.join("a3-off.yaml");
    fs::write(
        &a3_off_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Sheet1}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [400, 0, 0], [400, 277, 0], [0, 277, 0]], closed: true, space: paper}\n\
         \x20 - line: {from: [400, 10, 0], to: [450, 10, 0], space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: a3_off_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let a3_margins_off = dir.join("A3-MARGINS-OFF.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("a3-off.dwg"),
        &a3_margins_off,
        "Sheet1",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ISO_A3_(420.00_x_297.00_MM)".to_owned()),
            margins_mm: Some((10.0, 10.0, 10.0, 10.0)),
            ..Default::default()
        },
    );
    let checked = check_with(
        &CheckRequest {
            source: a3_margins_off,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(!checked.passed, "off-page line must fail check");
    assert!(
        checked.checks_skipped.is_empty(),
        "{:#?}",
        checked.checks_skipped
    );
    assert!(
        checked.findings.iter().any(|f| f.rule == Rule::OffSheet),
        "{:#?}",
        checked.findings
    );
    assert!(
        checked.findings.iter().any(|f| f.rule == Rule::PaperSize),
        "{:#?}",
        checked.findings
    );

    // Clean geometry filling the physical A3 sheet (-10,-10 to 410,287) with 10 mm margins passes!
    let a3_clean_yaml = dir.join("a3-clean.yaml");
    fs::write(
        &a3_clean_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Sheet1}\n\
         objects:\n\
         \x20 - polyline: {points: [[-10, -10, 0], [410, -10, 0], [410, 287, 0], [-10, 287, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: a3_clean_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let a3_margins_clean = dir.join("A3-MARGINS-CLEAN.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("a3-clean.dwg"),
        &a3_margins_clean,
        "Sheet1",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ISO_A3_(420.00_x_297.00_MM)".to_owned()),
            margins_mm: Some((10.0, 10.0, 10.0, 10.0)),
            ..Default::default()
        },
    );
    let clean_checked = check_with(
        &CheckRequest {
            source: a3_margins_clean,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(clean_checked.passed, "{:#?}", clean_checked.findings);
    assert!(clean_checked.checks_skipped.is_empty());

    // 2. Unitless (INSUNITS = 0) inch-based ANSI B layout at 1:1 (plot_paper_units = 0):
    //    must use the layout's own plot_paper_units (inches) rather than assuming mm!
    let inch_yaml = dir.join("inch-off.yaml");
    fs::write(
        &inch_yaml,
        "units: in\n\
         paper: ANSI B landscape\n\
         layout: {name: InchSheet}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, space: paper}\n\
         \x20 - line: {from: [16, 5, 0], to: [19, 5, 0], space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: inch_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let unitless_inch = dir.join("UNITLESS-INCH.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("inch-off.dwg"),
        &unitless_inch,
        "InchSheet",
        true,
        &dwg::TestLayoutSetup::default(),
    );
    let u_checked = check_with(
        &CheckRequest {
            source: unitless_inch,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(u_checked.units, None);
    assert!(
        !u_checked.passed,
        "off-sheet line on unitless inch layout must fail"
    );
    assert!(u_checked.checks_skipped.is_empty());
    let off = u_checked
        .findings
        .iter()
        .find(|f| f.rule == Rule::OffSheet)
        .expect("off_sheet finding");
    assert!(
        (off.past.as_ref().unwrap().right - 2.0).abs() < 1e-6,
        "{off:#?}"
    );

    // 3. Layout with non-1:1 plot scale (1:2) skips paper_size and off_sheet explicitly.
    let scaled_dwg = dir.join("SCALED-LAYOUT.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("a3-off.dwg"),
        &scaled_dwg,
        "Sheet1",
        false,
        &dwg::TestLayoutSetup {
            plot_scale: Some((1.0, 2.0)),
            ..Default::default()
        },
    );
    let s_checked = check_with(
        &CheckRequest {
            source: scaled_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(s_checked.passed, "{:#?}", s_checked.findings);
    assert_eq!(s_checked.checks_skipped.len(), 2);
    assert_eq!(s_checked.checks_skipped[0].rule, Rule::PaperSize);
    assert_eq!(s_checked.checks_skipped[1].rule, Rule::OffSheet);
    assert_eq!(
        s_checked.checks_skipped[0].layout.as_deref(),
        Some("Sheet1")
    );
    assert!(
        s_checked.checks_skipped[0].reason.contains("1:2"),
        "{:#?}",
        s_checked.checks_skipped
    );
    assert!(
        s_checked.message.contains("skipped paper_size, off_sheet")
            && !s_checked.message.starts_with("every check passes "),
        "{}",
        s_checked.message
    );

    // 4. Genuinely paperless model-space original reports checks_skipped and explains in message.
    let model_yaml = dir.join("model.yaml");
    fs::write(
        &model_yaml,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 0, 0]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: model_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let model_orig = dir.join("MODEL-ORIG.dwg");
    dwg::as_someone_elses(&dir.join("model.dwg"), &model_orig);
    let m_checked = check_with(
        &CheckRequest {
            source: model_orig,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(m_checked.passed);
    assert_eq!(m_checked.checks_skipped.len(), 2);
    assert_eq!(m_checked.checks_skipped[0].rule, Rule::PaperSize);
    assert_eq!(m_checked.checks_skipped[1].rule, Rule::OffSheet);
    assert_eq!(m_checked.checks_skipped[0].layout, None);
    assert!(
        m_checked.message.contains("every check run passes")
            && m_checked.message.contains("skipped paper_size, off_sheet"),
        "{}",
        m_checked.message
    );
}

#[test]
fn query_layout_reports_stored_dwg_plotter_and_plot_style() {
    let dir = scratch("query-layout-stored-setup");
    let yaml = dir.join("lay.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Plan}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [420, 0, 0], [420, 297, 0], [0, 297, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let orig = dir.join("CUSTOM-SETUP.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("lay.dwg"),
        &orig,
        "Plan",
        false,
        &dwg::TestLayoutSetup {
            plotter: Some("MadeUp Plotter.pc3".to_owned()),
            plot_style_sheet: Some("madeup-color.ctb".to_owned()),
            paper_size: Some("ISO_A3_(420.00_x_297.00_MM)".to_owned()),
            margins_mm: Some((5.0, 5.0, 5.0, 5.0)),
            ..Default::default()
        },
    );
    let report = query_with(
        &QueryRequest {
            source: orig,
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Layout { layouts, .. } = &report.answer else {
        panic!("layout")
    };
    let setup = layouts[0]
        .page_setup
        .as_ref()
        .expect("stored page_setup present");
    assert_eq!(setup.plotter, "MadeUp Plotter.pc3");
    assert_eq!(setup.plot_style, "madeup-color.ctb");
    assert_eq!(setup.paper, "ISO_A3_(420.00_x_297.00_MM)");
    assert_eq!(setup.area, "layout");
    assert_eq!(setup.scale, "1:1");
    assert_eq!(
        layouts[0].paper.as_ref().and_then(|p| p.name.as_deref()),
        Some("ISO A3 landscape")
    );
}

#[test]
fn query_touching_snap_and_measure_do_not_cross_spaces() {
    let dir = scratch("query-no-cross-space");
    let yaml = dir.join("spaces.yaml");
    // Two crossing lines at identical raw coordinates [0,50]-[100,50] and [50,0]-[50,100],
    // one in model space and one in paper space. They MUST NOT touch, intersect or measure!
    fs::write(
        &yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Sheet1}\n\
         objects:\n\
         \x20 - line: {from: [0, 50, 0], to: [100, 50, 0]}\n\
         \x20 - line: {from: [50, 0, 0], to: [50, 100, 0], space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let orig = dir.join("SPACES.dwg");
    dwg::as_someone_elses(&dir.join("spaces.dwg"), &orig);

    let found = query_with(
        &QueryRequest {
            source: orig.clone(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = &found.answer else {
        panic!("find")
    };
    let model_id = objects
        .iter()
        .find(|o| o.space.as_deref() == Some("model"))
        .unwrap()
        .id
        .clone();
    let paper_id = objects
        .iter()
        .find(|o| o.space.as_deref() == Some("Sheet1"))
        .unwrap()
        .id
        .clone();

    // find --touching <paper_id> must not match the model-space line!
    let touching = query_with(
        &QueryRequest {
            source: orig.clone(),
            sheet: None,
            query: Query::Find(Selection {
                touching: Some(ObjectRef {
                    id: Some(paper_id.clone()),
                    name: None,
                }),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { count, .. } = &touching.answer else {
        panic!("find")
    };
    assert_eq!(*count, 0, "cross-space touching must be 0: {touching:?}");

    // snap --snap intersection across model and paper space must return 0 points!
    for snap_kind in [SnapKind::Intersection, SnapKind::ApparentIntersection] {
        let snap = query_with(
            &QueryRequest {
                source: orig.clone(),
                sheet: None,
                query: Query::Snap(SnapRef {
                    id: Some(model_id.clone()),
                    name: None,
                    snap: Some(snap_kind),
                    which: None,
                    with: Some(ObjectRef {
                        id: Some(paper_id.clone()),
                        name: None,
                    }),
                    from: None,
                }),
            },
            &Settings::default(),
        )
        .unwrap();
        let Answer::Snap { count, points, .. } = &snap.answer else {
            panic!("snap")
        };
        assert_eq!(*count, 0, "{snap_kind:?}: {snap:?}");
        assert!(points.is_empty());
        assert!(
            snap.message.contains("different spaces"),
            "{}",
            snap.message
        );
    }

    // snap --snap perpendicular --from across spaces must also return 0 points.
    let perp = query_with(
        &QueryRequest {
            source: orig.clone(),
            sheet: None,
            query: Query::Snap(SnapRef {
                id: Some(model_id.clone()),
                name: None,
                snap: Some(SnapKind::Perpendicular),
                which: None,
                with: None,
                from: Some(Box::new(SnapRef {
                    id: Some(paper_id.clone()),
                    name: None,
                    snap: Some(SnapKind::Midpoint),
                    which: None,
                    with: None,
                    from: None,
                })),
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Snap { count, .. } = &perp.answer else {
        panic!("snap")
    };
    assert_eq!(*count, 0, "{perp:?}");
    assert!(
        perp.message.contains("different spaces"),
        "{}",
        perp.message
    );

    // measure across spaces is refused cleanly with BadValue.
    let meas_err = query_with(
        &QueryRequest {
            source: orig,
            sheet: None,
            query: Query::Measure(Measure {
                from: End {
                    id: Some(model_id),
                    ..End::default()
                },
                to: End {
                    id: Some(paper_id),
                    ..End::default()
                },
            }),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(meas_err.code, ErrorCode::BadValue);
    assert!(
        meas_err.message.contains("same space"),
        "{}",
        meas_err.message
    );
}

#[test]
fn query_reports_ambiguous_carried_name_with_all_matching_ids() {
    let dir = scratch("query-ambiguous-name");
    let yaml = dir.join("dup.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         objects:\n\
         \x20 - line: {from: [0, 0, 0], to: [10, 0, 0]}\n\
         \x20 - line: {from: [0, 10, 0], to: [10, 10, 0]}\n\
         \x20 - line: {from: [5, -5, 0], to: [5, 15, 0]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let dup_dwg = dir.join("DUP.dwg");
    dwg::with_duplicate_carried_name(&dir.join("dup.dwg"), &dup_dwg, "same-wire");

    // 1. measure --from-name
    let err = query_with(
        &QueryRequest {
            source: dup_dwg.clone(),
            sheet: None,
            query: Query::Measure(Measure {
                from: End {
                    name: Some("same-wire".into()),
                    ..End::default()
                },
                to: End {
                    id: Some("DUP/obj-0".into()),
                    ..End::default()
                },
            }),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadValue, "{err:?}");
    assert!(
        err.message.contains("ambiguous")
            && err.message.contains("DUP/obj-0")
            && err.message.contains("DUP/obj-1"),
        "{}",
        err.message
    );

    // 2. snap --name
    let err = query_with(
        &QueryRequest {
            source: dup_dwg.clone(),
            sheet: None,
            query: Query::Snap(SnapRef {
                id: None,
                name: Some("same-wire".into()),
                snap: Some(SnapKind::Midpoint),
                which: None,
                with: None,
                from: None,
            }),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadValue, "{err:?}");
    assert!(
        err.message.contains("DUP/obj-0") && err.message.contains("DUP/obj-1"),
        "{}",
        err.message
    );

    // 3. find --touching-name
    let err = query_with(
        &QueryRequest {
            source: dup_dwg,
            sheet: None,
            query: Query::Find(Selection {
                touching: Some(ObjectRef {
                    id: None,
                    name: Some("same-wire".into()),
                }),
                ..Selection::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadValue, "{err:?}");
    assert!(
        err.message.contains("ambiguous")
            && err.message.contains("DUP/obj-0")
            && err.message.contains("DUP/obj-1"),
        "{}",
        err.message
    );
}

#[test]
fn check_preserves_exact_custom_paper_size_and_respects_layout_paper_units_and_tolerance() {
    let dir = scratch("custom-paper-and-layout-units");

    // 1. Custom 420.4 x 297.0 mm sheet (0.4 mm wider than ISO A3) with an
    //    exact-fitting border: check must keep exact stored dimensions (not
    //    snap to 420 x 297) so paper_size and off_sheet pass.
    let custom_yaml = dir.join("custom-420-4.yaml");
    fs::write(
        &custom_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Custom}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [420.4, 0, 0], [420.4, 297, 0], [0, 297, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: custom_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let custom_dwg = dir.join("CUSTOM-420-4.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("custom-420-4.dwg"),
        &custom_dwg,
        "Custom",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("UserDefinedMetric (420.40 x 297.00MM)".to_owned()),
            paper_width_mm: Some(420.4),
            paper_height_mm: Some(297.0),
            ..Default::default()
        },
    );
    let checked = check_with(
        &CheckRequest {
            source: custom_dwg.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);
    assert!(checked.checks_skipped.is_empty());
    let layout_q = query_with(
        &QueryRequest {
            source: custom_dwg,
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Layout { layouts, .. } = layout_q.answer else {
        panic!("layout")
    };
    let paper = layouts[0].paper.as_ref().expect("custom paper reported");
    assert_eq!(paper.name, None);
    assert!((paper.width - 420.4).abs() < 1e-9);
    assert!((paper.height - 297.0).abs() < 1e-9);

    // 2. INSUNITS = mm with inch plot_paper_units (0) at 1:1 (ANSI B 17 x 11
    //    drawn in inches in paper space): accepted as a valid 1:1 inch layout,
    //    keeping CheckReport.units = mm and CheckReport.tolerance = 0.01 mm
    //    consistent while checking the inch layout with inch tolerance and
    //    tagging any inch-layout finding with `units: Some(Unit::In)`.
    let mm_with_inch_layout = dir.join("mm-inch-layout.yaml");
    fs::write(
        &mm_with_inch_layout,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: InchTab}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: mm_with_inch_layout,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let mm_inch_dwg = dir.join("MM-INSUNITS-INCH-LAYOUT.dwg");
    let inch_tab_setup = dwg::TestLayoutSetup {
        paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
        paper_width_mm: Some(17.0 * 25.4),
        paper_height_mm: Some(11.0 * 25.4),
        plot_paper_units: Some(0), // inches
        plot_scale: Some((1.0, 1.0)),
        ..Default::default()
    };
    dwg::as_original_with_layout_setup(
        &dir.join("mm-inch-layout.dwg"),
        &mm_inch_dwg,
        "InchTab",
        false,
        &inch_tab_setup,
    );
    let checked = check_with(
        &CheckRequest {
            source: mm_inch_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);
    assert!(checked.checks_skipped.is_empty());
    assert_eq!(checked.units, Some(Unit::Mm));
    assert!(
        (checked.tolerance - 0.01).abs() < 1e-12,
        "CheckReport.tolerance must match CheckReport.units (mm), got {}",
        checked.tolerance
    );

    // Oversizing the same inch layout by 0.005 in (> 0.01 mm = 0.0003937 in,
    // < 0.01 in) fails using the layout's inch tolerance and tags the findings
    // with `units: Some(Unit::In)` so they never contradict `CheckReport.units: mm`.
    let mm_inch_over_yaml = dir.join("mm-inch-over.yaml");
    fs::write(
        &mm_inch_over_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: InchTab}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17.005, 0, 0], [17.005, 11, 0], [0, 11, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: mm_inch_over_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let mm_inch_over_dwg = dir.join("MM-INSUNITS-INCH-OVER.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("mm-inch-over.dwg"),
        &mm_inch_over_dwg,
        "InchTab",
        false,
        &inch_tab_setup,
    );
    let checked_over = check_with(
        &CheckRequest {
            source: mm_inch_over_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(checked_over.units, Some(Unit::Mm));
    assert!((checked_over.tolerance - 0.01).abs() < 1e-12);
    assert!(!checked_over.passed);
    assert!(checked_over
        .findings
        .iter()
        .any(|f| f.rule == Rule::PaperSize));
    assert!(checked_over
        .findings
        .iter()
        .any(|f| f.rule == Rule::OffSheet));
    for f in &checked_over.findings {
        assert_eq!(f.units, Some(Unit::In), "{f:#?}");
        assert!(f.message.contains(" in"), "{f:#?}");
    }

    // 3. Unitless (INSUNITS = 0) inch layout at 1:1 with border oversized by
    //    0.005 in (> 0.01 mm = 0.0003937 in, yet < 0.01 in): top-level
    //    CheckReport reports `units: None` and `tolerance: 0.01` (drawing
    //    units, matching docs/check.md), while the inch layout still fails
    //    against its 0.01 mm inch tolerance and tags findings with `units: in`.
    let tiny_over_yaml = dir.join("unitless-inch-tol.yaml");
    fs::write(
        &tiny_over_yaml,
        "units: in\n\
         paper: ANSI B landscape\n\
         layout: {name: InchTol}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17.005, 0, 0], [17.005, 11, 0], [0, 11, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: tiny_over_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let tiny_over_dwg = dir.join("UNITLESS-INCH-TOL.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("unitless-inch-tol.dwg"),
        &tiny_over_dwg,
        "InchTol",
        true,
        &dwg::TestLayoutSetup::default(),
    );
    let checked = check_with(
        &CheckRequest {
            source: tiny_over_dwg.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(checked.units, None);
    assert!(
        (checked.tolerance - 0.01).abs() < 1e-12,
        "unitless CheckReport.tolerance must stay 0.01 drawing units, got {}",
        checked.tolerance
    );
    assert!(
        !checked.passed,
        "0.005 in overshoot must fail 0.01 mm inch-layout tolerance"
    );
    assert!(checked.findings.iter().any(|f| f.rule == Rule::PaperSize));
    assert!(checked.findings.iter().any(|f| f.rule == Rule::OffSheet));
    for f in &checked.findings {
        assert_eq!(f.units, Some(Unit::In), "{f:#?}");
    }
    // `query layout` on a unitless DWG keeps the inch paper dimensions in
    // inches (17 x 11) and exposes `paper_unit: Some(Unit::In)` so callers
    // know what unit `paper` is sized in.
    let unitless_lq = query_with(
        &QueryRequest {
            source: tiny_over_dwg,
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(unitless_lq.units, None);
    let Answer::Layout { layouts, .. } = unitless_lq.answer else {
        panic!("layout")
    };
    let u_paper = layouts[0].paper.as_ref().expect("unitless inch paper");
    assert!((u_paper.width - 17.0).abs() < 1e-3, "{u_paper:?}");
    assert!((u_paper.height - 11.0).abs() < 1e-3, "{u_paper:?}");
    assert_eq!(layouts[0].paper_unit, Some(Unit::In));
}

#[test]
fn inactive_empty_configured_layout_is_reported_by_query_without_breaking_plot_render_or_check() {
    use crate::plot::{plot_with, PlotRequest};
    use crate::render::{render_with, RenderRequest};

    let dir = scratch("inactive-empty-layout");
    let yaml = dir.join("base.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Active}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [420, 0, 0], [420, 297, 0], [0, 297, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let dwg_path = dir.join("INACTIVE-EMPTY.dwg");
    dwg::as_original_with_extra_layouts(
        &dir.join("base.dwg"),
        &dwg_path,
        &[
            (
                "EmptyConfigured",
                dwg::TestLayoutSetup {
                    plotter: Some("DWG To PDF.pc3".to_owned()),
                    paper_size: Some("Custom_420x297_MM".to_owned()),
                    paper_width_mm: Some(420.4),
                    paper_height_mm: Some(297.0),
                    plot_paper_units: Some(1),
                    plot_type: Some(5),
                    plot_scale: Some((1.0, 1.0)),
                    margins_mm: Some((5.0, 5.0, 5.0, 5.0)),
                    plot_style_sheet: Some("monochrome.ctb".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "ScaledEmpty",
                dwg::TestLayoutSetup {
                    plotter: Some("MadeUp Plotter.pc3".to_owned()),
                    paper_size: Some("ISO_A3_(420.00_x_297.00_MM)".to_owned()),
                    paper_width_mm: Some(420.0),
                    paper_height_mm: Some(297.0),
                    plot_paper_units: Some(1),
                    plot_type: Some(5),
                    plot_scale: Some((1.0, 2.0)),
                    plot_style_sheet: Some("madeup-color.ctb".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "ScaledInchEmpty",
                dwg::TestLayoutSetup {
                    plotter: Some("MadeUp Plotter.pc3".to_owned()),
                    paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
                    paper_width_mm: Some(17.0 * 25.4),
                    paper_height_mm: Some(11.0 * 25.4),
                    plot_paper_units: Some(0), // inches on a mm DWG
                    plot_type: Some(5),
                    plot_scale: Some((1.0, 2.0)),
                    plot_style_sheet: Some("madeup-color.ctb".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "BlankDefault",
                dwg::TestLayoutSetup {
                    plotter: Some("none_device".to_owned()),
                    paper_size: Some("ANSI_A_(8.50_x_11.00_Inches)".to_owned()),
                    paper_width_mm: Some(215.9),
                    paper_height_mm: Some(279.4),
                    plot_style_sheet: Some(String::new()),
                    ..Default::default()
                },
            ),
        ],
    );

    // (a) `plot` and `render` must succeed without aborting on empty tabs or
    //     renaming the populated tab's output PNG or emitting a blank PNG, and
    //     `check` must only check the populated tab.
    let pdf = dir.join("active.pdf");
    let plotted = plot_with(
        &PlotRequest {
            output: Some(pdf.clone()),
            ..PlotRequest::new(&dwg_path)
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(pdf.is_file(), "PDF must be written: {plotted:?}");
    assert!(
        plotted.layouts.is_empty(),
        "single populated tab must not expand into multi-layout PDF pages: {plotted:?}"
    );

    let png = dir.join("active.png");
    let rendered = render_with(
        &RenderRequest {
            output: Some(png.clone()),
            ..RenderRequest::new(&dwg_path)
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(
        png.is_file(),
        "exact PNG filename must be kept: {rendered:?}"
    );
    assert_eq!(rendered.png, png.display().to_string());
    assert!(
        rendered.layout.is_none(),
        "single populated tab must not split into multi-layout PNGs: {rendered:?}"
    );
    assert!(!dir.join("active-EmptyConfigured.png").exists());
    assert!(!dir.join("active-ScaledEmpty.png").exists());

    let checked = check_with(
        &CheckRequest {
            source: dwg_path.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);
    assert!(checked.checks_skipped.is_empty());

    // (b) `query layout` reports both inactive empty configured tabs (including
    //     the 1:2 scaled tab) with their stored metadata, while skipping the
    //     untouched blank default tab.
    let layout_q = query_with(
        &QueryRequest {
            source: dwg_path,
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Layout { layouts, .. } = layout_q.answer else {
        panic!("layout")
    };
    let names: Vec<Option<&str>> = layouts.iter().map(|l| l.layout.as_deref()).collect();
    assert_eq!(layout_q.units, Some(Unit::Mm));
    assert_eq!(
        names,
        vec![
            Some("Active"),
            Some("EmptyConfigured"),
            Some("ScaledEmpty"),
            Some("ScaledInchEmpty"),
        ]
    );
    assert!(
        layouts[1]
            .paper
            .as_ref()
            .is_some_and(|p| (p.width - 420.4).abs() < 1e-9),
        "{:#?}",
        layouts[1]
    );
    assert_eq!(layouts[1].paper_unit, None);
    let empty_setup = layouts[1]
        .page_setup
        .as_ref()
        .expect("EmptyConfigured setup");
    assert_eq!(empty_setup.plotter, "DWG To PDF.pc3");
    assert_eq!(empty_setup.paper, "Custom_420x297_MM");
    assert_eq!(empty_setup.scale, "1:1");
    assert_eq!(empty_setup.plot_style, "monochrome.ctb");

    let scaled_setup = layouts[2].page_setup.as_ref().expect("ScaledEmpty setup");
    assert_eq!(scaled_setup.plotter, "MadeUp Plotter.pc3");
    assert_eq!(scaled_setup.paper, "ISO_A3_(420.00_x_297.00_MM)");
    assert_eq!(scaled_setup.scale, "1:2");
    assert_eq!(scaled_setup.plot_style, "madeup-color.ctb");
    let scaled_paper = layouts[2]
        .paper
        .as_ref()
        .expect("1:2 extra layout must still populate stored paper dimensions");
    assert_eq!(scaled_paper.name.as_deref(), Some("ISO A3 landscape"));
    assert!((scaled_paper.width - 420.0).abs() < 1e-9);
    assert!((scaled_paper.height - 297.0).abs() < 1e-9);
    assert_eq!(layouts[2].paper_unit, None);

    // An inch 1:2 extra tab on a millimetre DWG converts its paper dimensions
    // into the reply's declared unit (mm: 431.8 x 279.4) so every length in a
    // unitful `query layout` reply is in `report.units`.
    let inch_paper = layouts[3]
        .paper
        .as_ref()
        .expect("1:2 inch extra layout must populate converted paper dimensions");
    assert_eq!(inch_paper.name.as_deref(), Some("ANSI B landscape"));
    assert!((inch_paper.width - 431.8).abs() < 1e-3, "{inch_paper:?}");
    assert!((inch_paper.height - 279.4).abs() < 1e-3, "{inch_paper:?}");
    assert_eq!(layouts[3].paper_unit, None);

    // Conversely, a model-space-only drawing saved on Model space whose empty
    // Layout1 merely carries a template printer/paper must remain model-space-only
    // (never hijacking render/plot/check/query onto an empty layout).
    let model_yaml = dir.join("model-only.yaml");
    fs::write(
        &model_yaml,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 0, 0]}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: model_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let model_dwg = dir.join("MODEL-WITH-TEMPLATE-LAYOUT.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("model-only.dwg"),
        &model_dwg,
        "Layout1",
        false,
        &dwg::TestLayoutSetup {
            plotter: Some("DWG To PDF.pc3".to_owned()),
            paper_size: Some("ANSI_A_(8.50_x_11.00_Inches)".to_owned()),
            paper_width_mm: Some(215.9),
            paper_height_mm: Some(279.4),
            plot_paper_units: Some(0),
            plot_type: Some(5),
            plot_scale: Some((1.0, 1.0)),
            margins_mm: Some((6.35, 6.35, 6.35, 6.35)),
            ..Default::default()
        },
    );
    let model_layout = query_with(
        &QueryRequest {
            source: model_dwg,
            sheet: None,
            query: Query::Layout,
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Layout { layouts, .. } = model_layout.answer else {
        panic!("layout")
    };
    assert_eq!(layouts.len(), 1);
    assert_eq!(
        layouts[0].layout, None,
        "model-space-only drawing must not be hijacked by an untouched empty Layout1"
    );
}

#[test]
fn plot_and_render_do_not_regress_on_unitless_inch_layouts() {
    use crate::draw::PageFrom;
    use crate::plot::{plot_with, PlotRequest};
    use crate::render::{render_with, RenderRequest};

    let dir = scratch("unitless-inch-plot-render");
    let inch_yaml = dir.join("inch.yaml");
    fs::write(
        &inch_yaml,
        "units: in\n\
         paper: ANSI B landscape\n\
         layout: {name: InchTab}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: inch_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let unitless_inch = dir.join("UNITLESS-INCH.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("inch.dwg"),
        &unitless_inch,
        "InchTab",
        true,
        &dwg::TestLayoutSetup::default(),
    );

    // `read_paper` must NOT classify a unitless 1:1 inch layout as a 17 x 11 mm
    // page in `Layout.paper`.
    let read = crate::read_dwg(&unitless_inch).unwrap();
    assert_eq!(read.units, None);
    assert!(
        read.layout.as_ref().unwrap().paper.is_none(),
        "unitless 1:1 inch layout must not set Layout.paper to 17x11 mm: {:?}",
        read.layout
    );

    // Plotting it must not claim a 17 x 11 mm declared paper page.
    let pdf = dir.join("unitless-inch.pdf");
    let plotted = plot_with(
        &PlotRequest {
            output: Some(pdf.clone()),
            ..PlotRequest::new(&unitless_inch)
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(plotted.units, None);
    assert_eq!(
        plotted.page,
        PageFrom::Extents,
        "must not claim a 17 x 11 mm declared paper: {:?}",
        plotted
    );

    // Rendering still succeeds cleanly.
    let png = dir.join("unitless-inch.png");
    let rendered = render_with(
        &RenderRequest {
            output: Some(png),
            ..RenderRequest::new(&unitless_inch)
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(rendered.units, None);

    // Meanwhile, a unitless mm-drawn layout on ANSI B inch paper at 1:25.4
    // (the main-branch contract) still keeps its 431.8 x 279.4 mm (17 x 11 in)
    // declared paper in `plot`.
    let mm_on_ansi_yaml = dir.join("mm-on-ansi.yaml");
    fs::write(
        &mm_on_ansi_yaml,
        "units: mm\n\
         paper: ANSI B landscape\n\
         layout: {name: MmOnAnsi}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [431.8, 0, 0], [431.8, 279.4, 0], [0, 279.4, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: mm_on_ansi_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let unitless_mm_on_ansi = dir.join("UNITLESS-MM-ON-ANSI.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("mm-on-ansi.dwg"),
        &unitless_mm_on_ansi,
        "MmOnAnsi",
        true,
        &dwg::TestLayoutSetup::default(),
    );
    let plotted_mm = plot_with(
        &PlotRequest {
            output: Some(dir.join("unitless-mm-on-ansi.pdf")),
            ..PlotRequest::new(&unitless_mm_on_ansi)
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(
        plotted_mm.page,
        PageFrom::Paper,
        "1:25.4 unitless mm layout keeps declared paper: {plotted_mm:?}"
    );
    assert!((plotted_mm.sheet_size.width_mm - 431.8).abs() < 1e-3);
    assert!((plotted_mm.sheet_size.height_mm - 279.4).abs() < 1e-3);
}

#[test]
fn check_handles_asymmetric_margins_90deg_rotation_and_nonzero_plot_origin() {
    let dir = scratch("asymmetric-rot90-origin");
    // Media is portrait 297 x 420 mm with asymmetric margins:
    //   m_left = 12, m_bottom = 8, m_right = 18, m_top = 6
    // Under plot_rotation = 1 (90 deg CCW):
    //   rotated sheet size: w = 420, h = 297
    //   rotated margins: (left, bottom, right, top) = (m_bottom, m_right, m_top, m_left) = (8, 18, 6, 12)
    // With plot_origin = (5, -3):
    //   physical paper origin = [-(8 + 5), -(18 + (-3))] = [-13, -15]
    //   physical paper box    = [-13, -15] .. [407, 282]
    let clean_yaml = dir.join("rot90-clean.yaml");
    fs::write(
        &clean_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Rot90}\n\
         objects:\n\
         \x20 - polyline: {points: [[-13, -15, 0], [407, -15, 0], [407, 282, 0], [-13, 282, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: clean_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let clean_dwg = dir.join("ROT90-CLEAN.dwg");
    let configure = dwg::TestLayoutSetup {
        paper_size: Some("ISO_A3_(297.00_x_420.00_MM)".to_owned()),
        paper_width_mm: Some(297.0),
        paper_height_mm: Some(420.0),
        plot_rotation: Some(1),
        margins_mm: Some((12.0, 8.0, 18.0, 6.0)),
        plot_origin_mm: Some((5.0, -3.0)),
        ..Default::default()
    };
    dwg::as_original_with_layout_setup(
        &dir.join("rot90-clean.dwg"),
        &clean_dwg,
        "Rot90",
        false,
        &configure,
    );
    let checked = check_with(
        &CheckRequest {
            source: clean_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(checked.passed, "{:#?}", checked.findings);
    assert!(checked.checks_skipped.is_empty());

    // Shift by +1 mm on left (x = -14) and +1 mm on top (y = 283) -> must fail off_sheet on left & top.
    let off_yaml = dir.join("rot90-off.yaml");
    fs::write(
        &off_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: Rot90}\n\
         objects:\n\
         \x20 - polyline: {points: [[-14, -15, 0], [407, -15, 0], [407, 283, 0], [-14, 283, 0]], closed: true, space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: off_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let off_dwg = dir.join("ROT90-OFF.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("rot90-off.dwg"),
        &off_dwg,
        "Rot90",
        false,
        &configure,
    );
    let checked_off = check_with(
        &CheckRequest {
            source: off_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(!checked_off.passed);
    let off = checked_off
        .findings
        .iter()
        .find(|f| f.rule == Rule::OffSheet)
        .expect("off_sheet finding");
    let past = off.past.as_ref().expect("past sides");
    assert!((past.left - 1.0).abs() < 1e-6, "{past:?}");
    assert!((past.top - 1.0).abs() < 1e-6, "{past:?}");
    assert!(past.right.abs() < 1e-6, "{past:?}");
    assert!(past.bottom.abs() < 1e-6, "{past:?}");
}

#[test]
fn check_preserves_sheet_units_for_viewport_dimensions_and_uses_inch_units_on_extents_layout() {
    let dir = scratch("preserve-sheet-units-and-extents-inch");

    // (a) INSUNITS = mm DWG with an inch layout (`plot_paper_units = 0`, 1:1)
    //     showing a model-space stacked-fraction linear dimension (`12 1/2`)
    //     through a viewport: `check` MUST NOT mutate `sheet.units` to `in`,
    //     so the stacked fraction's whole-number piece (`12`) keeps its metric
    //     bottom alignment (`-7h/6`), matching `display-list` / `plot` (ADR-0011),
    //     instead of shifting up by `+2h/3` into imperial middle alignment!
    let dim_yaml = dir.join("dim-vp.yaml");
    fs::write(
        &dim_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: InchTab}\n\
         dimension_styles:\n\
         \x20 FRAC:\n\
         \x20   text_height: 1.0\n\
         \x20   arrow_size: 0.2\n\
         \x20   unit_format: fractional\n\
         \x20   fraction_format: horizontal\n\
         \x20   precision: 4\n\
         \x20   placement: centered\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, space: paper}\n\
         \x20 - viewport: {center: [8.5, 5.5, 0], width: 15, height: 9, view_center: [6.25, 5.0, 0], scale: 1.0, space: paper}\n\
         \x20 - dimension: {kind: linear, points: [[0, 2, 0], [12.5, 2, 0]], location: [6.25, 5, 0], dimension_style: FRAC}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: dim_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let dim_dwg = dir.join("MM-DIM-INCH-TAB.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("dim-vp.dwg"),
        &dim_dwg,
        "InchTab",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
            paper_width_mm: Some(17.0 * 25.4),
            paper_height_mm: Some(11.0 * 25.4),
            plot_paper_units: Some(0),
            plot_scale: Some((1.0, 1.0)),
            ..Default::default()
        },
    );
    // Compute the exact world box of the metric `12` piece using the real
    // drawing pipeline (`measuring_drawing`) on `dim_dwg`, and confirm that
    // mutating `sheet.units` to Inch WOULD shift `12` up by ~0.667!
    let run_world_min = |dl: &crate::draw::DisplayList, text: &str| -> [f64; 2] {
        let run = dl
            .text
            .iter()
            .find(|run| run.characters.iter().map(|&(c, _)| c).collect::<String>() == text)
            .expect("expected text run");
        let [lo, _] = run.drawn.expect("drawn box");
        [
            run.origin[0] + lo[0] * run.across[0] + lo[1] * run.up[0],
            run.origin[1] + lo[0] * run.across[1] + lo[1] * run.up[1],
        ]
    };
    let read_ref = crate::read_dwg(&dim_dwg).unwrap();
    let mut ref_sheet = crate::model::Sheet {
        units: read_ref.units.unwrap(),
        block_library: None,
        linetypes: read_ref.linetypes,
        layers: read_ref.layers,
        text_styles: read_ref.text_styles,
        table_styles: read_ref.table_styles,
        mleader_styles: read_ref.mleader_styles,
        dimension_styles: read_ref.dimension_styles,
        blocks: read_ref.blocks,
        objects: read_ref.objects,
        paper: read_ref.paper,
        ltscale: read_ref.ltscale,
        layout: read_ref.layout,
        other_layouts: read_ref.other_layouts,
    };
    let ref_fonts = crate::text::placement::SheetFonts::find(&ref_sheet, &Settings::default());
    let metric_dl = crate::query::measuring_drawing(&ref_sheet, &ref_fonts);
    let [metric_12_min_x, metric_12_min_y] = run_world_min(&metric_dl, "12");
    ref_sheet.units = Unit::In;
    let imperial_dl = crate::query::measuring_drawing(&ref_sheet, &ref_fonts);
    let [_, imperial_12_min_y] = run_world_min(&imperial_dl, "12");
    assert!(
        (imperial_12_min_y - metric_12_min_y).abs() > 0.5,
        "stacked fraction alignment must differ between metric ({metric_12_min_y}) and imperial ({imperial_12_min_y})"
    );
    // Now run `check_with` on a DWG with paper-space text placed squarely over
    // the metric `12` baseline (and below imperial `12`): it must hit `12` at
    // `metric_12_min_y`, proving `check` did NOT mutate `sheet.units`!
    let hit_yaml = dir.join("dim-vp-hit.yaml");
    fs::write(
        &hit_yaml,
        format!(
            "units: mm\n\
             paper: ISO A3 landscape\n\
             layout: {{name: InchTab}}\n\
             dimension_styles:\n\
             \x20 FRAC:\n\
             \x20   text_height: 1.0\n\
             \x20   arrow_size: 0.2\n\
             \x20   unit_format: fractional\n\
             \x20   fraction_format: horizontal\n\
             \x20   precision: 4\n\
             \x20   placement: centered\n\
             objects:\n\
             \x20 - polyline: {{points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, space: paper}}\n\
             \x20 - viewport: {{center: [8.5, 5.5, 0], width: 15, height: 9, view_center: [6.25, 5.0, 0], scale: 1.0, space: paper}}\n\
             \x20 - dimension: {{kind: linear, points: [[0, 2, 0], [12.5, 2, 0]], location: [6.25, 5, 0], dimension_style: FRAC}}\n\
             \x20 - text: {{contents: \"X\", position: [{metric_12_min_x}, {y}, 0], height: 0.15, space: paper}}\n",
            y = metric_12_min_y + 0.05
        ),
    )
    .unwrap();
    build(&BuildRequest {
        description: hit_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let hit_dwg = dir.join("MM-DIM-INCH-HIT.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("dim-vp-hit.dwg"),
        &hit_dwg,
        "InchTab",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
            paper_width_mm: Some(17.0 * 25.4),
            paper_height_mm: Some(11.0 * 25.4),
            plot_paper_units: Some(0),
            plot_scale: Some((1.0, 1.0)),
            ..Default::default()
        },
    );
    let hit_checked = check_with(
        &CheckRequest {
            source: hit_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(hit_checked.units, Some(Unit::Mm));
    assert!((hit_checked.tolerance - 0.01).abs() < 1e-12);
    let tot = hit_checked
        .findings
        .iter()
        .find(|f| f.rule == Rule::TextOnText)
        .expect("paper-space text must collide with metric-aligned `12`");
    assert_eq!(tot.units, Some(Unit::In), "{tot:#?}");
    let other_min_y = tot.other_drawn.as_ref().unwrap().min[1];
    assert!(
        (other_min_y - metric_12_min_y).abs() < 1e-4,
        "expected metric `12` min_y {metric_12_min_y}, got {other_min_y} (imperial would be {imperial_12_min_y})"
    );

    // (b) Extents-plotted inch layout (`check_paper == None`, `plot_type = 1`,
    //     `plot_paper_units = 0`) in an `INSUNITS = mm` DWG:
    //     - `paper_size` and `off_sheet` are skipped in `checks_skipped`;
    //     - `text_on_text` STILL uses inch tolerance (`0.01 / 25.4 = 0.0003937 in`),
    //       catching a 0.005 in overlap (which would falsely pass under 0.01 mm!);
    //     - `text_on_line` STILL uses inch pen widths (`0.25 mm = 0.00984 in`,
    //       half-width `0.00492 in`), NOT `0.25 in`: a line 0.05 in below text
    //       does NOT trigger `text_on_line`, while a line 0.003 in below text DOES!
    let ext_yaml = dir.join("extents-inch.yaml");
    fs::write(
        &ext_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: ExtentsInch}\n\
         objects:\n\
         \x20 - text: {contents: \"A\", position: [1.0, 2.0, 0], height: 0.2, space: paper}\n\
         \x20 - text: {contents: \"B\", position: [1.0, 2.195, 0], height: 0.2, space: paper}\n\
         \x20 - text: {contents: \"C\", position: [5.0, 2.0, 0], height: 0.2, space: paper}\n\
         \x20 - line: {from: [4.8, 1.95, 0], to: [5.5, 1.95, 0], space: paper}\n\
         \x20 - text: {contents: \"D\", position: [8.0, 2.0, 0], height: 0.2, space: paper}\n\
         \x20 - line: {from: [7.8, 1.997, 0], to: [8.5, 1.997, 0], space: paper}\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: ext_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let ext_dwg = dir.join("EXTENTS-INCH.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("extents-inch.dwg"),
        &ext_dwg,
        "ExtentsInch",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
            paper_width_mm: Some(17.0 * 25.4),
            paper_height_mm: Some(11.0 * 25.4),
            plot_paper_units: Some(0), // inches
            plot_type: Some(1),        // extents -> check_paper == None
            plot_scale: Some((1.0, 1.0)),
            ..Default::default()
        },
    );
    let ext_checked = check_with(
        &CheckRequest {
            source: ext_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(ext_checked.units, Some(Unit::Mm));
    assert!((ext_checked.tolerance - 0.01).abs() < 1e-12);
    assert_eq!(ext_checked.checks_skipped.len(), 2);
    assert_eq!(ext_checked.checks_skipped[0].rule, Rule::PaperSize);
    assert_eq!(ext_checked.checks_skipped[1].rule, Rule::OffSheet);

    let ext_tot: Vec<&crate::check::Finding> = ext_checked
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOnText)
        .collect();
    assert_eq!(
        ext_tot.len(),
        1,
        "0.005 in overlap on extents-plotted inch layout must fail text_on_text: {:#?}",
        ext_checked.findings
    );
    assert_eq!(ext_tot[0].units, Some(Unit::In));

    let ext_tol: Vec<&crate::check::Finding> = ext_checked
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert_eq!(
        ext_tol.len(),
        1,
        "only the 0.003 in line (not the 0.05 in line) must trigger text_on_line with inch pen width: {:#?}",
        ext_tol
    );
    assert_eq!(ext_tol[0].units, Some(Unit::In));
    assert_eq!(
        ext_tol[0]
            .object
            .as_ref()
            .and_then(|o| o.text.as_deref()),
        Some("D"),
        "C's line is 0.05 in away and must NOT trigger text_on_line; D's line is 0.003 in away and MUST: {:#?}",
        ext_tol
    );

    // (c) Model-space overflowing table on an INSUNITS = mm DWG viewed through
    //     an inch layout: `text_overflow` reports `drawn`, `cell` and `past` in
    //     model-space mm, so `finding.units` MUST be `None` (matching
    //     `CheckReport.units == Some(Unit::Mm)` and message saying `mm`), while
    //     paper-space findings on that same inch layout STILL carry
    //     `finding.units == Some(Unit::In)`!
    let table_yaml = dir.join("mm-model-table-inch-tab.yaml");
    fs::write(
        &table_yaml,
        "units: mm\n\
         paper: ISO A3 landscape\n\
         layout: {name: InchTab}\n\
         objects:\n\
         \x20 - polyline: {points: [[0, 0, 0], [17.005, 0, 0], [17.005, 11, 0], [0, 11, 0]], closed: true, space: paper}\n\
         \x20 - viewport: {center: [8.5, 5.5, 0], width: 15, height: 9, view_center: [50, 50, 0], scale: 0.1, space: paper}\n\
         \x20 - table:\n\
         \x20     position: [20, 60, 0]\n\
         \x20     columns: [10]\n\
         \x20     rows: [8]\n\
         \x20     cells:\n\
         \x20       - [\"LONG OVERFLOWING MODEL TABLE TEXT\"]\n",
    )
    .unwrap();
    build(&BuildRequest {
        description: table_yaml,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let table_dwg = dir.join("MM-MODEL-TABLE-INCH-TAB.dwg");
    dwg::as_original_with_layout_setup(
        &dir.join("mm-model-table-inch-tab.dwg"),
        &table_dwg,
        "InchTab",
        false,
        &dwg::TestLayoutSetup {
            paper_size: Some("ANSI_B_(17.00_x_11.00_Inches)".to_owned()),
            paper_width_mm: Some(17.0 * 25.4),
            paper_height_mm: Some(11.0 * 25.4),
            plot_paper_units: Some(0),
            plot_scale: Some((1.0, 1.0)),
            ..Default::default()
        },
    );
    let table_checked = check_with(
        &CheckRequest {
            source: table_dwg,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(table_checked.units, Some(Unit::Mm));
    assert!((table_checked.tolerance - 0.01).abs() < 1e-12);
    let overflow = table_checked
        .findings
        .iter()
        .find(|f| f.rule == Rule::TextOverflow)
        .expect("model-space table must overflow");
    assert_eq!(
        overflow.units, None,
        "model-space text_overflow is in mm (matching CheckReport.units) and must NOT be tagged `in`: {overflow:#?}"
    );
    assert!(
        overflow.message.contains(" mm") && !overflow.message.contains(" in "),
        "{overflow:#?}"
    );
    let paper_off = table_checked
        .findings
        .iter()
        .find(|f| f.rule == Rule::OffSheet)
        .expect("oversized paper-space border must fail off_sheet");
    assert_eq!(
        paper_off.units,
        Some(Unit::In),
        "paper-space findings on the same inch layout must still carry `units: Some(Unit::In)`: {paper_off:#?}"
    );
}

fn desktop_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
}

/// Regenerates the three tiny 100%-made-up synthetic DWG fixtures used by
/// `viewer/e2e/run.mjs`'s desktop test (ADR-0008: zero real-plant material).
#[test]
#[ignore]
fn generate_desktop_e2e_synthetic_fixtures() {
    let fixtures = desktop_fixtures_dir();
    fs::create_dir_all(&fixtures).unwrap();
    let dir = scratch("gen-desktop-fixtures");
    let (base, _) = made_up_original(&dir);

    dwg::with_a_made_up_custom_object(
        &base,
        &fixtures.join("desktop-custom-undecoded-a.dwg"),
        "MADE_UP_VALVE",
    );
    dwg::with_a_made_up_custom_object(
        &base,
        &fixtures.join("desktop-custom-undecoded-b.dwg"),
        "MADE_UP_PUMP",
    );
    dwg::with_a_damaged_line(&base, &fixtures.join("desktop-damaged-line.dwg"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn desktop_e2e_synthetic_fixtures_decode_as_expected() {
    let fixtures = desktop_fixtures_dir();
    let a = fixtures.join("desktop-custom-undecoded-a.dwg");
    let b = fixtures.join("desktop-custom-undecoded-b.dwg");
    let damaged = fixtures.join("desktop-damaged-line.dwg");

    let ra = read_original(&ReadRequest {
        source: a,
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    let rb = read_original(&ReadRequest {
        source: b,
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(!ra.decoded);
    assert!(!rb.decoded);
    assert_ne!(ra.sha256, rb.sha256);
    assert!(ra.not_decoded.iter().any(|k| k.kind == "MADE_UP_VALVE"));
    assert!(rb.not_decoded.iter().any(|k| k.kind == "MADE_UP_PUMP"));

    let rd = read_original(&ReadRequest {
        source: damaged.clone(),
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(!rd.decoded);
    assert!(rd.read_past_damage.is_some());
    assert!(rd.not_decoded.iter().any(|k| k.kind == "LINE"));

    let mut dl_req = crate::DisplayListRequest::new(damaged);
    dl_req.read_past_damage = true;
    let dl =
        crate::display_list_with(&dl_req, &Settings::default().without_system_fonts()).unwrap();
    assert!(
        dl.objects.len() >= 2,
        "damaged-line fixture must render recoverable circle+polyline under read_past_damage: {dl:?}"
    );
}
