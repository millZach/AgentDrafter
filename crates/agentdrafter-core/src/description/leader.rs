//! Reads multileader styles, multileaders and older leaders.
//!
//! A multileader style is a mapping from its name to what its multileaders
//! draw with, like a table style. A multileader and an older leader are
//! objects of a sheet: `objects.rs` lists them and hands their keys to
//! [`mleader`] and [`leader`] here. Both belong on a sheet, not in a block.

use super::layers::{name_error, named_entries, object_color, object_lineweight};
use super::strict_yaml::{index_path, key_path, Node};
use super::text::{contents_problem, text_style_at};
use super::{at, bad_value, choice, length, points, scalar, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Arrowhead, Leader, LeaderAnnotation, LeaderAttachment, MLeader, MLeaderContent, MLeaderStyle,
    Point3, STANDARD_MLEADER_STYLE,
};
use crate::text::TextStyle;
use crate::units::Unit;

/// The key a multileader style names its text style under.
const TEXT_STYLE_KEY: &str = "text_style";

/// The keys a multileader style takes.
pub(crate) const MLEADER_STYLE_KEYS: [&str; 9] = [
    TEXT_STYLE_KEY,
    "text_height",
    "arrowhead",
    "arrow_size",
    "landing",
    "landing_gap",
    "attachment",
    "leader_color",
    "leader_lineweight",
];

