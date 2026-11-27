//! Leftover outputs (#109): every build and plot of a drawing set lists the
//! DWGs and PDFs AgentDrafter made for the set in its output folder that no
//! row writes any more, and moves them into `superseded/` when asked, by
//! `clean` for one run or `clean: true` in the set's settings. Nothing else
//! in the folder is ever listed or moved: a file without the build mark or
//! the stamp, an output of another set, and an original.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, build_with, plot_set_with, plot_with, BuildRequest, ErrorCode, Leftover,
    LeftoverKind, PlotRequest, PlotSetReport, PlotSetRequest, Reply, SetBuildRequest, SetReport,
    Settings, SUPERSEDED_FOLDER,
};

#[path = "support/demo_set.rs"]
mod demo_set;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-leftovers")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the demo plant's drawing set at `set`.
fn demo_set(set: &Path) -> PathBuf {
    demo_set::copy_demo_set(set);
    set.to_owned()
}

fn build(set: &Path, clean: Option<bool>) -> Reply<SetReport> {
    build_set_with(
        &SetBuildRequest {
            set: set.to_owned(),
            overwrite: true,
            clean,
        },
        &Settings::default(),
    )
}

fn plot(set: &Path, clean: Option<bool>) -> Reply<PlotSetReport> {
    plot_set_with(
        &PlotSetRequest {
            set: set.to_owned(),
            overwrite: true,
            clean,
            plot_style: None,
        },
        &Settings::default(),
    )
}

/// The set's leftovers as a build lists them, failing the test unless
/// every sheet was built.
fn built(set: &Path, clean: Option<bool>) -> Vec<Leftover> {
    let reply = build(set, clean);
    assert!(reply.ok, "{:#?}", reply.error);
    reply
        .result
        .unwrap()
        .leftovers
        .expect("the output folder was listed")
}

/// The set's leftovers as a plot lists them, failing the test unless
/// every sheet was plotted.
fn plotted(set: &Path, clean: Option<bool>) -> Vec<Leftover> {
    let reply = plot(set, clean);
    assert!(reply.ok, "{:#?}", reply.error);
    reply
        .result
        .unwrap()
        .leftovers
        .expect("the output folder was listed")
}

/// Takes the row of `sheet` out of the set's rows file.
fn take_out(set: &Path, sheet: &str) {
    let rows = set.join("rows.yaml");
    let text = fs::read_to_string(&rows).unwrap();
    let start = text
        .find(&format!("  - sheet: {sheet}\n"))
        .unwrap_or_else(|| panic!("a row of {sheet}"));
    let end = text[start + 4..]
        .find("  - sheet: ")
        .map_or(text.len(), |at| start + 4 + at);
    fs::write(&rows, format!("{}{}", &text[..start], &text[end..])).unwrap();
}

/// The files directly in `dir`, by name, in byte order.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Each leftover as its file's name, its kind and its sheet.
fn named(leftovers: &[Leftover]) -> Vec<(String, LeftoverKind, String)> {
    leftovers
        .iter()
        .map(|l| {
            let name = Path::new(&l.file).file_name().unwrap();
            (name.to_string_lossy().into_owned(), l.kind, l.sheet.clone())
        })
        .collect()
}

fn listed(sheet: &str) -> Vec<(String, LeftoverKind, String)> {
    vec![
        (format!("{sheet}.dwg"), LeftoverKind::Dwg, sheet.to_owned()),
        (format!("{sheet}.pdf"), LeftoverKind::Pdf, sheet.to_owned()),
    ]
}

