//! Reads dimension styles and dimensions.
//!
//! A dimension style is a mapping from its name to what its dimensions
//! draw with, like a multileader style: each starts from its unit's own,
//! `ISO-25` on a metric sheet and `Standard` on an imperial one, and sets
//! what it gives. A dimension is an object of a sheet: `objects.rs` lists
//! it and hands its keys to [`dimension`] here. It belongs on a sheet, not
//! in a block.
//!
//! What the reader is given it puts where AutoCAD keeps it: a linear or
//! aligned dimension's location on its dimension line where the second
//! extension line meets it, a radius or diameter dimension's point on the
//! line from its centre through its location, where its arrow points, and
//! an angular dimension's location on its arc a third of the way round.

use super::layers::{name_error, named_entries};
use super::leader::{arrowhead, not_negative, positive};
use super::strict_yaml::{key_path, Node};
use super::text::{character_code, contents_problem, text_style_at};
use super::{
    angle, at, bad_value, boolean, choice, index_path, plain_number, point, points, scalar,
    sequence, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    AngularUnit, DecimalSeparator, Dimension, DimensionKind, DimensionStyle, FractionFormat,
    Measured, OrdinateAxis, Point3, TextPlacement, UnitFormat, ISO_DIMENSION_STYLE,
    STANDARD_DIMENSION_STYLE,
};
use crate::text::TextStyle;
use crate::units::Unit;

/// The keys a dimension style takes.
pub(crate) const DIMENSION_STYLE_KEYS: [&str; 26] = [
    "arrowhead",
    "arrow_size",
    "text_style",
    "text_height",
    "gap",
    "extension_offset",
    "extension_beyond",
    "line_extension",
    "center_mark",
    "baseline_spacing",
    "placement",
    "unit_format",
    "precision",
    "fraction_format",
    "decimal_separator",
    "suppress_leading_zeros",
    "suppress_trailing_zeros",
    "suppress_zero_feet",
    "suppress_zero_inches",
    "angular_unit",
    "angular_precision",
    "suppress_angular_leading_zeros",
    "suppress_angular_trailing_zeros",
    "prefix",
    "suffix",
    "scale",
];

/// The keys a dimension takes.
pub(crate) const DIMENSION_KEYS: [&str; 17] = [
    "kind",
    "points",
    "angle",
    "center",
    "point",
    "lines",
    "feature",
    "axis",
    "location",
    "dimension_style",
    "text",
    "continued",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The keys a dimension of `kind` takes of its own, besides `kind`,
/// `location` and the keys every dimension takes: those it requires, then
/// those it may give, then those it does not take, which another kind's
/// are.
pub(crate) fn kind_keys(
    kind: DimensionKind,
) -> (
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
) {
    match kind {
        DimensionKind::Linear => (
            &["points"],
            &["angle", "continued"],
            &["center", "point", "lines", "feature", "axis"],
        ),
        DimensionKind::Aligned => (
            &["points"],
            &["continued"],
            &["center", "point", "angle", "lines", "feature", "axis"],
        ),
        DimensionKind::Radius | DimensionKind::Diameter => (
            &["center", "point"],
            &[],
            &["points", "angle", "lines", "feature", "axis", "continued"],
        ),
        DimensionKind::Angular => (
            &["lines"],
            &["continued"],
            &["points", "angle", "center", "point", "feature", "axis"],
        ),
        DimensionKind::ThreePointAngular => (
            &["center", "points"],
            &["continued"],
            &["angle", "point", "lines", "feature", "axis"],
        ),
        DimensionKind::Ordinate => (
            &["feature", "axis"],
            &[],
            &["points", "angle", "center", "point", "lines", "continued"],
        ),
    }
}

/// One of `all` by its name at `key`, if the fields give it: `what` it is,
/// and the sentence that lists them.
fn named<T: Copy>(
    fields: &Fields,
    key: &str,
    what: &str,
    all: &[T],
    name: fn(T) -> &'static str,
    listed: &str,
) -> Result<Option<T>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let text = choice(node, &path, what, listed)?;
    all.iter()
        .copied()
        .find(|t| name(*t) == text)
        .map(Some)
        .ok_or_else(|| bad_value(node, &path, format!("`{text}` is not {what}; {listed}")))
}

