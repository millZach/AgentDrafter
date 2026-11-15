//! The schema of dimension styles and dimensions, part of the sheet
//! description's schema ([`super::schema`]).

use serde_json::{json, Map, Value};

use super::dimension::{DECIMAL, MOST_DECIMALS};
use super::schema::{object_def, optional_text, POSITIVE_NUMBER, TEXT_TYPES};
use crate::model::{
    Arrowhead, DecimalSeparator, DimensionKind, TextPlacement, ISO_DIMENSION_STYLE,
    STANDARD_DIMENSION_STYLE,
};
use crate::text::STANDARD;

/// The `dimension_styles` key of a sheet.
pub(super) fn dimension_styles_property() -> Value {
    json!({
        "description": format!(
            "The sheet's dimension styles, from name to dimension style: what a dimension \
             draws with. A sheet has its unit's own without listing it, as AutoCAD 2026's \
             templates have it: `{ISO_DIMENSION_STYLE}` on a metric sheet, arrows and text \
             2.5 mm, a gap of 0.625 mm, extension lines offset 0.625 mm and reaching 1.25 mm \
             past the dimension line, a centre mark 2.5 mm, text above its line, two decimals \
             after a comma with trailing zeros suppressed; `{STANDARD_DIMENSION_STYLE}` on an \
             imperial sheet, arrows and text 0.18 in, a gap of 0.09 in, extension lines offset \
             0.0625 in and reaching 0.18 in past, a centre mark 0.09 in, text centred on its \
             line, four decimals after a period. Listing it sets what it draws with, and any \
             other style starts from it. AutoCAD's names are written exactly: in another case \
             they are an error."
        ),
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/dimension_style" },
    })
}

fn length(def: &str, description: &str) -> Value {
    json!({ "$ref": format!("#/$defs/{def}"), "description": description })
}

fn plain_text(description: &str) -> Value {
    optional_text(json!({
        "description": description,
        "type": TEXT_TYPES,
        "not": { "type": "string", "pattern": "[<>{}\\\\\\n]" },
    }))
}