#[test]
fn a_row_taken_out_leaves_its_dwg_and_pdf_listed_by_build_and_plot() {
    let set = demo_set(&scratch("listed").join("demo-set"));
    assert_eq!(built(&set, None), []);
    assert_eq!(plotted(&set, None), []);
    take_out(&set, "DEMO-003");
    let dwg = set.join("dwg");
    let before = fs::read(dwg.join("DEMO-003.dwg")).unwrap();

    let by_build = built(&set, None);
    assert_eq!(named(&by_build), listed("DEMO-003"));
    assert!(by_build
        .iter()
        .all(|l| l.moved_to.is_none() && l.error.is_none()));
    assert_eq!(by_plot_names(&set, None), listed("DEMO-003"));
    // Only listed: both are still there, as they were.
    assert_eq!(fs::read(dwg.join("DEMO-003.dwg")).unwrap(), before);
    assert!(dwg.join("DEMO-003.pdf").is_file());
    assert!(!dwg.join(SUPERSEDED_FOLDER).exists());

    // The reply names the key, with each file's path in the output folder.
    let json = serde_json::to_value(&by_build).unwrap();
    assert_eq!(json[0]["kind"], "dwg");
    assert_eq!(json[0]["sheet"], "DEMO-003");
    assert!(json[0].get("moved_to").is_none());
    assert_eq!(
        Path::new(json[0]["file"].as_str().unwrap()),
        dwg.join("DEMO-003.dwg")
    );
}

fn by_plot_names(set: &Path, clean: Option<bool>) -> Vec<(String, LeftoverKind, String)> {
    named(&plotted(set, clean))
}

