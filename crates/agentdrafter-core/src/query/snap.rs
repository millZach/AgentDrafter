//! The snap and geometry queries: the points AutoCAD's object snaps find
//! on an object named by its ID or its name, or between it and another,
//! and each selected object's length, area, extents and angles.
//!
//! [`resolve`] works out a [`SnapRef`] on the sheets a query looks at, so
//! that whatever takes a snapped point, as an operation will, finds the
//! same point a query gives.

use serde::Serialize;

use super::{article, matched, object_named, Answer, LookedAt, Selection, SheetRecord};
use crate::error::{Error, ErrorCode};
use crate::geometry::measure::Measures;
use crate::geometry::snap::{
    counted, from_point, intersections, own_points, tally, SnapKind, SnapPoint,
};
use crate::geometry::{p3, ObjectRef, Reader, SnapRef};
use crate::model::Point3;
use crate::settings::Settings;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// An object a snap is of.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnapObject {
    pub sheet: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The object's type, such as `insert`.
    #[serde(rename = "type")]
    pub kind: String,
}

/// The point a perpendicular, a tangent or a nearest point is taken from:
/// a snap of another object.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnappedFrom {
    #[serde(flatten)]
    pub object: SnapObject,
    pub snap: SnapKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    /// Where it is, in the declared unit.
    pub at: Point3,
}

/// What one object measures, and what it has none of.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObjectGeometry {
    #[serde(flatten)]
    pub object: SnapObject,
    #[serde(flatten)]
    pub measures: Measures,
    /// What it has none of: `length`, `area`, `extents`, `angle`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub without: Vec<&'static str>,
}

/// An object found on one of the sheets looked at.
#[derive(Clone, Copy)]
struct Located<'a> {
    record: &'a SheetRecord,
    index: usize,
}

impl Located<'_> {
    fn described(&self) -> SnapObject {
        SnapObject {
            sheet: self.record.name.clone(),
            id: self.record.id(self.index),
            name: self.record.objects[self.index].name.clone(),
            kind: self.object().type_name().to_owned(),
        }
    }

    fn object(&self) -> &crate::model::Object {
        &self.record.sheet.objects[self.index]
    }

    /// "the line `S/w1`".
    fn words(&self) -> String {
        format!(
            "the {} `{}`",
            self.object().type_name(),
            self.record.id(self.index)
        )
    }
}

/// A snap worked out: the objects it names, those that matched, and the
/// points it found.
struct Resolved<'a> {
    object: Option<Located<'a>>,
    with: Option<Option<Located<'a>>>,
    /// The point `from` names, or the objects it names, however deep, that
    /// match nothing.
    from: Option<Result<Box<SnappedFrom>, Vec<Missing>>>,
    points: Vec<SnapPoint>,
    /// What the object has none of the kind asked, in words, when it found
    /// no point.
    none: Option<String>,
}

/// An object a snap names that no sheet looked at holds: as it is named,
/// what it is for, and the place of its ID or name in the request.
#[derive(Clone, Debug)]
struct Missing {
    named: ObjectRef,
    what: &'static str,
    field: String,
}

impl Missing {
    fn new(named: &ObjectRef, what: &'static str, at: &str) -> Missing {
        let key = if named.id.is_some() { "id" } else { "name" };
        Missing {
            named: named.clone(),
            what,
            field: format!("{at}.{key}"),
        }
    }

    /// "the ID `S/ring`, the object to snap to".
    fn words(&self) -> String {
        format!("{}, the object {}", self.named.words(), self.what)
    }
}

impl Resolved<'_> {
    /// The objects `request`, at `field`, names that match nothing: its
    /// own, its `with`, and those its `from` names, at their places.
    fn missing(&self, request: &SnapRef, field: &str) -> Vec<Missing> {
        let mut missing = Vec::new();
        if self.object.is_none() {
            missing.push(Missing::new(&request.object(), "to snap to", field));
        }
        if let (Some(None), Some(with)) = (&self.with, &request.with) {
            let at = format!("{field}.with");
            missing.push(Missing::new(with, "to intersect with", &at));
        }
        if let Some(Err(within)) = &self.from {
            missing.extend(within.iter().cloned());
        }
        missing
    }
}

