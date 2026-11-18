//! A display list's object snaps, as the viewer finds the nearest one with
//! no round trip: on every sheet of the demo plant, they are the points
//! `query snap` gives for the same object, snap and `which`, and `with` for
//! an intersection, every one of them and no more; and a display list asked
//! for without them is as it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    display_list_with, query_with, Answer, DisplayListRequest, ObjectRef, Query, QueryRequest,
    Settings, SnapKind, SnapRef,
};

fn demo_plant() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-plant")
}

/// No font folders: text is measured with the look-alikes alone, as the
/// demo plant is held to the same bytes on every system.
fn settings() -> Settings {
    Settings::default()
}

#[test]
fn every_snap_a_display_list_gives_is_the_point_query_snap_gives() {
    let source = demo_plant();
    let sheets = display_list_with(&DisplayListRequest::new(&source), &settings())
        .unwrap()
        .sheets;
    assert_eq!(sheets.len(), 8);
    let mut checked = 0;
    let mut kinds = BTreeSet::new();
    for sheet in &sheets {
        let mut request = DisplayListRequest::new(&source);
        request.sheet = Some(sheet.clone());
        request.snaps = true;
        let report = display_list_with(&request, &settings()).unwrap();
        let snaps = report.snaps.as_ref().expect("snaps were asked for");
        assert!(snaps.len() > 100, "{sheet}: {} snaps", snaps.len());
        let id = |place: usize| {
            report.objects[place]
                .id
                .clone()
                .unwrap_or_else(|| panic!("{sheet}: object {place} has an ID"))
        };
        // Asked of `query snap` once for each object the page draws itself,
        // with no snap, which gives every point of its own snaps; and once
        // for each pair an intersection is listed for. Without `which`, every
        // point of the kind.
        let mut asked: BTreeMap<(usize, Option<usize>), Vec<_>> = report
            .display_list
            .items
            .iter()
            .filter(|item| item.viewport.is_none())
            .map(|item| ((item.object, None), Vec::new()))
            .collect();
        for snap in snaps {
            kinds.insert(snap.snap.name());
            asked
                .entry((snap.object, snap.with))
                .or_default()
                .push(snap);
        }
        for ((object, with), given) in &asked {
            let question = SnapRef {
                id: Some(id(*object)),
                snap: with.map(|_| SnapKind::Intersection),
                with: with.map(|w| ObjectRef {
                    id: Some(id(w)),
                    name: None,
                }),
                ..SnapRef::default()
            };
            let answer = query_with(
                &QueryRequest {
                    source: source.clone(),
                    sheet: Some(sheet.clone()),
                    query: Query::Snap(question.clone()),
                },
                &settings(),
            )
            .unwrap_or_else(|e| panic!("{sheet}: {question:?}: {e:#?}"));
            let Answer::Snap { points, .. } = &answer.answer else {
                panic!("not a snap: {answer:#?}");
            };
            // The same points, each named by the same `which`, in the same
            // place: none left out, none added.
            let found: Vec<(Option<String>, _)> =
                points.iter().map(|p| (p.which.clone(), p.at)).collect();
            let listed: Vec<(Option<String>, _)> =
                given.iter().map(|s| (s.which.clone(), s.at)).collect();
            assert_eq!(listed, found, "{sheet}: {question:?}");
            checked += listed.len();
        }
    }
    // The demo plant has every snap a loop sheet's objects have.
    for kind in [
        "endpoint",
        "midpoint",
        "center",
        "quadrant",
        "insertion",
        "intersection",
    ] {
        assert!(kinds.contains(kind), "no {kind} among {kinds:?}");
    }
    assert!(checked > 1000, "{checked} snaps checked");
    assert!(!kinds.contains(SnapKind::Nearest.name()));
}

#[test]
fn a_display_list_without_snaps_says_nothing_of_them() {
    let mut request = DisplayListRequest::new(demo_plant());
    request.sheet = Some("DP-LD-101".to_owned());
    let report = display_list_with(&request, &settings()).unwrap();
    assert!(report.snaps.is_none());
    let json = serde_json::to_value(&report).unwrap();
    assert!(
        json.get("snaps").is_none(),
        "no `snaps` key without the option"
    );
    // The request without it is written as before, with no `snaps` key.
    let asked = serde_json::to_value(&request).unwrap();
    assert!(asked.get("snaps").is_none());
}
