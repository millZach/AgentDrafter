//! A display list's object snaps, as the viewer finds the nearest one with
//! no round trip: on every sheet of the demo plant, they are the points
//! `query snap` gives for the same object, snap and `which`, and `with` for
//! an intersection, every one of them and no more, and on a made-up sheet
//! of dotted and dashed curves every crossing `query snap` finds between
//! any two objects; only what the page draws has snaps; and a display list
//! asked for without them is as it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    display_list_with, nearest, query_with, Answer, DisplayListReport, DisplayListRequest,
    ObjectRef, Query, QueryRequest, Settings, SnapKind, SnapRef,
};

/// The places in its blocks a snap's `which` passes through:
/// `objects[3].objects[1].to` is `[3, 1]`.
fn chain_of(path: &str) -> Vec<usize> {
    path.split('.')
        .map_while(|step| {
            step.strip_prefix("objects[")?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect()
}

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
        // What of each insert's block the page draws: `[3, 1]` for the
        // object 1 of the block nested at 3. `query snap`, asked of an object
        // by its ID, also answers for what it does not draw, such as an
        // attribute with no value; the page has no snaps there.
        let mut chains: BTreeSet<(usize, Vec<usize>)> = BTreeSet::new();
        for item in report
            .display_list
            .items
            .iter()
            .filter(|i| i.viewport.is_none())
        {
            if let Some(part) = item.part {
                let mut chain = vec![part];
                chain.extend(&item.nested);
                chains.insert((item.object, chain));
            }
        }
        let drawn = |object: usize, place: Option<&str>| {
            let chain = chain_of(place.unwrap_or(""));
            chain.is_empty() || chains.contains(&(object, chain))
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
            let found: Vec<(Option<String>, _)> = points
                .iter()
                .filter(|p| match with {
                    None => drawn(*object, p.which.as_deref()),
                    Some(w) => {
                        drawn(*object, p.part.as_deref()) && drawn(*w, p.with_part.as_deref())
                    }
                })
                .map(|p| (p.which.clone(), p.at))
                .collect();
            let listed: Vec<(Option<String>, _)> =
                given.iter().map(|s| (s.which.clone(), s.at)).collect();
            let missing: Vec<_> = found.iter().filter(|p| !listed.contains(p)).collect();
            let extra: Vec<_> = listed.iter().filter(|p| !found.contains(p)).collect();
            assert_eq!(
                listed, found,
                "{sheet}: {question:?}: missing {missing:?}, extra {extra:?}"
            );
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
    eprintln!("{checked} snaps checked on the demo plant");
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

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-display-snaps")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The display list of `source` with its snaps, of its layout `layout`.
fn with_snaps(source: &Path, layout: Option<&str>) -> DisplayListReport {
    let mut request = DisplayListRequest::new(source);
    request.snaps = true;
    request.layout = layout.map(str::to_owned);
    display_list_with(&request, &settings()).unwrap()
}

/// The place of the object whose ID ends `/<id>`.
fn place_of(report: &DisplayListReport, id: &str) -> usize {
    let end = format!("/{id}");
    report
        .objects
        .iter()
        .position(|o| o.id.as_deref().is_some_and(|i| i.ends_with(&end)))
        .unwrap_or_else(|| panic!("no {id}"))
}

/// Curves drawn dotted and dashed, whose drawn dots and dashes miss where
/// they cross, among others drawn whole, a polyline with width, which draws
/// only a fill, and a solid hatch.
const DOTTED: &str = "units: mm
linetypes:
  DOT: {pattern: [0, -30]}
  DASH: {pattern: [3, -27]}
objects:
  - circle: {id: ring, center: [0, 0, 0], radius: 10, linetype: DOT}
  - line: {id: chord, from: [-12, 5, 0], to: [12, 5, 0]}
  - arc: {id: bow, center: [30, 0, 0], radius: 8, start_angle: 0, end_angle: 180, linetype: DASH}
  - line: {id: across, from: [20, 4, 0], to: [40, 4, 0], linetype: DOT}
  - ellipse: {id: oval, center: [60, 0, 0], major_axis: [10, 0, 0], ratio: 0.5, linetype: DASH}
  - polyline: {id: zigzag, points: [[50, -8, 0], [55, 8, 0], [60, -8, 0], [65, 8, 0], [70, -8, 0]], linetype: DOT}
  - spline: {id: wave, fit_points: [[45, 2, 0], [60, -6, 0], [75, 2, 0]], linetype: DASH}
  - polyline: {id: wide, points: [{point: [0, 40, 0], start_width: 2, end_width: 2}, [20, 40, 0]]}
  - line: {id: over-wide, from: [5, 35, 0], to: [5, 45, 0], linetype: DASH}
  - hatch: {id: fill, pattern: SOLID, boundary: [{polyline: {points: [[80, 30, 0], [90, 30, 0], [90, 40, 0], [80, 40, 0]]}}]}
";

#[test]
fn every_crossing_query_snap_finds_the_display_list_lists_however_it_is_dotted() {
    let dir = scratch("dotted");
    let source = dir.join("dotted.yaml");
    std::fs::write(&source, DOTTED).unwrap();
    let report = with_snaps(&source, None);
    let snaps = report.snaps.as_ref().unwrap();
    let drawn: BTreeSet<usize> = report
        .display_list
        .items
        .iter()
        .map(|item| item.object)
        .collect();
    let id = |place: usize| report.objects[place].id.clone().unwrap();
    // Every pair of objects drawn, asked of `query snap`, not only those
    // the display list names: the same crossings, none missed.
    let mut crossings = 0;
    for &i in &drawn {
        for &j in drawn.iter().filter(|&&j| j > i) {
            let question = SnapRef {
                id: Some(id(i)),
                snap: Some(SnapKind::Intersection),
                with: Some(ObjectRef {
                    id: Some(id(j)),
                    name: None,
                }),
                ..SnapRef::default()
            };
            let answer = query_with(
                &QueryRequest {
                    source: source.clone(),
                    sheet: None,
                    query: Query::Snap(question.clone()),
                },
                &settings(),
            )
            .unwrap_or_else(|e| panic!("{question:?}: {e:#?}"));
            let Answer::Snap { points, .. } = &answer.answer else {
                panic!("not a snap: {answer:#?}");
            };
            let found: Vec<(Option<String>, _)> =
                points.iter().map(|p| (p.which.clone(), p.at)).collect();
            let listed: Vec<(Option<String>, _)> = snaps
                .iter()
                .filter(|s| s.object == i && s.with == Some(j))
                .map(|s| (s.which.clone(), s.at))
                .collect();
            assert_eq!(listed, found, "{} with {}", id(i), id(j));
            crossings += found.len();
        }
    }
    // The dotted ring and the chord through it cross twice, at x = ±√75.
    let ring = place_of(&report, "ring");
    let chord = place_of(&report, "chord");
    let at: Vec<f64> = snaps
        .iter()
        .filter(|s| s.object == ring && s.with == Some(chord))
        .map(|s| s.at.x)
        .collect();
    assert_eq!(at.len(), 2, "{at:?}");
    assert!(
        at.iter().all(|x| (x.abs() - 75f64.sqrt()).abs() < 1e-9),
        "{at:?}"
    );
    assert!(crossings >= 10, "{crossings} crossings");
}

#[test]
fn the_nearest_snap_finds_the_edge_of_a_polyline_with_width_not_a_hatch() {
    let dir = scratch("wide");
    let source = dir.join("dotted.yaml");
    std::fs::write(&source, DOTTED).unwrap();
    let report = with_snaps(&source, None);
    let edges = report.edges.as_ref().expect("edges come with the snaps");
    let wide = place_of(&report, "wide");
    let fill = place_of(&report, "fill");
    let of = |object: usize| -> Vec<usize> {
        edges
            .iter()
            .copied()
            .filter(|&k| report.display_list.items[k].object == object)
            .collect()
    };
    assert_eq!(of(wide).len(), 1, "the wide polyline's fill: {edges:?}");
    assert!(of(fill).is_empty(), "no hatch's: {edges:?}");
    // On its edge, half its width above its middle line.
    let (object, at) = nearest(&report.display_list, edges, [12.0, 41.4], 1.0).unwrap();
    assert_eq!(object, wide);
    assert!(
        (at[0] - 12.0).abs() < 1e-9 && (at[1] - 41.0).abs() < 1e-6,
        "{at:?}"
    );
    // The solid hatch's fill is passed over.
    assert_eq!(
        nearest(&report.display_list, edges, [85.0, 41.0], 2.0),
        None
    );
    // Without the snaps, no edges.
    let request = DisplayListRequest::new(&source);
    assert!(display_list_with(&request, &settings())
        .unwrap()
        .edges
        .is_none());
}

#[test]
fn an_insert_gives_no_snaps_on_its_blocks_geometry_the_page_does_not_draw() {
    let dir = scratch("frozen");
    std::fs::write(
        dir.join("blocks.yaml"),
        "units: mm
layers:
  hidden: {frozen: true}
  dark: {on: false}
blocks:
  PAIR:
    objects:
      - line: {from: [80, 80, 0], to: [85, 80, 0]}
      - line: {from: [90, 90, 0], to: [95, 90, 0], layer: hidden}
      - line: {from: [90, 95, 0], to: [95, 95, 0], layer: dark}
",
    )
    .unwrap();
    let source = dir.join("sheet.yaml");
    std::fs::write(
        &source,
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {id: pair, block: PAIR, position: [0, 0, 0]}
  - line: {id: through, from: [92, 85, 0], to: [92, 99, 0]}
",
    )
    .unwrap();
    let report = with_snaps(&source, None);
    let snaps = report.snaps.as_ref().unwrap();
    let pair = place_of(&report, "pair");
    let on_pair: Vec<(SnapKind, Option<String>)> = snaps
        .iter()
        .filter(|s| s.object == pair)
        .map(|s| (s.snap, s.which.clone()))
        .collect();
    // The line drawn: its ends and its middle, and the insert's own point.
    for snap in [
        (SnapKind::Endpoint, Some("objects[0].from".to_owned())),
        (SnapKind::Midpoint, Some("objects[0]".to_owned())),
        (SnapKind::Insertion, Some("position".to_owned())),
    ] {
        assert!(on_pair.contains(&snap), "{snap:?}: {on_pair:?}");
    }
    // Nothing of the lines on a frozen layer and on one that is off, nor
    // where `through` crosses them.
    let hidden = |which: &Option<String>| {
        let which = which.as_deref().unwrap_or("");
        which.starts_with("objects[1]") || which.starts_with("objects[2]")
    };
    assert!(!on_pair.iter().any(|(_, w)| hidden(w)), "{on_pair:?}");
    let crossings: Vec<_> = snaps
        .iter()
        .filter(|s| s.snap == SnapKind::Intersection)
        .collect();
    assert!(crossings.is_empty(), "{crossings:?}");
    // `query snap`, asked of the insert by its ID, still answers for them.
    let answer = query_with(
        &QueryRequest {
            source: source.clone(),
            sheet: None,
            query: Query::Snap(SnapRef {
                id: Some(report.objects[pair].id.clone().unwrap()),
                snap: Some(SnapKind::Endpoint),
                which: Some("objects[1].from".into()),
                ..SnapRef::default()
            }),
        },
        &settings(),
    )
    .unwrap();
    let Answer::Snap { points, .. } = &answer.answer else {
        panic!("not a snap");
    };
    assert_eq!(points.len(), 1);
}

#[test]
fn a_clipped_viewport_snaps_on_the_boundary_it_is_drawn_as() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/layouts.yaml");
    let report = with_snaps(&source, Some("Assembly"));
    let snaps = report.snaps.as_ref().unwrap();
    let of = |id: &str| -> Vec<(SnapKind, Option<String>, [f64; 2])> {
        let place = place_of(&report, id);
        snaps
            .iter()
            .filter(|s| s.object == place && s.with.is_none())
            .map(|s| (s.snap, s.which.clone(), [s.at.x, s.at.y]))
            .collect()
    };
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    // Clipped to a circle centred (230, 195), radius 55: its centre and its
    // quadrants, and no corner of the frame it is clipped from.
    let circle = of("detail-x2");
    assert!(
        circle
            .iter()
            .all(|(k, _, _)| matches!(k, SnapKind::Center | SnapKind::Quadrant)),
        "{circle:?}"
    );
    let point = |snap: SnapKind, which: Option<&str>| {
        circle
            .iter()
            .find(|(k, w, _)| *k == snap && w.as_deref() == which)
            .map(|(_, _, at)| *at)
    };
    assert!(near(point(SnapKind::Center, None).unwrap(), [230.0, 195.0]));
    for (which, at) in [
        ("0", [285.0, 195.0]),
        ("90", [230.0, 250.0]),
        ("180", [175.0, 195.0]),
        ("270", [230.0, 140.0]),
    ] {
        let found =
            point(SnapKind::Quadrant, Some(which)).unwrap_or_else(|| panic!("{which}: {circle:?}"));
        assert!(near(found, at), "{which}: {found:?}");
    }
    // Clipped to a polyline: its points, as a closed polyline's.
    let polygon = of("detail-k1");
    for (i, at) in [[290.0, 80.0], [370.0, 80.0], [370.0, 190.0], [290.0, 190.0]]
        .into_iter()
        .enumerate()
    {
        let which = format!("points[{i}]");
        assert!(
            polygon.iter().any(|(k, w, p)| *k == SnapKind::Endpoint
                && w.as_deref() == Some(which.as_str())
                && near(*p, at)),
            "{which}: {polygon:?}"
        );
    }
    assert!(
        !polygon
            .iter()
            .any(|(_, w, _)| w.as_deref().unwrap_or("").starts_with("corners")),
        "{polygon:?}"
    );
}