/// The names of `all`, as a sentence lists them.
fn names<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|t| format!("`{}`", name(*t)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A whole number from `least` to `most` at `key`, if the fields give it.
fn whole(fields: &Fields, key: &str, least: i8, most: i8, says: &str) -> Result<Option<i8>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let text = scalar(node, &path, "a whole number")?;
    match text.parse::<i8>() {
        Ok(n) if (least..=most).contains(&n) => Ok(Some(n)),
        _ => Err(bad_value(node, &path, says.to_owned())),
    }
}

/// The most decimals AutoCAD shows.
pub(crate) const MOST_DECIMALS: u8 = 8;

pub(super) fn dimension_styles(
    node: &Node,
    units: Unit,
    text_styles: &[TextStyle],
) -> Result<Vec<DimensionStyle>, Error> {
    let entries = named_entries(node, "dimension_styles", "dimension style")?;
    let mut styles = Vec::with_capacity(entries.len());
    for entry in entries {
        // As for text styles: AutoCAD's own names are written exactly.
        for own in [STANDARD_DIMENSION_STYLE, ISO_DIMENSION_STYLE] {
            if entry.key != own && entry.key.eq_ignore_ascii_case(own) {
                return Err(name_error(
                    entry,
                    "dimension_styles",
                    format!(
                        "`{}` is AutoCAD's dimension style `{own}` written in another case, and AutoCAD ignores case in names; list it as `{own}`",
                        entry.key
                    ),
                ));
            }
        }
        let path = key_path("dimension_styles", &entry.key);
        let fields = Fields::of(
            &entry.value,
            &path,
            &DIMENSION_STYLE_KEYS,
            "a dimension style",
        )?;
        let mut style = DimensionStyle::built_in(DimensionStyle::default_name(units), units)
            .expect("a unit has its own dimension style");
        entry.key.clone_into(&mut style.name);
        style.text_style = text_style_at(&fields, "text_style", text_styles)?;
        if let Some(a) = arrowhead(&fields)? {
            style.arrowhead = a;
        }
        let lengths: [(&str, &str, &mut f64, bool); 8] = [
            ("arrow_size", "an arrow's size", &mut style.arrow_size, true),
            ("text_height", "a text height", &mut style.text_height, true),
            ("gap", "a gap", &mut style.gap, false),
            (
                "extension_offset",
                "an extension line's offset",
                &mut style.extension_offset,
                false,
            ),
            (
                "extension_beyond",
                "how far an extension line reaches",
                &mut style.extension_beyond,
                false,
            ),
            (
                "line_extension",
                "how far the dimension line reaches",
                &mut style.line_extension,
                false,
            ),
            (
                "center_mark",
                "a centre mark",
                &mut style.center_mark,
                false,
            ),
            (
                "baseline_spacing",
                "how far apart baseline dimensions are",
                &mut style.baseline_spacing,
                false,
            ),
        ];
        for (key, what, value, more_than_zero) in lengths {
            let read = if more_than_zero {
                positive(&fields, key, units, what)?
            } else {
                not_negative(&fields, key, units, what)?
            };
            if let Some(read) = read {
                *value = read;
            }
        }
        if let Some(node) = fields.get("scale") {
            let path = fields.path_of("scale");
            let scale = plain_number(node, &path, "a scale, a number such as `2`")?;
            if scale.is_nan() || scale <= 0.0 {
                return Err(bad_value(
                    node,
                    &path,
                    "a dimension style's scale is more than 0; AutoCAD's scale of 0, which takes \
                     the layout's, waits for layouts",
                ));
            }
            style.scale = scale;
        }
        if let Some(node) = fields.get("placement") {
            let path = fields.path_of("placement");
            let listed = "a dimension's text is placed `centered`, horizontal in the middle of \
                          its line as Standard has it, or `above`, aligned with its line and \
                          above it as ISO-25 has it";
            let text = choice(node, &path, "a placement", listed)?;
            style.placement = TextPlacement::ALL
                .into_iter()
                .find(|p| p.name() == text)
                .ok_or_else(|| {
                    bad_value(
                        node,
                        &path,
                        format!("`{text}` is not a placement; {listed}"),
                    )
                })?;
        }
        let listed = format!(
            "a unit format is {}",
            names(&UnitFormat::ALL, UnitFormat::name)
        );
        if let Some(format) = named(
            &fields,
            "unit_format",
            "a unit format",
            &UnitFormat::ALL,
            UnitFormat::name,
            &listed,
        )? {
            // AutoCAD counts a drawing unit as an inch in these, whatever
            // the drawing's unit: on a sheet in another unit they would show
            // a length as so many inches that is not.
            if format.counts_inches() && units != Unit::In {
                return Err(bad_value(
                    fields.require("unit_format")?,
                    &fields.path_of("unit_format"),
                    format!(
                        "an {} length counts feet and inches, so it is for a sheet in `in`; this \
                         sheet is in `{}`",
                        format.name(),
                        units.symbol()
                    ),
                ));
            }
            style.unit_format = format;
        }
        if let Some(n) = whole(
            &fields,
            "precision",
            0,
            MOST_DECIMALS as i8,
            &format!(
                "a precision is a whole number from 0 to {MOST_DECIMALS}: how many decimals a \
                 length shows, or for an architectural or fractional one how fine its fraction \
                 is, 1/2 to 1/256"
            ),
        )? {
            style.precision = n as u8;
        }
        let listed = format!(
            "a fraction format is {}",
            names(&FractionFormat::ALL, FractionFormat::name)
        );
        if let Some(format) = named(
            &fields,
            "fraction_format",
            "a fraction format",
            &FractionFormat::ALL,
            FractionFormat::name,
            &listed,
        )? {
            style.fraction_format = format;
        }
        let listed = format!(
            "an angular unit is {}",
            names(&AngularUnit::ALL, AngularUnit::name)
        );
        if let Some(unit) = named(
            &fields,
            "angular_unit",
            "an angular unit",
            &AngularUnit::ALL,
            AngularUnit::name,
            &listed,
        )? {
            style.angular_unit = unit;
        }
        if let Some(n) = whole(
            &fields,
            "angular_precision",
            0,
            MOST_DECIMALS as i8,
            &format!(
                "an angular precision is a whole number of decimals from 0 to {MOST_DECIMALS}, \
                 as AutoCAD's DIMADEC in a dimension style"
            ),
        )? {
            style.angular_precision = n as u8;
        }
        if let Some(node) = fields.get("decimal_separator") {
            let path = fields.path_of("decimal_separator");
            let names = DecimalSeparator::ALL.map(DecimalSeparator::name);
            let listed = format!("a decimal separator is {}", names.join(", "));
            let text = choice(node, &path, "a decimal separator", &listed)?;
            style.decimal_separator = DecimalSeparator::ALL
                .into_iter()
                .find(|s| s.name() == text)
                .ok_or_else(|| {
                    bad_value(
                        node,
                        &path,
                        format!("`{text}` is not a decimal separator; {listed}"),
                    )
                })?;
        }
        if let Some(node) = fields.get("suppress_leading_zeros") {
            style.suppress_leading_zeros =
                boolean(node, &fields.path_of("suppress_leading_zeros"))?;
        }
        if let Some(node) = fields.get("suppress_trailing_zeros") {
            style.suppress_trailing_zeros =
                boolean(node, &fields.path_of("suppress_trailing_zeros"))?;
        }
        let switches: [(&str, &mut bool); 4] = [
            ("suppress_zero_feet", &mut style.suppress_zero_feet),
            ("suppress_zero_inches", &mut style.suppress_zero_inches),
            (
                "suppress_angular_leading_zeros",
                &mut style.suppress_angular_leading_zeros,
            ),
            (
                "suppress_angular_trailing_zeros",
                &mut style.suppress_angular_trailing_zeros,
            ),
        ];
        for (key, value) in switches {
            if let Some(node) = fields.get(key) {
                *value = boolean(node, &fields.path_of(key))?;
            }
        }
        style.prefix = affix(&fields, "prefix")?;
        style.suffix = affix(&fields, "suffix")?;
        styles.push(style);
    }
    Ok(styles)
}