/// Checks the parts of a snap agree: one object, a kind for `which`, and
/// `with` or `from` just where the kind takes them.
pub(crate) fn check_shape(request: &SnapRef, field: &str) -> Result<(), Error> {
    if request.id.is_some() == request.name.is_some() {
        return Err(
            Error::usage("a snap names its object by `id` or by `name`, and not both")
                .at_field(field.to_owned()),
        );
    }
    let kind = request.snap;
    if request.which.is_some() && kind.is_none() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`which` names a point of one snap: give the snap too",
        )
        .at_field(format!("{field}.which")));
    }
    match (kind, &request.with) {
        (Some(k), None) if k.takes_with() => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "an {} is between two objects: give the other as `with`",
                    k.word()
                ),
            )
            .at_field(format!("{field}.with")))
        }
        (k, Some(_)) if !k.is_some_and(SnapKind::takes_with) => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`with` gives the other object of an intersection or an apparent \
                     intersection, not of {}",
                    k.map_or("every snap of one object".to_owned(), |k| format!(
                        "a {} snap",
                        k.name()
                    ))
                ),
            )
            .at_field(format!("{field}.with")))
        }
        (_, Some(with)) if with.id.is_some() == with.name.is_some() => {
            return Err(
                Error::usage("`with` names its object by `id` or by `name`, and not both")
                    .at_field(format!("{field}.with")),
            )
        }
        _ => {}
    }
    match (kind, &request.from) {
        (Some(k), None) if k.takes_from() => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "a {} snap is taken from a point: give it as `from`, a snap of another object",
                k.name()
            ),
        )
        .at_field(format!("{field}.from"))),
        (k, Some(_)) if !k.is_some_and(SnapKind::takes_from) => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`from` gives the point a perpendicular, a tangent or a nearest point is taken \
                 from, not that of {}",
                k.map_or("every snap of one object".to_owned(), |k| format!(
                    "a {} snap",
                    k.name()
                ))
            ),
        )
        .at_field(format!("{field}.from"))),
        (_, Some(from)) if from.snap.is_none() => Err(Error::new(
            ErrorCode::BadValue,
            "`from` is a point: give its snap, and which of its points when it has several",
        )
        .at_field(format!("{field}.from.snap"))),
        _ => Ok(()),
    }
}

/// The sheet's fonts, for text's nodes and multileaders' layout.
fn fonts_of(record: &SheetRecord, settings: &Settings) -> SheetFonts {
    SheetFonts::find(&record.sheet, settings)
}