#[test]
fn clean_moves_them_into_superseded_keeping_their_names() {
    let set = demo_set(&scratch("clean").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    let dwg = set.join("dwg");
    let (old_dwg, old_pdf) = (
        fs::read(dwg.join("DEMO-002.dwg")).unwrap(),
        fs::read(dwg.join("DEMO-002.pdf")).unwrap(),
    );
    take_out(&set, "DEMO-002");

    let cleaned = built(&set, Some(true));
    assert_eq!(named(&cleaned), listed("DEMO-002"));
    let superseded = dwg.join(SUPERSEDED_FOLDER);
    for leftover in &cleaned {
        let name = Path::new(&leftover.file).file_name().unwrap();
        assert_eq!(
            Path::new(leftover.moved_to.as_deref().unwrap()),
            superseded.join(name)
        );
    }
    assert_eq!(files_in(&superseded), ["DEMO-002.dwg", "DEMO-002.pdf"]);
    assert_eq!(fs::read(superseded.join("DEMO-002.dwg")).unwrap(), old_dwg);
    assert_eq!(fs::read(superseded.join("DEMO-002.pdf")).unwrap(), old_pdf);
    // The folder holds exactly the set's sheets, and nothing is left over.
    assert_eq!(
        files_in(&dwg),
        [
            "DEMO-001.dwg",
            "DEMO-001.pdf",
            "DEMO-003.dwg",
            "DEMO-003.pdf"
        ]
    );
    assert_eq!(built(&set, None), []);

    // A plot cleans as a build does.
    take_out(&set, "DEMO-003");
    let by_plot = plotted(&set, Some(true));
    assert_eq!(named(&by_plot), listed("DEMO-003"));
    assert!(by_plot.iter().all(|l| l.moved_to.is_some()));
    assert_eq!(
        files_in(&superseded),
        [
            "DEMO-002.dwg",
            "DEMO-002.pdf",
            "DEMO-003.dwg",
            "DEMO-003.pdf"
        ]
    );
    assert_eq!(files_in(&dwg), ["DEMO-001.dwg", "DEMO-001.pdf"]);
}

#[test]
fn the_setting_cleans_unless_a_run_says_otherwise() {
    let set = demo_set(&scratch("setting").join("demo-set"));
    let settings = set.join("set.yaml");
    fs::write(
        &settings,
        format!("{}clean: true\n", fs::read_to_string(&settings).unwrap()),
    )
    .unwrap();
    built(&set, None);
    plotted(&set, None);
    let dwg = set.join("dwg");
    let superseded = dwg.join(SUPERSEDED_FOLDER);

    // `clean: false` for one run only lists them.
    take_out(&set, "DEMO-003");
    let listed_only = built(&set, Some(false));
    assert_eq!(named(&listed_only), listed("DEMO-003"));
    assert!(listed_only.iter().all(|l| l.moved_to.is_none()));
    assert!(dwg.join("DEMO-003.dwg").is_file());
    assert!(!superseded.exists());

    // Without a word from the run, the setting cleans.
    let cleaned = built(&set, None);
    assert!(cleaned.iter().all(|l| l.moved_to.is_some()));
    assert_eq!(files_in(&superseded), ["DEMO-003.dwg", "DEMO-003.pdf"]);

    // `clean: false` in the settings is the same as leaving it out.
    fs::write(
        &settings,
        fs::read_to_string(&settings)
            .unwrap()
            .replace("clean: true", "clean: false"),
    )
    .unwrap();
    take_out(&set, "DEMO-002");
    assert!(plotted(&set, None).iter().all(|l| l.moved_to.is_none()));
    assert!(dwg.join("DEMO-002.dwg").is_file());

    // A setting that is not `true` or `false` is refused, before anything
    // is built.
    fs::write(
        &settings,
        fs::read_to_string(&settings)
            .unwrap()
            .replace("clean: false", "clean: yes"),
    )
    .unwrap();
    let refused = build(&set, None);
    assert!(!refused.ok);
    assert!(refused.result.is_none());
    assert_eq!(refused.error.unwrap().field.as_deref(), Some("clean"));
}

#[test]
fn a_file_without_the_mark_or_the_stamp_or_from_another_set_is_never_touched() {
    let dir = scratch("others");
    let jobs = dir.join("jobs");
    // Two sets share one output folder, `issued`, beside them: each one's
    // DWGs name its own set, and each one's PDFs its own rows file.
    let ours = demo_set(&jobs.join("ours"));
    let theirs = demo_set(&jobs.join("theirs"));
    for (set, prefix) in [(&ours, "DEMO-"), (&theirs, "OTHER-")] {
        let settings = set.join("set.yaml");
        let text = fs::read_to_string(&settings).unwrap();
        fs::write(&settings, text.replace("output: dwg", "output: ../issued")).unwrap();
        let rows = set.join("rows.yaml");
        let text = fs::read_to_string(&rows).unwrap();
        fs::write(&rows, text.replace("DEMO-", prefix)).unwrap();
    }
    built(&theirs, None);
    plotted(&theirs, None);
    assert_eq!(built(&ours, None), []);
    assert_eq!(plotted(&ours, None), []);
    let issued = jobs.join("issued");

    // In the folder too: a DWG and a PDF of a sheet description built and
    // plotted on its own, whose mark and stamp name no set; a DWG that is
    // not a build; and a PDF that carries no stamp.
    let lone = dir.join("lone.yaml");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/one-line.yaml"),
        &lone,
    )
    .unwrap();
    let alone = build_with(
        &BuildRequest {
            description: lone.clone(),
            output: Some(issued.join("LONE.dwg")),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(alone.is_ok(), "{alone:#?}");
    let mut request = PlotRequest::new(&lone);
    request.output = Some(issued.join("LONE.pdf"));
    assert!(plot_with(&request, &Settings::default()).is_ok());
    fs::write(issued.join("NOTES.dwg"), b"someone else's drawing").unwrap();
    fs::write(issued.join("NOTES.pdf"), b"%PDF-1.7\n% not a plot\n").unwrap();

    let before: Vec<(String, Vec<u8>)> = files_in(&issued)
        .into_iter()
        .map(|name| {
            let bytes = fs::read(issued.join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    // Our set lists none of them, and cleaning moves none of them.
    assert_eq!(built(&ours, Some(true)), []);
    assert_eq!(plotted(&ours, Some(true)), []);
    // Our own sheet taken out is the one leftover, and the one moved.
    take_out(&ours, "DEMO-001");
    assert_eq!(named(&built(&ours, Some(true))), listed("DEMO-001"));
    assert_eq!(
        files_in(&issued.join(SUPERSEDED_FOLDER)),
        ["DEMO-001.dwg", "DEMO-001.pdf"]
    );
    for (name, bytes) in before {
        if name.starts_with("DEMO-001.") {
            continue;
        }
        assert_eq!(fs::read(issued.join(&name)).unwrap(), bytes, "{name}");
    }
}

#[test]
fn an_original_is_never_touched() {
    let set = demo_set(&scratch("original").join("demo-set"));
    built(&set, None);
    let dwg = set.join("dwg");
    // A plant's original DWG, which no build wrote and which carries no
    // build mark, under the name of a sheet the rows no longer write.
    let original = acadrust::DwgWriter::write_to_vec(&acadrust::CadDocument::new()).unwrap();
    fs::write(dwg.join("PLANT-100.dwg"), &original).unwrap();
    take_out(&set, "DEMO-003");
    fs::remove_file(dwg.join("DEMO-003.dwg")).unwrap();
    fs::write(dwg.join("DEMO-003.dwg"), &original).unwrap();

    assert_eq!(built(&set, Some(true)), []);
    assert_eq!(plotted(&set, Some(true)), []);
    assert_eq!(fs::read(dwg.join("PLANT-100.dwg")).unwrap(), original);
    assert_eq!(fs::read(dwg.join("DEMO-003.dwg")).unwrap(), original);
    assert!(!dwg.join(SUPERSEDED_FOLDER).exists());
}

#[test]
fn a_name_already_in_superseded_is_kept_and_the_leftover_takes_the_next_number() {
    let set = demo_set(&scratch("clash").join("demo-set"));
    let rows = set.join("rows.yaml");
    let all_rows = fs::read_to_string(&rows).unwrap();
    let superseded = set.join("dwg").join(SUPERSEDED_FOLDER);
    let mut taken = Vec::new();
    // DEMO-003 is taken out and cleaned three times, at three revisions.
    for rev in ["A", "B", "C"] {
        fs::write(&rows, all_rows.replace("REV: A", &format!("REV: {rev}"))).unwrap();
        built(&set, None);
        taken.push(fs::read(set.join("dwg").join("DEMO-003.dwg")).unwrap());
        take_out(&set, "DEMO-003");
        let cleaned = built(&set, Some(true));
        assert_eq!(cleaned.len(), 1, "{cleaned:#?}");
        assert!(cleaned[0].moved_to.is_some());
    }
    assert_eq!(
        files_in(&superseded),
        ["DEMO-003 (2).dwg", "DEMO-003 (3).dwg", "DEMO-003.dwg"]
    );
    assert_eq!(fs::read(superseded.join("DEMO-003.dwg")).unwrap(), taken[0]);
    assert_eq!(
        fs::read(superseded.join("DEMO-003 (2).dwg")).unwrap(),
        taken[1]
    );
    assert_eq!(
        fs::read(superseded.join("DEMO-003 (3).dwg")).unwrap(),
        taken[2]
    );
    assert_ne!(taken[0], taken[1]);
}

#[test]
fn a_leftover_that_cannot_be_moved_stays_and_the_reply_says_so() {
    let set = demo_set(&scratch("stuck").join("demo-set"));
    built(&set, None);
    take_out(&set, "DEMO-003");
    // A file, not a folder, where `superseded` would be.
    let dwg = set.join("dwg");
    fs::write(dwg.join(SUPERSEDED_FOLDER), b"in the way").unwrap();
    let reply = build(&set, Some(true));
    assert!(!reply.ok);
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::WriteFailed);
    assert!(error.message.contains("leftovers"), "{}", error.message);
    let leftovers = reply.result.unwrap().leftovers.unwrap();
    assert_eq!(leftovers.len(), 1);
    assert!(leftovers[0].moved_to.is_none());
    assert_eq!(
        leftovers[0].error.as_ref().unwrap().code,
        ErrorCode::WriteFailed
    );
    assert!(dwg.join("DEMO-003.dwg").is_file());
}

/// Whether the file system at `dir` tells names apart by letter case.
fn tells_case_apart(dir: &Path) -> bool {
    let probe = dir.join("Case-Probe.txt");
    fs::write(&probe, b"").unwrap();
    let apart = !dir.join("case-probe.txt").exists();
    fs::remove_file(&probe).unwrap();
    apart
}

/// A row renamed in letter case writes its outputs under the new name.
/// Where the file system tells the two names apart, the old files are
/// other files, and left over; where it does not, they are the row's own
/// files, and stay where they are, current.
#[test]
fn a_sheet_renamed_in_letter_case_is_left_over_only_where_the_names_are_two_files() {
    let set = demo_set(&scratch("case").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    let dwg = set.join("dwg");
    let apart = tells_case_apart(&dwg);
    let rows = set.join("rows.yaml");
    let text = fs::read_to_string(&rows).unwrap();
    fs::write(
        &rows,
        text.replace("  - sheet: DEMO-002\n", "  - sheet: demo-002\n"),
    )
    .unwrap();

    let cleaned = built(&set, Some(true));
    assert!(dwg.join("demo-002.dwg").is_file());
    if apart {
        assert_eq!(named(&cleaned), listed("DEMO-002"));
        assert!(cleaned.iter().all(|l| l.moved_to.is_some()));
        assert_eq!(
            files_in(&dwg.join(SUPERSEDED_FOLDER)),
            ["DEMO-002.dwg", "DEMO-002.pdf"]
        );
    } else {
        assert_eq!(cleaned, []);
        assert!(!dwg.join(SUPERSEDED_FOLDER).exists());
        // The PDF, still under its old name, is the row's own.
        assert!(dwg.join("demo-002.pdf").is_file());
        assert_eq!(plotted(&set, Some(true)), []);
    }
}

/// Makes `link` a link to the folder `to`: a symbolic link on Unix, a
/// junction on Windows, which needs no privilege.
fn link_folder(to: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(to, link).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(to)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }
}

#[test]
fn a_superseded_folder_that_is_a_link_or_a_junction_is_refused_and_nothing_moves() {
    let dir = scratch("linked");
    let set = demo_set(&dir.join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    take_out(&set, "DEMO-003");
    let elsewhere = dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let dwg = set.join("dwg");
    link_folder(&elsewhere, &dwg.join(SUPERSEDED_FOLDER));

    let reply = build(&set, Some(true));
    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::WriteFailed);
    let leftovers = reply.result.unwrap().leftovers.unwrap();
    assert_eq!(named(&leftovers), listed("DEMO-003"));
    for leftover in &leftovers {
        assert!(leftover.moved_to.is_none());
        let error = leftover.error.as_ref().unwrap();
        assert!(
            error.message.contains("link or a junction"),
            "{}",
            error.message
        );
    }
    assert!(dwg.join("DEMO-003.dwg").is_file());
    assert!(dwg.join("DEMO-003.pdf").is_file());
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

/// A leftover no one may read is listed as unreadable and never moved,
/// and an output folder that cannot be listed leaves no list at all: the
/// reply says the scan did not finish. Unix alone takes a file's reading
/// away from its owner; as root, who reads everything, it is not shown.
#[cfg(unix)]
#[test]
fn a_scan_that_cannot_finish_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let set = demo_set(&scratch("unread").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    take_out(&set, "DEMO-003");
    let dwg = set.join("dwg");
    let pdf = dwg.join("DEMO-003.pdf");
    let mode = |path: &Path, mode: u32| {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    };
    mode(&pdf, 0o000);
    if fs::read(&pdf).is_ok() {
        mode(&pdf, 0o644);
        eprintln!("run as root, which reads every file: not shown");
        return;
    }
    let reply = build(&set, Some(true));
    mode(&pdf, 0o644);
    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::ReadFailed);
    let leftovers = reply.result.unwrap().leftovers.unwrap();
    assert_eq!(named(&leftovers), listed("DEMO-003"));
    assert!(leftovers[0].moved_to.is_some() && !leftovers[0].unreadable);
    assert!(leftovers[1].unreadable && leftovers[1].moved_to.is_none());
    assert_eq!(
        leftovers[1].error.as_ref().unwrap().code,
        ErrorCode::ReadFailed
    );
    assert!(pdf.is_file());
    let json = serde_json::to_value(&leftovers).unwrap();
    assert_eq!(json[1]["unreadable"], true);
    assert!(json[0].get("unreadable").is_none());

    // Written to, but not listed.
    mode(&dwg, 0o333);
    let reply = build(&set, None);
    mode(&dwg, 0o755);
    assert!(!reply.ok);
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert_eq!(
        error.file.as_deref(),
        Some(dwg.display().to_string().as_str())
    );
    let report = reply.result.unwrap();
    assert_eq!(report.built, 2);
    assert!(report.leftovers.is_none());
    assert_eq!(report.leftovers_error.unwrap().code, ErrorCode::ReadFailed);
    let json = serde_json::to_value(build(&set, None).result.unwrap()).unwrap();
    assert!(json["leftovers"].is_array(), "listed again once it can be");
}

/// A leftover DWG cut short, which starts as a DWG of the 2018 format but
/// whose drawing properties cannot be read, may or may not carry the mark:
/// it is listed as unreadable, never moved, and never dropped.
#[test]
fn a_dwg_whose_mark_cannot_be_read_is_listed_as_unreadable() {
    let set = demo_set(&scratch("cut-short").join("demo-set"));
    built(&set, None);
    take_out(&set, "DEMO-003");
    let dwg = set.join("dwg").join("DEMO-003.dwg");
    let bytes = fs::read(&dwg).unwrap();
    fs::write(&dwg, &bytes[..256]).unwrap();

    let reply = build(&set, Some(true));
    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::ReadFailed);
    let leftovers = reply.result.unwrap().leftovers.unwrap();
    assert_eq!(leftovers.len(), 1, "{leftovers:#?}");
    assert!(leftovers[0].unreadable);
    assert_eq!(leftovers[0].sheet, "DEMO-003");
    assert!(leftovers[0].moved_to.is_none());
    assert_eq!(fs::read(&dwg).unwrap(), &bytes[..256]);
    assert!(!set.join("dwg").join(SUPERSEDED_FOLDER).exists());
}

/// On Windows, holding a sheet's leftover PDF open without delete sharing
/// (as a PDF viewer does) prevents moving it; the sheet's DWG must stay with
/// it rather than moving to `superseded/` alone (#236), while another sheet's
/// unlocked pair still cleans.
#[cfg(windows)]
#[test]
fn a_locked_pdf_keeps_its_sheets_dwg_from_splitting_into_superseded() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;

    let set = demo_set(&scratch("locked-pdf").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    let dwg_dir = set.join("dwg");
    let dwg2_bytes = fs::read(dwg_dir.join("DEMO-002.dwg")).unwrap();
    let pdf2_bytes = fs::read(dwg_dir.join("DEMO-002.pdf")).unwrap();

    take_out(&set, "DEMO-002");
    take_out(&set, "DEMO-003");

    // Hold DEMO-002.pdf open with read-only sharing (no FILE_SHARE_DELETE).
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(dwg_dir.join("DEMO-002.pdf"))
        .unwrap();

    let reply = build(&set, Some(true));
    drop(_held);

    assert!(!reply.ok);
    assert_eq!(reply.error.unwrap().code, ErrorCode::WriteFailed);
    let leftovers = reply.result.unwrap().leftovers.unwrap();
    let superseded = dwg_dir.join(SUPERSEDED_FOLDER);

    // DEMO-002 stayed together in dwg/ and nothing of DEMO-002 is in superseded/.
    assert_eq!(fs::read(dwg_dir.join("DEMO-002.dwg")).unwrap(), dwg2_bytes);
    assert_eq!(fs::read(dwg_dir.join("DEMO-002.pdf")).unwrap(), pdf2_bytes);
    assert!(!superseded.join("DEMO-002.dwg").exists());
    assert!(!superseded.join("DEMO-002.pdf").exists());
    for l in leftovers.iter().filter(|l| l.sheet == "DEMO-002") {
        assert!(l.moved_to.is_none(), "{l:#?}");
        assert!(l.error.is_some(), "{l:#?}");
    }

    // DEMO-003 (unlocked) moved cleanly as a complete pair.
    assert!(!dwg_dir.join("DEMO-003.dwg").exists());
    assert!(!dwg_dir.join("DEMO-003.pdf").exists());
    assert_eq!(files_in(&superseded), ["DEMO-003.dwg", "DEMO-003.pdf"]);
    for l in leftovers.iter().filter(|l| l.sheet == "DEMO-003") {
        assert!(l.moved_to.is_some() && l.error.is_none(), "{l:#?}");
    }
}

/// Concurrent builds and plots with `clean: true` serialize on the output
/// folder lock so neither removes or moves a current output published by the
/// other (#238).
#[test]
fn concurrent_builds_and_plots_with_clean_preserve_all_current_outputs() {
    let set = demo_set(&scratch("concurrent-clean").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    take_out(&set, "DEMO-003");

    let mut handles = Vec::new();
    for i in 0..4 {
        let set_path = set.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..2 {
                if i % 2 == 0 {
                    let reply = build(&set_path, Some(true));
                    assert!(reply.ok, "build failed: {:#?}", reply.error);
                } else {
                    let reply = plot(&set_path, Some(true));
                    assert!(reply.ok, "plot failed: {:#?}", reply.error);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let dwg = set.join("dwg");
    assert_eq!(
        files_in(&dwg),
        [
            "DEMO-001.dwg",
            "DEMO-001.pdf",
            "DEMO-002.dwg",
            "DEMO-002.pdf"
        ]
    );
    assert_eq!(
        files_in(&dwg.join(SUPERSEDED_FOLDER)),
        ["DEMO-003.dwg", "DEMO-003.pdf"]
    );
}

/// On Windows, a read-only leftover DWG and PDF move cleanly into `superseded/`
/// (staying read-only there with no copy left in `dwg/`), and if one sibling is
/// held open the read-only sibling rolls back cleanly without leaving a second
/// copy in `superseded/` (#239).
#[cfg(windows)]
#[test]
fn a_read_only_leftover_moves_cleanly_and_rolls_back_cleanly() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;

    let set = demo_set(&scratch("readonly-win").join("demo-set"));
    built(&set, None);
    plotted(&set, None);
    let dwg_dir = set.join("dwg");
    take_out(&set, "DEMO-002");
    take_out(&set, "DEMO-003");

    let set_ro = |p: &Path, ro: bool| {
        let mut perms = fs::metadata(p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(ro);
        fs::set_permissions(p, perms).unwrap();
    };
    set_ro(&dwg_dir.join("DEMO-002.dwg"), true);
    set_ro(&dwg_dir.join("DEMO-003.dwg"), true);
    set_ro(&dwg_dir.join("DEMO-003.pdf"), true);

    // Hold DEMO-002.pdf open so DEMO-002.dwg (read-only) must roll back, while
    // DEMO-003 (both read-only) moves cleanly.
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(dwg_dir.join("DEMO-002.pdf"))
        .unwrap();

    let reply = build(&set, Some(true));
    drop(_held);

    assert!(!reply.ok);
    let sup = dwg_dir.join(SUPERSEDED_FOLDER);
    assert_eq!(files_in(&sup), ["DEMO-003.dwg", "DEMO-003.pdf"]);
    assert!(fs::metadata(sup.join("DEMO-003.dwg"))
        .unwrap()
        .permissions()
        .readonly());
    assert!(fs::metadata(sup.join("DEMO-003.pdf"))
        .unwrap()
        .permissions()
        .readonly());
    assert!(!dwg_dir.join("DEMO-003.dwg").exists());
    assert!(!dwg_dir.join("DEMO-003.pdf").exists());
    assert!(dwg_dir.join("DEMO-002.dwg").is_file());
    assert!(fs::metadata(dwg_dir.join("DEMO-002.dwg"))
        .unwrap()
        .permissions()
        .readonly());
    assert!(!sup.join("DEMO-002.dwg").exists());

    // Clear read-only before scratch teardown on future runs.
    set_ro(&dwg_dir.join("DEMO-002.dwg"), false);
    set_ro(&sup.join("DEMO-003.dwg"), false);
    set_ro(&sup.join("DEMO-003.pdf"), false);
}