/// The keys a multileader takes.
pub(crate) const MLEADER_KEYS: [&str; 11] = [
    "leaders",
    "content",
    "landing",
    "arrowhead",
    "arrow_size",
    "mleader_style",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The keys a multileader's content takes.
pub(crate) const CONTENT_KEYS: [&str; 4] = ["text", "style", "height", "attachment"];

/// The keys an older leader takes.
pub(crate) const LEADER_KEYS: [&str; 10] = [
    "points",
    "arrowhead",
    "arrow_size",
    "gap",
    "annotation",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The keys an older leader's annotation takes.
pub(crate) const ANNOTATION_KEYS: [&str; 3] = ["text", "style", "height"];

pub(super) fn mleader_styles(
    node: &Node,
    units: Unit,
    text_styles: &[TextStyle],
) -> Result<Vec<MLeaderStyle>, Error> {
    let entries = named_entries(node, "mleader_styles", "multileader style")?;
    let mut styles = Vec::with_capacity(entries.len());
    for entry in entries {
        // As for text styles: `Standard` is written exactly.
        if entry.key != STANDARD_MLEADER_STYLE
            && entry.key.eq_ignore_ascii_case(STANDARD_MLEADER_STYLE)
        {
            return Err(name_error(
                entry,
                "mleader_styles",
                format!(
                    "`{}` is the multileader style `{STANDARD_MLEADER_STYLE}`, which every drawing has, written in another case, and AutoCAD ignores case in names; to set what it draws with, list it as `{STANDARD_MLEADER_STYLE}`",
                    entry.key
                ),
            ));
        }
        let path = key_path("mleader_styles", &entry.key);
        let fields = Fields::of(
            &entry.value,
            &path,
            &MLEADER_STYLE_KEYS,
            "a multileader style",
        )?;
        let mut style = MLeaderStyle::standard(units);
        entry.key.clone_into(&mut style.name);
        style.text_style = text_style_at(&fields, TEXT_STYLE_KEY, text_styles)?;
        if let Some(height) = positive(&fields, "text_height", units, "a text height")? {
            style.text_height = height;
        }
        if let Some(arrowhead) = arrowhead(&fields)? {
            style.arrowhead = arrowhead;
        }
        if let Some(size) = positive(&fields, "arrow_size", units, "an arrow's size")? {
            style.arrow_size = size;
        }
        if let Some(landing) = not_negative(&fields, "landing", units, "a landing's length")? {
            style.landing = landing;
        }
        if let Some(gap) = not_negative(&fields, "landing_gap", units, "a landing's gap")? {
            style.landing_gap = gap;
        }
        if let Some(attachment) = attachment(&fields)? {
            style.attachment = attachment;
        }
        if let Some(node) = fields.get("leader_color") {
            style.leader_color = object_color(node, &fields.path_of("leader_color"))?;
        }
        if let Some(node) = fields.get("leader_lineweight") {
            style.leader_lineweight =
                object_lineweight(node, &fields.path_of("leader_lineweight"))?;
        }
        styles.push(style);
    }
    Ok(styles)
}

pub(super) fn mleader(
    fields: &Fields,
    units: Unit,
    layer: String,
    text_styles: &[TextStyle],
    mleader_styles: &[MLeaderStyle],
) -> Result<MLeader, Error> {
    let mleader_style = mleader_style_name(fields, mleader_styles)?;
    let style = mleader_styles
        .iter()
        .find(|s| s.name == mleader_style)
        .cloned()
        .unwrap_or_else(|| MLeaderStyle::standard(units));
    let arrowhead = arrowhead(fields)?;
    let arrow_size = positive(fields, "arrow_size", units, "an arrow's size")?;
    let leaders_node = fields.require("leaders")?;
    let leaders_path = fields.path_of("leaders");
    let listed = super::sequence(
        leaders_node,
        &leaders_path,
        "a list of leader lines, each a list of points",
    )?;
    if listed.is_empty() {
        return Err(bad_value(
            leaders_node,
            &leaders_path,
            "a multileader has at least one leader line",
        ));
    }
    let mut leaders = Vec::with_capacity(listed.len());
    for (i, item) in listed.iter().enumerate() {
        let path = index_path(&leaders_path, i);
        let line = leader_points(item, &path, units)?;
        if let Some(first) = leaders.first().and_then(|l: &Vec<Point3>| l.last()) {
            let end = line[line.len() - 1];
            if end != *first {
                return Err(bad_value(
                    &listed[i],
                    &index_path(&path, line.len() - 1),
                    format!(
                        "every leader line of a multileader ends where its landing starts, at the first one's last point, [{}, {}, {}]; a multileader with landings in two places waits",
                        first.x, first.y, first.z
                    ),
                ));
            }
        }
        check_arrow(
            item,
            &path,
            &line,
            arrowhead.unwrap_or(style.arrowhead),
            arrow_size.unwrap_or(style.arrow_size),
        )?;
        leaders.push(line);
    }
    let content_node = fields.require("content")?;
    let content_path = fields.path_of("content");
    let content_fields = Fields::of(
        content_node,
        &content_path,
        &CONTENT_KEYS,
        "a multileader's content",
    )?;
    let content = MLeaderContent {
        text: text_of(&content_fields)?,
        style: match content_fields.text("style") {
            Some(_) => Some(text_style_at(&content_fields, "style", text_styles)?),
            None => None,
        },
        height: positive(&content_fields, "height", units, "a text height")?,
        attachment: attachment(&content_fields)?,
    };
    Ok(MLeader {
        leaders,
        content,
        landing: not_negative(fields, "landing", units, "a landing's length")?,
        arrowhead,
        arrow_size,
        mleader_style,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn leader(
    fields: &Fields,
    units: Unit,
    layer: String,
    text_styles: &[TextStyle],
) -> Result<Leader, Error> {
    let (default_size, default_gap) = Leader::default_sizes(units);
    let arrowhead = arrowhead(fields)?.unwrap_or_default();
    let arrow_size =
        positive(fields, "arrow_size", units, "an arrow's size")?.unwrap_or(default_size);
    let points_node = fields.require("points")?;
    let path = fields.path_of("points");
    let points = leader_points(points_node, &path, units)?;
    check_arrow(points_node, &path, &points, arrowhead, arrow_size)?;
    let annotation = match fields.text("annotation") {
        Some(node) => {
            let path = fields.path_of("annotation");
            let annotation = Fields::of(node, &path, &ANNOTATION_KEYS, "a leader's annotation")?;
            Some(LeaderAnnotation {
                text: text_of(&annotation)?,
                style: text_style_at(&annotation, "style", text_styles)?,
                height: positive(&annotation, "height", units, "a text height")?
                    .unwrap_or(default_size),
            })
        }
        None => None,
    };
    Ok(Leader {
        points,
        arrowhead,
        arrow_size,
        gap: not_negative(fields, "gap", units, "a gap")?.unwrap_or(default_gap),
        annotation,
        layer,
        properties: Default::default(),
    })
}

/// A leader line's points: at least two, each apart from the one before.
fn leader_points(node: &Node, path: &str, units: Unit) -> Result<Vec<Point3>, Error> {
    let points = points(node, path, units, 2)?;
    for (i, pair) in points.windows(2).enumerate() {
        if pair[0] == pair[1] {
            return Err(at(
                Error::new(
                    ErrorCode::BadValue,
                    "a leader line's point is the one before it again; each point lies apart from the one before",
                ),
                &index_path(path, i + 1),
                node.at,
            ));
        }
    }
    Ok(points)
}

/// Refuses a first segment shorter than the arrowhead, behind which AutoCAD
/// starts the line.
fn check_arrow(
    node: &Node,
    path: &str,
    points: &[Point3],
    arrowhead: Arrowhead,
    size: f64,
) -> Result<(), Error> {
    if !arrowhead.trims_line() {
        return Ok(());
    }
    let (a, b) = (points[0], points[1]);
    let first = (b.x - a.x).hypot(b.y - a.y);
    if first < size {
        return Err(bad_value(
            node,
            path,
            format!(
                "a leader line's first segment is {first} long, shorter than its arrow, {size}; AutoCAD draws the line from behind the arrowhead, so the first segment is at least as long as the arrow"
            ),
        ));
    }
    Ok(())
}

/// The text a leader shows: multiline text's contents.
fn text_of(fields: &Fields) -> Result<String, Error> {
    let node = fields.require_text("text")?;
    let path = fields.path_of("text");
    let written = scalar(node, &path, "the text")?;
    // A block scalar ends with a line break; text does not end with an empty line.
    let text = written.trim_end_matches(['\n', '\r']).replace("\r\n", "\n");
    if let Some(problem) = contents_problem(&text) {
        return Err(bad_value(node, &path, problem));
    }
    Ok(text)
}

/// The length under `key`, more than 0, if it is given. `what` names it in
/// the error, such as "a text height".
fn positive(fields: &Fields, key: &str, units: Unit, what: &str) -> Result<Option<f64>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let value = length(node, &path, units)?;
    if value > 0.0 {
        Ok(Some(value))
    } else {
        Err(bad_value(node, &path, format!("{what} is more than 0")))
    }
}

/// The length under `key`, 0 or more, if it is given.
fn not_negative(fields: &Fields, key: &str, units: Unit, what: &str) -> Result<Option<f64>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let value = length(node, &path, units)?;
    if value >= 0.0 {
        Ok(Some(value + 0.0))
    } else {
        Err(bad_value(node, &path, format!("{what} is 0 or more")))
    }
}

/// The arrowhead under `arrowhead`, if it is given.
fn arrowhead(fields: &Fields) -> Result<Option<Arrowhead>, Error> {
    let Some(node) = fields.get("arrowhead") else {
        return Ok(None);
    };
    let path = fields.path_of("arrowhead");
    let names = Arrowhead::ALL.map(Arrowhead::name);
    let listed = format!("an arrowhead is {}", names.join(", "));
    let text = choice(node, &path, "an arrowhead", &listed)?;
    match Arrowhead::ALL.into_iter().find(|a| a.name() == text) {
        Some(a) => Ok(Some(a)),
        None => Err(bad_value(
            node,
            &path,
            format!("`{text}` is not an arrowhead the pipeline draws; {listed}"),
        )),
    }
}

/// The attachment under `attachment`, if it is given.
fn attachment(fields: &Fields) -> Result<Option<LeaderAttachment>, Error> {
    let Some(node) = fields.get("attachment") else {
        return Ok(None);
    };
    let path = fields.path_of("attachment");
    let names = LeaderAttachment::ALL.map(LeaderAttachment::name);
    let listed = format!(
        "a landing meets its text at {}; AutoCAD's attachments that underline the text wait",
        names.join(", ")
    );
    let text = choice(node, &path, "an attachment", &listed)?;
    match LeaderAttachment::ALL.into_iter().find(|a| a.name() == text) {
        Some(a) => Ok(Some(a)),
        None => Err(bad_value(
            node,
            &path,
            format!("`{text}` is not an attachment; {listed}"),
        )),
    }
}

/// The multileader style a multileader draws with: `Standard` or one of the
/// sheet's.
fn mleader_style_name(fields: &Fields, styles: &[MLeaderStyle]) -> Result<String, Error> {
    let Some(node) = fields.text("mleader_style") else {
        return Ok(STANDARD_MLEADER_STYLE.to_owned());
    };
    let path = fields.path_of("mleader_style");
    let mut choices = vec![STANDARD_MLEADER_STYLE];
    choices.extend(
        styles
            .iter()
            .map(|s| s.name.as_str())
            .filter(|n| *n != STANDARD_MLEADER_STYLE),
    );
    let listed = format!("the sheet's multileader styles: {}", choices.join(", "));
    let text = choice(node, &path, "the name of a multileader style", &listed)?;
    if choices.contains(&text) {
        return Ok(text.to_owned());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown multileader style `{text}`; {listed}"),
        ),
        &path,
        node.at,
    ))
}