/// Works out `request` on `sheets`: the objects it names and the points it
/// finds, those `which` names only. `field` is its place in the request,
/// for an error.
fn resolve<'a>(
    sheets: &[&'a SheetRecord],
    request: &SnapRef,
    settings: &Settings,
    field: &str,
) -> Result<Resolved<'a>, Error> {
    check_shape(request, field)?;
    let locate = |object: &ObjectRef, field: &str| -> Result<Option<Located<'a>>, Error> {
        Ok(object_named(sheets, object, field, "a snap", "snap")?
            .map(|(record, index)| Located { record, index }))
    };
    let object = locate(&request.object(), field)?;
    let with = match &request.with {
        Some(with) => Some(locate(with, &format!("{field}.with"))?),
        None => None,
    };
    let from = match &request.from {
        Some(from) => Some(from_of(sheets, from, settings, &format!("{field}.from"))?),
        None => None,
    };
    let mut resolved = Resolved {
        object,
        with,
        from,
        points: Vec::new(),
        none: None,
    };
    let Some(object) = object else {
        return Ok(resolved);
    };
    if matches!(resolved.with, Some(None)) || matches!(resolved.from, Some(Err(_))) {
        return Ok(resolved);
    }
    let on_sheet = |other: &str, sheet: &str, at: &str| -> Result<(), Error> {
        if sheet == object.record.name {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`{}` is on sheet `{}` and {other} on sheet `{sheet}`; a snap is between \
                 objects on one sheet",
                object.record.id(object.index),
                object.record.name
            ),
        )
        .at_field(at.to_owned()))
    };
    if let Some(Some(with)) = &resolved.with {
        on_sheet(
            &format!("`{}`", with.record.id(with.index)),
            &with.record.name,
            &format!("{field}.with"),
        )?;
        if with.index == object.index {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`with` names {} itself; an intersection is between two objects",
                    object.words()
                ),
            )
            .at_field(format!("{field}.with")));
        }
    }
    if let Some(Ok(from)) = &resolved.from {
        on_sheet(
            &format!("the point to snap from, of `{}`,", from.object.id),
            &from.object.sheet,
            &format!("{field}.from"),
        )?;
    }
    let fonts = fonts_of(object.record, settings);
    let reader = Reader {
        sheet: &object.record.sheet,
        fonts: &fonts,
    };
    let kind = request.snap;
    let mut named: Vec<(String, SnapPoint)> = match (kind, &resolved.with, &resolved.from) {
        (Some(k), Some(Some(with)), _) => {
            let apparent = k == SnapKind::ApparentIntersection;
            let points = intersections(
                &reader.parts(object.object()),
                &reader.parts(with.object()),
                apparent,
            );
            points.into_iter().map(|p| (which_of(&p), p)).collect()
        }
        (Some(k), _, Some(Ok(from))) => {
            let at = p3(from.at);
            from_point(&reader.parts(object.object()), k, at)
                .into_iter()
                .map(|p| (which_of(&p), p))
                .collect()
        }
        _ => own_points(&reader, object.object(), kind),
    };
    if let Some(which) = &request.which {
        let choices: Vec<String> = named
            .iter()
            .map(|(name, _)| name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        // Counted before `which` picks among them.
        let found = named.len();
        named.retain(|(name, _)| name == which);
        if named.is_empty() {
            let kind = kind.unwrap_or(SnapKind::Endpoint);
            let message = if choices.is_empty() {
                format!(
                    "{} has no {} `{which}`; it has {}",
                    object.words(),
                    kind.word(),
                    match found {
                        0 => format!("no {}", kind.word()),
                        1 => format!("one {}, which takes no `which`", kind.word()),
                        n => format!("{n} {}s, none of which has a `which`", kind.word()),
                    }
                )
            } else {
                format!(
                    "{} has no {} `{which}`; its {}s: {}",
                    object.words(),
                    kind.word(),
                    kind.word(),
                    choices.join(", ")
                )
            };
            return Err(
                Error::new(ErrorCode::UnknownName, message).at_field(format!("{field}.which"))
            );
        }
    }
    resolved.points = named.into_iter().map(|(_, p)| p).collect();
    if resolved.points.is_empty() {
        resolved.none = Some(none_words(&object, kind, &resolved));
    }
    Ok(resolved)
}

/// The name a snap between two objects is asked for again by: its place in
/// the order the reply gives them.
fn which_of(point: &SnapPoint) -> String {
    point.which.clone().unwrap_or_default()
}

/// What an object has none of, in words, when a snap found no point.
fn none_words(object: &Located, kind: Option<SnapKind>, resolved: &Resolved) -> String {
    let thing = article(object.object().type_name());
    let id = object.record.id(object.index);
    match (kind, &resolved.with, &resolved.from) {
        (Some(k), Some(Some(with)), _) => format!(
            "{} and {} do not {}",
            object.words(),
            with.words(),
            if k == SnapKind::ApparentIntersection {
                "cross seen from above"
            } else {
                "cross"
            }
        ),
        (Some(k), _, Some(Ok(from))) => format!(
            "there is no {} from the {} of `{}` to {}",
            k.name(),
            from.snap.word(),
            from.object.id,
            object.words()
        ),
        (Some(k), _, _) => format!(
            "{thing} has no {} snap, as in AutoCAD: `{id}` has none",
            k.word()
        ),
        (None, _, _) => format!("{thing} has no point any object snap finds: `{id}` has none"),
    }
}