/// A prefix or a suffix: one line of text that holds no `<>` and no code
/// AutoCAD reads in multiline text; empty when it is left out.
fn affix(fields: &Fields, key: &str) -> Result<String, Error> {
    let Some(node) = fields.text(key) else {
        return Ok(String::new());
    };
    let path = fields.path_of(key);
    let text = scalar(node, &path, "text")?;
    let problem = if text.contains('<') || text.contains('>') {
        Some("holds `<` or `>`, which AutoCAD's DIMPOST reads round the measured value".to_owned())
    } else if text.contains('\n') {
        Some("is one line".to_owned())
    } else if text.contains('\\') || text.contains('{') || text.contains('}') {
        Some(
            "holds `\\`, `{` or `}`, which AutoCAD reads as codes in a dimension's text".to_owned(),
        )
    } else if text.contains("%%") {
        Some(
            "holds `%%`, which AutoCAD reads in DIMPOST as a control code, `%%c` as a diameter \
             sign"
                .to_owned(),
        )
    } else {
        text.chars()
            .find(|c| c.is_control())
            .map(|c| format!("holds the control character `{}`", c.escape_default()))
    };
    match problem {
        Some(problem) => Err(bad_value(node, &path, format!("a {key} {problem}"))),
        None => Ok(text.to_owned()),
    }
}