pub(super) fn dimension_defs(defs: &mut Map<String, Value>) {
    defs.insert(
        "dimension_style".into(),
        json!({
            "description": "A dimension style, AutoCAD's DIMSTYLE: what a dimension draws \
                            with. Each key left out is as the sheet's unit's own style has it. \
                            Every length is multiplied by `scale`.",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "arrowhead": {
                    "description": "The arrowhead at each end of the dimension line, of \
                                    AutoCAD's named arrowheads that AgentDrafter draws \
                                    exactly: closed_filled, a filled triangle; dot, a filled \
                                    dot as wide as the arrow is long; open, the triangle's \
                                    sides; none. Without it, closed_filled.",
                    "enum": Arrowhead::ALL.map(Arrowhead::name),
                },
                "arrow_size": length("length_more_than_0", "How long an arrowhead is: AutoCAD's DIMASZ, a length more than 0."),
                "text_style": optional_text(json!({
                    "description": format!(
                        "The text style the text draws with: `{STANDARD}`, which every drawing \
                         has, or a name under `text_styles`, written exactly as it is there. \
                         Without it, `{STANDARD}`."
                    ),
                    "type": TEXT_TYPES,
                })),
                "text_height": length("length_more_than_0", "The height of the text's capitals: AutoCAD's DIMTXT, a length more than 0."),
                "gap": length("length_not_negative", "The space round the text, between it and the dimension line: AutoCAD's DIMGAP, a length of 0 or more."),
                "extension_offset": length("length_not_negative", "How far an extension line starts from the point it extends: AutoCAD's DIMEXO, a length of 0 or more."),
                "extension_beyond": length("length_not_negative", "How far an extension line reaches past the dimension line: AutoCAD's DIMEXE, a length of 0 or more."),
                "line_extension": length("length_not_negative", "How far the dimension line reaches past the extension lines when its arrowhead is none: AutoCAD's DIMDLE, a length of 0 or more. Without it, 0."),
                "center_mark": length("length_not_negative", "How far the arms of the cross that marks a circle's centre reach from it, drawn when a radius or diameter dimension's text, centred, is outside the circle: AutoCAD's DIMCEN, a length of 0 or more, 0 for none."),
                "placement": {
                    "description": "Where the text sits, one of AutoCAD's two defaults: \
                                    centered, horizontal in the middle of the dimension line, \
                                    which breaks for it, as Standard has it; above, aligned \
                                    with the dimension line and above it, which runs between \
                                    the extension lines even when the arrowheads are outside \
                                    them, as ISO-25 has it.",
                    "enum": TextPlacement::ALL.map(TextPlacement::name),
                },
                "unit_format": {
                    "description": "How the value is written: decimal, AutoCAD's DIMLUNIT 2. \
                                    Scientific, engineering, architectural and fractional \
                                    wait.",
                    "enum": [DECIMAL],
                },
                "precision": {
                    "description": format!("How many decimals the value shows: AutoCAD's DIMDEC, a whole number from 0 to {MOST_DECIMALS}."),
                    "anyOf": [
                        { "type": "integer", "minimum": 0, "maximum": MOST_DECIMALS },
                        { "type": "string", "pattern": format!("^[0-{MOST_DECIMALS}]$") },
                    ],
                },
                "decimal_separator": {
                    "description": "The character between the value's whole part and its \
                                    decimals: AutoCAD's DIMDSEP.",
                    "enum": DecimalSeparator::ALL.map(DecimalSeparator::name),
                },
                "suppress_leading_zeros": {
                    "description": "Whether 0.5 shows as .5: AutoCAD's DIMZIN 4.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "suppress_trailing_zeros": {
                    "description": "Whether 2.50 shows as 2.5, and 2.00 as 2: AutoCAD's DIMZIN 8.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "prefix": plain_text("What comes before every value the style shows: AutoCAD's DIMPOST before `<>`. One line, with no `<`, `>`, `\\`, `{` or `}`. A prefix or a suffix takes the place of a radius's R and a diameter's ∅, as in AutoCAD."),
                "suffix": plain_text("What comes after every value the style shows, such as ` mm`: AutoCAD's DIMPOST after `<>`. One line, with no `<`, `>`, `\\`, `{` or `}`."),
                "scale": {
                    "description": "What every length of the style is multiplied by: \
                                    AutoCAD's DIMSCALE, a number more than 0, with no unit. \
                                    Without it, 1.",
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0 },
                        { "type": "string", "pattern": format!("^{POSITIVE_NUMBER}$") },
                    ],
                },
            },
        }),
    );
    let kinds = DimensionKind::ALL.map(DimensionKind::name);
    defs.insert(
        "dimension".into(),
        object_def(
            "A dimension, AutoCAD's DIMENSION, drawn as AutoCAD 2026 draws its dimension's \
             block: of `kind` linear, the distance between two points along a line at an \
             angle (DIMLINEAR); aligned, the distance between two points (DIMALIGNED); \
             radius (DIMRADIUS) or diameter (DIMDIAMETER) of a circle or an arc, by its centre \
             and a point on it. Its text shows the measured value as its dimension style \
             formats it, a radius's after R and a diameter's after ∅. Text and arrowheads sit \
             between the extension lines when they fit, as AutoCAD's best fit places them. A \
             dimension belongs on a sheet, not in a block.",
            &["kind", "location"],
            json!({
                "kind": {
                    "description": "What the dimension measures. Angular, ordinate, baseline \
                                    and continued dimensions wait.",
                    "enum": kinds,
                },
                "points": {
                    "description": "A linear or aligned dimension's two points, where its \
                                    extension lines start from: the first, then the second, \
                                    beyond which text that does not fit goes. They lie apart \
                                    along the dimension line.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/point" },
                    "minItems": 2,
                    "maxItems": 2,
                },
                "angle": {
                    "$ref": "#/$defs/angle",
                    "description": "A linear dimension's angle: the direction it measures \
                                    along, in degrees anticlockwise from the X axis; 0 \
                                    measures horizontally and 90 vertically. Without it, 0.",
                },
                "center": {
                    "$ref": "#/$defs/point",
                    "description": "A radius or diameter dimension's circle's or arc's centre.",
                },
                "point": {
                    "$ref": "#/$defs/point",
                    "description": "A point on a radius or diameter dimension's circle or \
                                    arc, apart from its centre: its distance from the centre \
                                    is the radius. The arrow points where the line from the \
                                    centre through the location meets the circle, as AutoCAD's \
                                    DIMRADIUS puts it.",
                },
                "location": {
                    "$ref": "#/$defs/point",
                    "description": "Where the dimension line goes, as AutoCAD's command asks \
                                    for it: for a linear or aligned dimension, a point the \
                                    dimension line passes through, to one side of both points; \
                                    for a radius or diameter dimension, where the text goes, \
                                    inside or outside the circle, its line running from the \
                                    centre through it. AutoCAD moves text nearer the circle \
                                    than two arrowheads' lengths, and AgentDrafter as it does. \
                                    The points and the location lie at one height.",
                },
                "dimension_style": optional_text(json!({
                    "description": format!(
                        "The dimension style it draws with: its unit's own, `{ISO_DIMENSION_STYLE}` \
                         on a metric sheet and `{STANDARD_DIMENSION_STYLE}` on an imperial one, or \
                         a name under `dimension_styles`. Without it, its unit's own."
                    ),
                    "type": TEXT_TYPES,
                })),
                "text": optional_text(json!({
                    "description": "The dimension's own text, `<>` standing for the measured \
                                    value, as in AutoCAD: `<> TYP` shows `25 TYP`. One line \
                                    that shows something, with no `\\`, `{`, `}` or `%%`. \
                                    Without it, the measured value.",
                    "type": TEXT_TYPES,
                    "not": { "type": "string", "pattern": "[{}\\\\\\n]|%%" },
                })),
            }),
        ),
    );
}