/// The point a snap's `from` names, which must be one point; none when
/// an object it names matches nothing.
fn from_of(
    sheets: &[&SheetRecord],
    request: &SnapRef,
    settings: &Settings,
    field: &str,
) -> Result<Result<Box<SnappedFrom>, Vec<Missing>>, Error> {
    let resolved = resolve(sheets, request, settings, field)?;
    let missing = resolved.missing(request, field);
    let Some(object) = resolved.object.filter(|_| missing.is_empty()) else {
        // Its own object is the one to snap from.
        let own = format!("{field}.");
        return Ok(Err(missing
            .into_iter()
            .map(|m| {
                let mine = m.field.strip_prefix(&own).is_some_and(|k| !k.contains('.'));
                Missing {
                    what: if mine { "to snap from" } else { m.what },
                    ..m
                }
            })
            .collect()));
    };
    let kind = request.snap.unwrap_or(SnapKind::Endpoint);
    match resolved.points.as_slice() {
        [] => Err(Error::new(
            ErrorCode::BadValue,
            format!("{} has no {} to snap from", object.words(), kind.word()),
        )
        .at_field(field.to_owned())),
        [one] => Ok(Ok(Box::new(SnappedFrom {
            object: object.described(),
            snap: one.snap,
            which: request.which.clone().or_else(|| one.which.clone()),
            at: one.at,
        }))),
        several => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "{} has {}; a point to snap from is one: name it by `which`, one of {}",
                object.words(),
                counted(kind, several.len()),
                several
                    .iter()
                    .filter_map(|p| p.which.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .at_field(format!("{field}.which"))),
    }
}

/// The one point a snapped point an operation takes names on `sheets`:
/// the object it snaps to, and the point. `field` is the snap's place in
/// the operation, for an error. A snap that names an object no sheet
/// holds, finds no point, or finds several and names none by `which`, is
/// an error at its key, the last listing each point with where it is.
pub(crate) fn one_point(
    sheets: &[&SheetRecord],
    request: &SnapRef,
    settings: &Settings,
    field: &str,
) -> Result<(SnapObject, SnapPoint), Error> {
    let Some(kind) = request.snap else {
        return Err(no_kind(field));
    };
    let resolved = resolve(sheets, request, settings, field)?;
    let on = match sheets {
        [one] => format!("on sheet `{}`", one.name),
        all => format!("on the {} sheets", all.len()),
    };
    // The IDs on the sheets, or for an object named by its name, the names.
    let choices = |by_id: bool| {
        let listed: Vec<String> = sheets
            .iter()
            .flat_map(|r| {
                (0..r.objects.len()).filter_map(move |i| {
                    if by_id {
                        Some(r.id(i))
                    } else {
                        r.objects[i].name.clone()
                    }
                })
            })
            .collect();
        listed_ids(&listed)
    };
    // The first object it names, however deep, that the sheet does not
    // hold, at its place.
    if let Some(m) = resolved.missing(request, field).first() {
        let by_id = m.named.id.is_some();
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "no object {on} has {}; its {}: {}",
                m.words(),
                if by_id { "IDs" } else { "names" },
                choices(by_id)
            ),
        )
        .at_field(m.field.clone()));
    }
    let object = resolved.object.expect("an object that matched");
    match resolved.points.as_slice() {
        [] => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the snapped point names no point: {}",
                none_words(&object, Some(kind), &resolved)
            ),
        )
        .at_field(field.to_owned())),
        [one] => Ok((object.described(), one.clone())),
        several => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "{} has {}; a snapped point is one: name it by `which`, one of {}",
                object.words(),
                counted(kind, several.len()),
                several
                    .iter()
                    .map(|p| format!(
                        "`{}` at [{}, {}, {}]",
                        p.which.as_deref().unwrap_or_default(),
                        p.at.x,
                        p.at.y,
                        p.at.z
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .at_field(format!("{field}.which"))),
    }
}

/// The error for a snapped point that names no snap.
pub(crate) fn no_kind(field: &str) -> Error {
    let kinds: Vec<&str> = SnapKind::ALL.iter().map(|k| k.name()).collect();
    Error::new(
        ErrorCode::BadValue,
        format!(
            "a snapped point names its snap, one of {}, and which of its points when it has              several",
            kinds.join(", ")
        ),
    )
    .at_field(format!("{field}.snap"))
}

/// The IDs on a sheet, for an error: the first sixty, and how many more.
fn listed_ids(ids: &[String]) -> String {
    const MOST: usize = 60;
    if ids.is_empty() {
        return "none".to_owned();
    }
    let shown = ids
        .iter()
        .take(MOST)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if ids.len() > MOST {
        format!("{shown} and {} more", ids.len() - MOST)
    } else {
        shown
    }
}