pub(super) fn dimension(
    fields: &Fields,
    units: Unit,
    layer: String,
    dimension_styles: &[DimensionStyle],
) -> Result<Dimension, Error> {
    let kind_node = fields.require("kind")?;
    let kind_path = fields.path_of("kind");
    let names = DimensionKind::ALL.map(DimensionKind::name);
    let listed = format!("a dimension is {}", names.join(", "));
    let text = choice(kind_node, &kind_path, "a kind of dimension", &listed)?;
    let Some(kind) = DimensionKind::ALL.into_iter().find(|k| k.name() == text) else {
        return Err(bad_value(
            kind_node,
            &kind_path,
            format!("`{text}` is not a kind of dimension; {listed}"),
        ));
    };
    // Each kind takes its own points.
    let (required, optional, others) = kind_keys(kind);
    let own: Vec<&str> = required
        .iter()
        .chain(optional)
        .copied()
        .filter(|k| *k != "continued")
        .collect();
    if let Some(key) = others.iter().find(|k| fields.get(k).is_some()) {
        let node = fields.require(key)?;
        return Err(bad_value(
            node,
            &fields.path_of(key),
            format!(
                "{} {} dimension takes {}, not `{key}`",
                super::article(kind.name()),
                kind.name(),
                own.iter()
                    .map(|k| format!("`{k}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        ));
    }
    let location_node = fields.require("location")?;
    let location_path = fields.path_of("location");
    let location = point(location_node, &location_path, units)?;
    let refused = |node: &Node, path: &str, why: String| bad_value(node, path, why);
    let measured = match kind {
        DimensionKind::Linear | DimensionKind::Aligned => {
            let node = fields.require("points")?;
            let path = fields.path_of("points");
            let read = points(node, &path, units, 2)?;
            let [p1, p2] = read.as_slice() else {
                return Err(refused(
                    node,
                    &path,
                    format!(
                        "a {} dimension measures between two points; this lists {}",
                        kind.name(),
                        read.len()
                    ),
                ));
            };
            let u = if kind == DimensionKind::Linear {
                let degrees = match fields.get("angle") {
                    Some(node) => angle(node, &fields.path_of("angle"))?,
                    None => 0.0,
                };
                let (s, c) = degrees.to_radians().sin_cos();
                [c, s]
            } else {
                let d = [p2.x - p1.x, p2.y - p1.y];
                let length = d[0].hypot(d[1]);
                if length == 0.0 {
                    return Err(refused(
                        node,
                        &path,
                        "an aligned dimension's two points lie apart".to_owned(),
                    ));
                }
                [d[0] / length, d[1] / length]
            };
            let n = [-u[1], u[0]];
            let across = |p: &Point3| (location.x - p.x) * n[0] + (location.y - p.y) * n[1];
            let measures = ((p2.x - p1.x) * u[0] + (p2.y - p1.y) * u[1]).abs();
            if measures == 0.0 {
                return Err(refused(
                    node,
                    &path,
                    "a linear dimension's two points lie apart along its angle; it measures 0"
                        .to_owned(),
                ));
            }
            if across(p1) == 0.0 || across(p2) == 0.0 {
                return Err(refused(
                    location_node,
                    &location_path,
                    "the dimension line passes through one of the dimension's points, which \
                     leaves its extension line no direction; put its location to one side"
                        .to_owned(),
                ));
            }
            flat(&[*p1, *p2, location], node, &path)?;
            // Kept where the dimension line meets the second extension line.
            let at = across(p2);
            let kept = Point3::new(p2.x + n[0] * at, p2.y + n[1] * at, p2.z);
            let measured = if kind == DimensionKind::Linear {
                Measured::Linear {
                    points: [*p1, *p2],
                    angle: match fields.get("angle") {
                        Some(node) => angle(node, &fields.path_of("angle"))?,
                        None => 0.0,
                    },
                }
            } else {
                Measured::Aligned { points: [*p1, *p2] }
            };
            return finish(fields, measured, kept, layer, units, dimension_styles);
        }
        DimensionKind::Radius | DimensionKind::Diameter => {
            let center_node = fields.require("center")?;
            let center = point(center_node, &fields.path_of("center"), units)?;
            let point_node = fields.require("point")?;
            let point_path = fields.path_of("point");
            let on = point(point_node, &point_path, units)?;
            let radius = (on.x - center.x).hypot(on.y - center.y);
            if radius == 0.0 {
                return Err(refused(
                    point_node,
                    &point_path,
                    "a point on the circle lies apart from its centre".to_owned(),
                ));
            }
            let reach = (location.x - center.x).hypot(location.y - center.y);
            if reach == 0.0 {
                return Err(refused(
                    location_node,
                    &location_path,
                    "the text's location is the circle's centre, which gives its line no \
                     direction"
                        .to_owned(),
                ));
            }
            flat(&[center, on, location], point_node, &point_path)?;
            // The arrow points where the line from the centre through the
            // location meets the circle.
            let k = radius / reach;
            let arrow = Point3::new(
                center.x + (location.x - center.x) * k,
                center.y + (location.y - center.y) * k,
                center.z,
            );
            if kind == DimensionKind::Radius {
                Measured::Radius {
                    center,
                    point: arrow,
                }
            } else {
                Measured::Diameter {
                    center,
                    point: arrow,
                }
            }
        }
        DimensionKind::Angular | DimensionKind::ThreePointAngular => {
            let (measured, node, path) = if kind == DimensionKind::Angular {
                let node = fields.require("lines")?;
                let path = fields.path_of("lines");
                let lines = two_lines(node, &path, units)?;
                (Measured::Angular { lines }, node, path)
            } else {
                let center_node = fields.require("center")?;
                let center = point(center_node, &fields.path_of("center"), units)?;
                let node = fields.require("points")?;
                let path = fields.path_of("points");
                let read = points(node, &path, units, 2)?;
                let [p1, p2] = read.as_slice() else {
                    return Err(refused(
                        node,
                        &path,
                        format!(
                            "a three-point angular dimension measures between two points; this \
                             lists {}",
                            read.len()
                        ),
                    ));
                };
                for p in [p1, p2] {
                    if p.x == center.x && p.y == center.y {
                        return Err(refused(
                            node,
                            &path,
                            "each of a three-point angular dimension's points lies apart from \
                             its centre"
                                .to_owned(),
                        ));
                    }
                }
                let measured = Measured::ThreePointAngular {
                    center,
                    points: [*p1, *p2],
                };
                (measured, node, path)
            };
            let probe = Dimension {
                measured: measured.clone(),
                location,
                dimension_style: String::new(),
                text: None,
                continued: false,
                layer: String::new(),
                properties: Default::default(),
            };
            let mut flat_points = probe.points();
            flat_points.push(location);
            flat(&flat_points, node, &path)?;
            let Some(sector) = probe.sector() else {
                return Err(refused(
                    location_node,
                    &location_path,
                    if kind == DimensionKind::Angular {
                        "an angular dimension's lines meet, at an angle, and its location lies \
                         apart from where they meet"
                    } else {
                        "a three-point angular dimension's points lie at an angle to each other \
                         round its centre, and its location apart from the centre"
                    }
                    .to_owned(),
                ));
            };
            // Kept on the arc a third of the way round, as AutoCAD keeps it.
            return finish(
                fields,
                measured,
                sector.arc_point(),
                layer,
                units,
                dimension_styles,
            );
        }
        DimensionKind::Ordinate => {
            let node = fields.require("feature")?;
            let path = fields.path_of("feature");
            let feature = point(node, &path, units)?;
            let listed =
                "an ordinate dimension measures a feature's `x` or its `y`, from the origin";
            let axis = named(
                fields,
                "axis",
                "an axis",
                &OrdinateAxis::ALL,
                OrdinateAxis::name,
                listed,
            )?
            .expect("the kind requires its axis");
            flat(&[feature, location], node, &path)?;
            if feature.x == location.x && feature.y == location.y {
                return Err(refused(
                    location_node,
                    &location_path,
                    "an ordinate dimension's location, the end of its leader, lies apart from \
                     its feature"
                        .to_owned(),
                ));
            }
            Measured::Ordinate { feature, axis }
        }
    };
    finish(fields, measured, location, layer, units, dimension_styles)
}

/// An angular dimension's two lines, each through two points apart.
fn two_lines(node: &Node, path: &str, units: Unit) -> Result<[[Point3; 2]; 2], Error> {
    let says = "an angular dimension measures between two lines, each a list of two points";
    let items = sequence(node, path, "a list of two lines")?;
    if items.len() != 2 {
        return Err(bad_value(node, path, says.to_owned()));
    }
    let mut lines = [[Point3::new(0.0, 0.0, 0.0); 2]; 2];
    for (i, item) in items.iter().enumerate() {
        let line_path = index_path(path, i);
        let read = points(item, &line_path, units, 2)?;
        let [a, b] = read.as_slice() else {
            return Err(bad_value(item, &line_path, says.to_owned()));
        };
        if a.x == b.x && a.y == b.y {
            return Err(bad_value(
                item,
                &line_path,
                "a line's two points lie apart".to_owned(),
            ));
        }
        lines[i] = [*a, *b];
    }
    Ok(lines)
}

/// Refuses points of a dimension at different heights: a dimension lies
/// flat.
fn flat(points: &[Point3], node: &Node, path: &str) -> Result<(), Error> {
    if points.iter().any(|p| p.z != points[0].z) {
        return Err(bad_value(
            node,
            path,
            "a dimension's points and its location lie at one height; a dimension lies flat",
        ));
    }
    Ok(())
}

fn finish(
    fields: &Fields,
    measured: Measured,
    location: Point3,
    layer: String,
    units: Unit,
    dimension_styles: &[DimensionStyle],
) -> Result<Dimension, Error> {
    Ok(Dimension {
        measured,
        location,
        dimension_style: dimension_style_name(fields, units, dimension_styles)?,
        text: own_text(fields)?,
        continued: match fields.get("continued") {
            Some(node) => boolean(node, &fields.path_of("continued"))?,
            None => false,
        },
        layer,
        properties: Default::default(),
    })
}

/// A dimension's own text, where `<>` stands for its measured value: one
/// line that shows something, with no code AutoCAD reads in it.
fn own_text(fields: &Fields) -> Result<Option<String>, Error> {
    let Some(node) = fields.text("text") else {
        return Ok(None);
    };
    let path = fields.path_of("text");
    let text = scalar(node, &path, "the text")?;
    let problem = if text.contains('\n') {
        Some("a dimension's text is one line; text over more lines waits".to_owned())
    } else if text.contains('\\') || text.contains('{') || text.contains('}') {
        Some(
            "a dimension's text holds no `\\`, `{` or `}`, which AutoCAD reads as codes in a \
             dimension's text"
                .to_owned(),
        )
    } else if text.contains("%%") {
        Some("a dimension's text holds no `%%`, which AutoCAD reads as a control code".to_owned())
    } else {
        contents_problem(text).or_else(|| character_code(text))
    };
    match problem {
        Some(problem) => Err(bad_value(node, &path, problem)),
        None => Ok(Some(text.to_owned())),
    }
}

/// The dimension style a dimension draws with: its unit's own or one of
/// the sheet's.
fn dimension_style_name(
    fields: &Fields,
    units: Unit,
    styles: &[DimensionStyle],
) -> Result<String, Error> {
    let own = DimensionStyle::default_name(units);
    let Some(node) = fields.text("dimension_style") else {
        return Ok(own.to_owned());
    };
    let path = fields.path_of("dimension_style");
    let mut choices = vec![own];
    choices.extend(styles.iter().map(|s| s.name.as_str()).filter(|n| *n != own));
    let listed = format!("the sheet's dimension styles: {}", choices.join(", "));
    let text = choice(node, &path, "the name of a dimension style", &listed)?;
    if choices.contains(&text) {
        return Ok(text.to_owned());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown dimension style `{text}`; {listed}"),
        ),
        &path,
        node.at,
    ))
}