/// The snap query.
pub(super) fn snap(
    sheets: &[&SheetRecord],
    request: &SnapRef,
    settings: &Settings,
    looked: &LookedAt,
) -> Result<(String, Answer), Error> {
    let resolved = resolve(sheets, request, settings, "query")?;
    let missing: Vec<String> = resolved
        .missing(request, "query")
        .iter()
        .map(Missing::words)
        .collect();
    let answer = |points: Vec<SnapPoint>| Answer::Snap {
        count: points.len(),
        object: resolved.object.map(|o| o.described()),
        with: resolved.with.flatten().map(|o| o.described()),
        from: resolved.from.clone().and_then(Result::ok),
        points,
    };
    if !missing.is_empty() {
        let message = format!(
            "nothing matches: no object {} has {}",
            looked.on(),
            missing.join(", nor ")
        );
        return Ok((message, answer(Vec::new())));
    }
    let object = resolved.object.expect("an object that matched");
    let on = format!("on sheet `{}`", object.record.name);
    let message = match (&resolved.none, request.snap, resolved.points.as_slice()) {
        (Some(none), _, _) => format!("nothing matches: {none}"),
        (None, _, [one]) if request.which.is_some() || one.which.is_none() => format!(
            "the {} of {}, {on}: [{}, {}, {}]",
            one.snap.word(),
            object.words(),
            one.at.x,
            one.at.y,
            one.at.z
        ),
        (None, Some(k), points) => format!(
            "{} of {}{}, {on}",
            counted(k, points.len()),
            object.words(),
            match &resolved.from {
                Some(Ok(from)) => format!(" from the {} of `{}`", from.snap.word(), from.object.id),
                _ => match resolved.with.flatten() {
                    Some(with) => format!(" with {}", with.words()),
                    None => String::new(),
                },
            }
        ),
        (None, None, points) => format!(
            "{} snap points of {}, {on}: {}",
            points.len(),
            object.words(),
            tally(points)
        ),
    };
    let points = resolved.points.clone();
    Ok((message, answer(points)))
}

/// The geometry query: each matching object's measures.
pub(super) fn geometry(
    sheets: &[&SheetRecord],
    selection: &Selection,
    settings: &Settings,
    units: Unit,
    looked: &LookedAt,
) -> (String, Answer) {
    let mut fonts: Vec<(String, SheetFonts)> = Vec::new();
    let objects: Vec<ObjectGeometry> = matched(sheets, selection, settings)
        .map(|m| {
            let located = Located {
                record: m.record,
                index: m.index,
            };
            if !fonts.iter().any(|(name, _)| *name == m.record.name) {
                fonts.push((m.record.name.clone(), fonts_of(m.record, settings)));
            }
            let (_, sheet_fonts) = fonts
                .iter()
                .find(|(name, _)| *name == m.record.name)
                .expect("the sheet's fonts");
            let reader = Reader {
                sheet: &m.record.sheet,
                fonts: sheet_fonts,
            };
            let object = located.object();
            let measures = reader.measures(object);
            ObjectGeometry {
                object: located.described(),
                without: measures.without(object),
                measures,
            }
        })
        .collect();
    let count = objects.len();
    let message = match objects.as_slice() {
        [one] => {
            let mut said = Vec::new();
            if let Some(measured) = one.measures.measurement {
                let shows = one
                    .measures
                    .text
                    .as_deref()
                    .map(|t| format!(" and shows `{t}`"))
                    .unwrap_or_default();
                said.push(format!("measures {measured} {units}{shows}"));
            }
            if let Some(length) = one.measures.length {
                said.push(format!("is {length} {units} long"));
            }
            if let Some(area) = one.measures.area {
                said.push(format!("encloses {area} square {units}"));
            }
            if said.is_empty() {
                format!(
                    "{} `{}`, on sheet `{}`, has no length and no area",
                    article(&one.object.kind),
                    one.object.id,
                    one.object.sheet
                )
            } else {
                format!(
                    "{} `{}`, on sheet `{}`, {}",
                    article(&one.object.kind),
                    one.object.id,
                    one.object.sheet,
                    said.join(" and ")
                )
            }
        }
        _ => super::found_message(count, "object", selection, sheets, &looked.on()),
    };
    (message, Answer::Geometry { count, objects })
}
