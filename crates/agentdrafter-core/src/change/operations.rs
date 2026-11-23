//! The operations a change set holds: their names, their AutoCAD aliases,
//! their schemas, and how one sent as plain data is read.
//!
//! Every object a sheet description holds has an operation that adds it,
//! named after the object and aliased to the AutoCAD command that draws it.
//! `change`, `move`, `delete` and `set_attribute` edit an object an agent
//! names by its ID or by its name, `set_cell`, `add_row`, `delete_row`,
//! `add_column` and `delete_column` a table's grid, and `add_leader_line`
//! and `delete_leader_line` a multileader's leader lines, and
//! `add_baseline_dimension` and `add_continued_dimension` add a dimension
//! from another, as AutoCAD's DIMBASELINE and DIMCONTINUE do.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::description::schema::{block_library_schema, sheet_description_schema};
use crate::description::{
    article, kind_keys, objects_table, rows_schema, template_schema, value_name_problem, Axis,
    LeaderLineEdit, TableEdit, CELL_KEYS, DIMENSION_STYLE_KEYS, LAYER_KEYS, LINETYPE_KEYS,
    TEXT_STYLE_KEYS, VALUE_NAME,
};
use crate::error::{Error, ErrorCode};
use crate::geometry::{SnapKind, SnapRef};
use crate::model::{DimensionKind, ISO_DIMENSION_STYLE, STANDARD_DIMENSION_STYLE};
use crate::query::snap::{check_shape, no_kind};
use crate::units::{parse_length, LengthError, Unit};

/// What an operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Adds an object of the type it names; a dimension of the kind it
    /// names, when it names one.
    Add(&'static str, Option<&'static str>),
    Change,
    /// Sets a viewport's scale, view centre or display lock.
    ChangeViewport,
    /// Clips a viewport to a boundary, or takes its boundary away:
    /// AutoCAD's VPCLIP.
    ClipViewport,
    /// Sets the layers frozen in a viewport: AutoCAD's VPLAYER.
    SetFrozenLayers,
    Move,
    Delete,
    /// Copies objects by one offset or several: AutoCAD's COPY.
    Copy,
    /// Copies objects in rows and columns, round a centre or along a path:
    /// AutoCAD's ARRAY, as plain copies.
    Array,
    /// Turns objects about a base point: AutoCAD's ROTATE.
    Rotate,
    /// Scales objects about a base point: AutoCAD's SCALE.
    Scale,
    /// Mirrors objects across a line, as copies or in place: AutoCAD's
    /// MIRROR.
    Mirror,
    SetAttribute,
    /// Adds, changes, renames or deletes a layer, a linetype or a text
    /// style of a template.
    Table(TableOp, Table),
    /// Adds or redefines a block of a block library, or brings the inserts
    /// of its blocks into line with their definitions.
    Block(BlockOp),
    /// Sets a cell of a table, or adds or deletes a row or a column.
    Grid(GridOp),
    /// Adds a leader line to a multileader, or deletes one: AutoCAD's
    /// MLEADEREDIT.
    LeaderLine(LeaderLineOp),
    /// Adds a dimension from another: AutoCAD's DIMBASELINE or
    /// DIMCONTINUE.
    DimensionFrom(FromOp),
}

/// How a dimension added from another is added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FromOp {
    /// From the other's first extension line, its line further out:
    /// AutoCAD's DIMBASELINE.
    Baseline,
    /// From the other's second extension line, on its line: AutoCAD's
    /// DIMCONTINUE.
    Continued,
}

/// What an operation on a multileader's leader lines does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaderLineOp {
    Add,
    Delete,
}

/// What an operation on a table's grid does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridOp {
    /// Sets keys of a cell: AutoCAD's TABLEDIT.
    SetCell,
    /// Adds a row or a column.
    Add(Axis),
    /// Deletes a row or a column.
    Delete(Axis),
}

/// What a block operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockOp {
    /// Adds a block: AutoCAD's BLOCK.
    Add,
    /// Replaces a block's whole definition, then brings its inserts into
    /// line: AutoCAD's BEDIT.
    Redefine,
    /// Brings inserts into line with their block's definition alone:
    /// AutoCAD's ATTSYNC.
    Sync,
    /// Imports a drawing's blocks, each with every block it nests: AutoCAD's
    /// DesignCenter, ADCENTER.
    Import,
}

/// A table of a template's definitions that an operation edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Table {
    Layer,
    Linetype,
    TextStyle,
    DimensionStyle,
}

impl Table {
    /// The key the table sits under in a template, such as `layers`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Table::Layer => "layers",
            Table::Linetype => "linetypes",
            Table::TextStyle => "text_styles",
            Table::DimensionStyle => "dimension_styles",
        }
    }

    /// One entry, in words, such as `text style`.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "text style",
            Table::DimensionStyle => "dimension style",
        }
    }

    /// The definition of one entry in the sheet description's schema.
    fn def(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "text_style",
            Table::DimensionStyle => "dimension_style",
        }
    }

    /// The keys one entry takes, in the order a sheet description writes
    /// them.
    pub(crate) fn entry_keys(self) -> &'static [&'static str] {
        match self {
            Table::Layer => &LAYER_KEYS,
            Table::Linetype => &LINETYPE_KEYS,
            Table::TextStyle => &TEXT_STYLE_KEYS,
            Table::DimensionStyle => &DIMENSION_STYLE_KEYS,
        }
    }

    /// The names every drawing has in the table, which are never renamed
    /// or deleted: layer `0`, `Continuous` (with AutoCAD's `ByLayer` and
    /// `ByBlock`), `Standard`, and AutoCAD's two dimension styles,
    /// `Standard` and `ISO-25`, one of which every sheet draws its
    /// dimensions with unless they name another.
    pub(crate) fn built_in(self) -> &'static [&'static str] {
        match self {
            Table::Layer => &["0"],
            Table::Linetype => &["Continuous", "ByLayer", "ByBlock"],
            Table::TextStyle => &["Standard"],
            Table::DimensionStyle => &[STANDARD_DIMENSION_STYLE, ISO_DIMENSION_STYLE],
        }
    }

    /// The key an object, or a layer, names an entry by: `layer`,
    /// `linetype`, `style` or `dimension_style`.
    pub(crate) fn named_by(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "style",
            Table::DimensionStyle => "dimension_style",
        }
    }
}

/// What a table operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableOp {
    Add,
    Change,
    Rename,
    Delete,
}

/// One operation: its name, the AutoCAD command it is aliased to, and what
/// it does.
#[derive(Debug)]
pub(crate) struct Spec {
    pub name: &'static str,
    pub alias: &'static str,
    pub kind: Kind,
}

impl Spec {
    /// The AutoCAD commands besides its alias that name it: ARRAYRECT,
    /// ARRAYPOLAR and ARRAYPATH for `array`, each of which gives the array
    /// its kind, and the short forms AutoCAD's own aliases give ROTATE,
    /// SCALE and MIRROR: RO, SC and MI.
    pub(crate) fn other_aliases(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Array => &ARRAY_ALIASES,
            Kind::Rotate => &["RO"],
            Kind::Scale => &["SC"],
            Kind::Mirror => &["MI"],
            _ => &[],
        }
    }

    /// Whether `name`, as an operation is sent, names it: its name, or any
    /// of its aliases in any case.
    fn named(&self, name: &str) -> bool {
        self.name == name
            || (!self.alias.is_empty() && self.alias.eq_ignore_ascii_case(name))
            || self
                .other_aliases()
                .iter()
                .any(|a| a.eq_ignore_ascii_case(name))
    }

    /// Its name with its aliases, as an error lists the operations, such as
    /// `move (MOVE)`.
    fn listed(&self) -> String {
        let aliases: Vec<&str> = std::iter::once(self.alias)
            .filter(|a| !a.is_empty())
            .chain(self.other_aliases().iter().copied())
            .collect();
        if aliases.is_empty() {
            self.name.to_owned()
        } else {
            format!("{} ({})", self.name, aliases.join(", "))
        }
    }
}

/// The aliases of `array` besides ARRAY, each with the kind of array it
/// makes.
const ARRAY_ALIASES: [&str; 3] = ["ARRAYRECT", "ARRAYPOLAR", "ARRAYPATH"];

/// The kinds of array, as `kind` names them, in the order of
/// [`ARRAY_ALIASES`].
pub(crate) const ARRAY_KINDS: [&str; 3] = ["rectangular", "polar", "path"];

/// The keys each kind of array takes besides `target`, `targets` and
/// `kind`, in the order of [`ARRAY_KINDS`].
const ARRAY_KEYS: [&[&str]; 3] = [
    &["rows", "columns", "row_spacing", "column_spacing"],
    &["center", "count", "angle", "rotate", "base_point"],
    &["path", "count", "spacing", "align"],
];

/// The most copies one copy or array makes: its places times the objects
/// it copies.
pub(crate) const MOST_COPIES: usize = 10_000;

/// Every operation, in the order they are listed.
pub(crate) const SPECS: [Spec; 64] = [
    add("add_line", "LINE", "line"),
    add("add_polyline", "PLINE", "polyline"),
    add("add_arc", "ARC", "arc"),
    add("add_circle", "CIRCLE", "circle"),
    add("add_ellipse", "ELLIPSE", "ellipse"),
    add("add_spline", "SPLINE", "spline"),
    add("add_hatch", "HATCH", "hatch"),
    add("add_text", "TEXT", "text"),
    add("add_mtext", "MTEXT", "mtext"),
    add("add_donut", "DONUT", "donut"),
    add("add_solid", "SOLID", "solid"),
    add("insert_block", "INSERT", "insert"),
    add("add_table", "TABLE", "table"),
    add("add_mleader", "MLEADER", "mleader"),
    add("add_leader", "LEADER", "leader"),
    dimension("add_linear_dimension", "DIMLINEAR", "linear"),
    dimension("add_aligned_dimension", "DIMALIGNED", "aligned"),
    dimension("add_radius_dimension", "DIMRADIUS", "radius"),
    dimension("add_diameter_dimension", "DIMDIAMETER", "diameter"),
    dimension("add_angular_dimension", "DIMANGULAR", "angular"),
    dimension(
        "add_three_point_angular_dimension",
        "",
        "three_point_angular",
    ),
    dimension("add_ordinate_dimension", "DIMORDINATE", "ordinate"),
    add("add_viewport", "MVIEW", "viewport"),
    Spec {
        name: "add_baseline_dimension",
        alias: "DIMBASELINE",
        kind: Kind::DimensionFrom(FromOp::Baseline),
    },
    Spec {
        name: "add_continued_dimension",
        alias: "DIMCONTINUE",
        kind: Kind::DimensionFrom(FromOp::Continued),
    },
    Spec {
        name: "change",
        alias: "CHANGE",
        kind: Kind::Change,
    },
    Spec {
        name: "change_viewport",
        alias: "",
        kind: Kind::ChangeViewport,
    },
    Spec {
        name: "clip_viewport",
        alias: "VPCLIP",
        kind: Kind::ClipViewport,
    },
    Spec {
        name: "set_frozen_layers",
        alias: "VPLAYER",
        kind: Kind::SetFrozenLayers,
    },
    Spec {
        name: "move",
        alias: "MOVE",
        kind: Kind::Move,
    },
    Spec {
        name: "delete",
        alias: "ERASE",
        kind: Kind::Delete,
    },
    Spec {
        name: "copy",
        alias: "COPY",
        kind: Kind::Copy,
    },
    Spec {
        name: "array",
        alias: "ARRAY",
        kind: Kind::Array,
    },
    Spec {
        name: "rotate",
        alias: "ROTATE",
        kind: Kind::Rotate,
    },
    Spec {
        name: "scale",
        alias: "SCALE",
        kind: Kind::Scale,
    },
    Spec {
        name: "mirror",
        alias: "MIRROR",
        kind: Kind::Mirror,
    },
    Spec {
        name: "set_attribute",
        alias: "ATTEDIT",
        kind: Kind::SetAttribute,
    },
    grid("set_cell", "TABLEDIT", GridOp::SetCell),
    grid("add_row", "", GridOp::Add(Axis::Row)),
    grid("delete_row", "", GridOp::Delete(Axis::Row)),
    grid("add_column", "", GridOp::Add(Axis::Column)),
    grid("delete_column", "", GridOp::Delete(Axis::Column)),
    Spec {
        name: "add_leader_line",
        alias: "MLEADEREDIT",
        kind: Kind::LeaderLine(LeaderLineOp::Add),
    },
    Spec {
        name: "delete_leader_line",
        alias: "",
        kind: Kind::LeaderLine(LeaderLineOp::Delete),
    },
    table("add_layer", "LAYER", TableOp::Add, Table::Layer),
    table("change_layer", "", TableOp::Change, Table::Layer),
    table("rename_layer", "RENAME", TableOp::Rename, Table::Layer),
    table("delete_layer", "LAYDEL", TableOp::Delete, Table::Layer),
    table("add_linetype", "LINETYPE", TableOp::Add, Table::Linetype),
    table("change_linetype", "", TableOp::Change, Table::Linetype),
    table("rename_linetype", "", TableOp::Rename, Table::Linetype),
    table("delete_linetype", "", TableOp::Delete, Table::Linetype),
    table("add_text_style", "STYLE", TableOp::Add, Table::TextStyle),
    table("change_text_style", "", TableOp::Change, Table::TextStyle),
    table("rename_text_style", "", TableOp::Rename, Table::TextStyle),
    table("delete_text_style", "", TableOp::Delete, Table::TextStyle),
    table(
        "add_dimension_style",
        "DIMSTYLE",
        TableOp::Add,
        Table::DimensionStyle,
    ),
    table(
        "change_dimension_style",
        "",
        TableOp::Change,
        Table::DimensionStyle,
    ),
    table(
        "rename_dimension_style",
        "",
        TableOp::Rename,
        Table::DimensionStyle,
    ),
    table(
        "delete_dimension_style",
        "",
        TableOp::Delete,
        Table::DimensionStyle,
    ),
    block("add_block", "BLOCK", BlockOp::Add),
    block("redefine_block", "BEDIT", BlockOp::Redefine),
    block("attsync", "ATTSYNC", BlockOp::Sync),
    block("import_blocks", "ADCENTER", BlockOp::Import),
];

const fn grid(name: &'static str, alias: &'static str, op: GridOp) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Grid(op),
    }
}

const fn block(name: &'static str, alias: &'static str, op: BlockOp) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Block(op),
    }
}

const fn table(name: &'static str, alias: &'static str, op: TableOp, table: Table) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Table(op, table),
    }
}

const fn add(name: &'static str, alias: &'static str, object: &'static str) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Add(object, None),
    }
}

/// An operation that adds a dimension of one kind, which it gives the
/// dimension's `kind` key.
const fn dimension(name: &'static str, alias: &'static str, kind: &'static str) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Add("dimension", Some(kind)),
    }
}

/// The key a dimension names its kind by, which an operation that adds one
/// of a kind sets.
const DIMENSION_KIND_KEY: &str = "kind";

/// The kind of dimension an operation that adds one names.
fn dimension_kind(name: &str) -> DimensionKind {
    DimensionKind::ALL
        .into_iter()
        .find(|k| k.name() == name)
        .expect("an operation names a kind of dimension")
}

/// The keys of a change set.
const CHANGE_SET_KEYS: [&str; 3] = ["operations", "fixes", "guide"];

/// The key every operation names itself by.
const OP_KEY: &str = "op";

/// The key a copy names the first object of its copies by, which the
/// engine writes and keeps: no add or change takes it.
const COPY_OF: &str = "copy_of";

/// One operation as an agent lists them: its name, its AutoCAD alias, what
/// it does and its schema.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationInfo {
    pub name: String,
    /// The AutoCAD command it does the work of, which names it too.
    pub alias: String,
    /// The other AutoCAD commands that name it, such as ARRAYRECT for
    /// `array`; none for most.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_aliases: Vec<String>,
    pub does: String,
    /// A JSON Schema (draft 2020-12) of the operation as a change set holds
    /// it, with the definitions it refers to.
    pub schema: Value,
}

/// Every operation, with how a change set holds them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationList {
    /// How a change set is sent, and how an operation names the object it
    /// edits.
    pub how: String,
    pub operations: Vec<OperationInfo>,
}

/// The name of the operation `name` names, as a change set sends it: its
/// name, or one of its AutoCAD aliases in any case, such as `add_line` for
/// `LINE`; none when it names no operation. The same table `operations`
/// lists, so an interface that reads an operation reads it as `change` does.
pub fn operation_name(name: &str) -> Option<&'static str> {
    SPECS.iter().find(|s| s.named(name)).map(|s| s.name)
}

/// Every operation a change set can hold, with its alias, what it does and
/// its schema.
pub fn operations() -> OperationList {
    let defs = definitions();
    OperationList {
        how: HOW.to_owned(),
        operations: SPECS
            .iter()
            .map(|spec| OperationInfo {
                name: spec.name.to_owned(),
                alias: spec.alias.to_owned(),
                other_aliases: spec
                    .other_aliases()
                    .iter()
                    .map(|a| (*a).to_owned())
                    .collect(),
                does: does(spec),
                schema: with_definitions(operation_schema(spec, &defs), &defs),
            })
            .collect(),
    }
}

const HOW: &str = "A change set is {\"operations\": [...]}: a list of operations, each a mapping \
                   that names its operation under op, by its name or its AutoCAD alias in any \
                   case, applied in order to the drawing set's working revision, its own files. \
                   An operation that edits an object names it under target, by its ID, \
                   {\"id\": \"DEMO-001/border\"}, or by its name, {\"name\": \"title-block\", \
                   \"sheet\": \"DEMO-001\"}, where sheet may be left out when the name is on one \
                   sheet only; never by coordinates. Wherever an operation takes a point, it \
                   may take a snapped point instead, {\"snap\": {\"id\": \"DEMO-001/border\", \
                   \"snap\": \"endpoint\", \"which\": \"points[2]\"}}, as query snap names one: \
                   it is worked out when the operation applies, on the sheet as it stands then, \
                   must name one point, and is written as coordinates. An object of a template is on every sheet \
                   built from it, and editing it edits the template, so every one of those \
                   sheets follows. An object added with sheet goes in that sheet's row, and \
                   with template in the template. Each sheet takes its part fully or not at \
                   all, and a template edit lands only if every sheet built from the template \
                   takes its whole part. A change set that lands on some sheets and fails on \
                   others marks the set inconsistent until a later change set names the failed \
                   sheets under fixes and lands on them. Lengths are in the set's declared unit \
                   and may carry their own. Send it with `agentdrafter change <set> \
                   <change-set.json>`, and add --dry-run to see what it would do without \
                   writing.";

/// What `spec` does, in words.
fn does(spec: &Spec) -> String {
    match spec.kind {
        Kind::Add(object, kind) => {
            let what = match kind {
                Some(kind) => format!("{} {kind} {object}", article(kind)),
                None => format!("{} {object}", article(object)),
            };
            let kind_key = match kind {
                Some(kind) => format!(
                    ", but its kind, which is {kind}: the operation writes it as `kind: {kind}`"
                ),
                None => String::new(),
            };
            format!(
                "Adds {what}, written with the keys {} {object} takes in a sheet description{kind_key}, \
                 to one sheet's row with sheet, or to a template with template, so that it is on \
                 every sheet built from it. It is drawn after the objects already there. Without \
                 an id it is given one, {object}-1 or the next number free, so that its ID is \
                 fixed for its life. A key sent as null is read as a sheet description reads a \
                 bare null: it leaves an optional text key, such as layer, out. Any point may be a \
                 snapped point, {{\"snap\": {{...}}}}, which is written as the coordinates it \
                 names. AutoCAD's {}.",
                article(object),
                spec.alias
            )
        }
        Kind::Change => "Sets keys of an object: each key under set takes its new value, as a \
                         sheet description writes it, and null takes a key away. It changes \
                         neither the object's type nor its id, which is fixed for its life; an \
                         insert's attributes are set with set_attribute. A text's contents its \
                         template takes from the row are set in that sheet's row, and so is the \
                         text of a multileader's content or a leader's annotation its template \
                         takes from the row, when the new content or annotation differs in its \
                         text alone; any other change of them edits the template. Contents the \
                         template joins from a list of parts, plain text and values from the \
                         row, are matched to the parts' fixed text, and each row value they \
                         hold is set in that sheet's row; it is refused when the fixed text \
                         differs or the text splits more than one way, and row_value sets one \
                         part alone. AutoCAD's CHANGE."
            .to_owned(),
        Kind::ChangeViewport => "Sets a viewport's scale, its view centre, the point of model \
                                 space at its middle, or its display lock, locked: each that is \
                                 sent takes its new value, and the viewport stays where it is on \
                                 the paper, as AutoCAD's ZOOM with a scale, PAN and the display \
                                 lock do in it. A viewport is named by target, by its ID or its \
                                 name. view_center is a point or a snapped point, on model space. \
                                 The scale or the view centre of a locked viewport is refused \
                                 unless locked: false is sent with it, as AutoCAD keeps a locked \
                                 view from being zoomed or panned; move moves a viewport on the \
                                 paper, its view with it."
            .to_owned(),
        Kind::ClipViewport => "Clips a viewport to a boundary on the paper, clip, a closed \
                               polyline's points or a circle's center and radius, as a \
                               viewport's clip is written, or with clip: null takes its \
                               boundary away, as AutoCAD's VPCLIP and its Delete option do. \
                               Model space stays where it showed on the paper: the viewport's \
                               window becomes the box round the boundary, and its view centre \
                               the model point at the box's middle, its scale unchanged, so a \
                               locked viewport may be clipped too. Taken away, the boundary \
                               leaves the viewport rectangular, the box it was clipped to. A \
                               viewport that is not clipped has no boundary to take away. A \
                               viewport is named by target, by its ID or its name."
            .to_owned(),
        Kind::SetFrozenLayers => "Sets the layers frozen in one viewport, frozen_layers, as \
                                  AutoCAD's VPLAYER freezes and thaws them: their objects \
                                  neither draw nor plot through that viewport, and do through \
                                  the others. The list replaces the viewport's own; an empty \
                                  list thaws every layer in it. Each is layer 0 or one of the \
                                  sheet's, named exactly as it is defined. A viewport is named \
                                  by target, by its ID or its name."
            .to_owned(),
        Kind::Move => "Moves an object by by, [dx, dy, dz], or from a base point, from, to a \
                       second point, to, as AutoCAD's MOVE takes them, each a point or a \
                       snapped point: give by, or from and to. Every point it is drawn through \
                       moves, and an ellipse's major axis, an arc's angles and the like stay \
                       as they are. AutoCAD's MOVE."
            .to_owned(),
        Kind::Delete => "Deletes an object. Each later object of its file that has no id is \
                         given its place as its id, such as objects[3], so that its ID does not \
                         move. AutoCAD's ERASE."
            .to_owned(),
        Kind::Copy => "Copies objects, target, one by its ID or its name, or targets, several \
                       of one sheet: once by by, [dx, dy, dz], to each of offsets, a list of \
                       them, or from a base point, from, to a second point, to, each a point or \
                       a snapped point, as AutoCAD's COPY takes them. Each copy is an ordinary \
                       object with an ID of its own, its source's id and the first number free, \
                       such as term-1, or its type's, such as line-1, for a source with no id; \
                       it keeps every key of its source, an insert's attribute values among \
                       them, values from the row too, but its name, which is given once on a \
                       sheet, and is written with copy_of, the first object of the copies: a \
                       value from the row it shares with them becomes the copy's own only \
                       when an edit changes the copy's, which then sets that copy alone; the \
                       first object sets the shared value where it is. A copy is written \
                       where its source is: a template's object's in the template, so on every \
                       sheet built from it, all or nothing, and a row's own object's in the \
                       row. The reply lists each copy under copies, with its source and where \
                       it went. AutoCAD's COPY."
            .to_owned(),
        Kind::Array => "Copies objects, target or targets as copy takes them, in an array of \
                        plain copies, each an ordinary object with an ID of its own as copy \
                        gives it, written where its source is. kind is rectangular: rows and \
                        columns, the objects the first row's first column, rows counting up \
                        by row_spacing and columns across by column_spacing, either negative \
                        to go the other way; polar: count items round center, the objects the \
                        first, over angle degrees anticlockwise, 360 when left out, negative \
                        clockwise, each turned with it unless rotate is false, when each is \
                        moved so that its base_point, the centre of the box round what they \
                        draw when left \
                        out, goes round; or path: count items along path, a line, polyline, \
                        arc, circle, ellipse or spline by its ID or its name, the objects the \
                        first at the path's start, evenly spaced from its start to its end \
                        (round a closed path, count to a turn) or spacing apart, each turned \
                        with the path's direction unless align is false. Sent as ARRAYRECT, \
                        ARRAYPOLAR or ARRAYPATH, kind may be left out. A copy is turned only \
                        when it is a line, polyline, arc, circle, ellipse, spline, hatch, \
                        text, mtext, donut, solid or insert. The reply lists each copy under \
                        copies, with its source and its place in the array. AutoCAD's ARRAY, \
                        ARRAYRECT, ARRAYPOLAR and ARRAYPATH, without the associative array."
            .to_owned(),
        Kind::Rotate => "Turns objects in place, target, one by its ID or its name, or \
                         targets, several of one sheet, by angle degrees, anticlockwise, \
                         negative clockwise, about base_point, a point or a snapped point, as \
                         AutoCAD's ROTATE takes them. Every point each is drawn through turns \
                         about it, and so does every direction it is drawn in: a text's, an \
                         mtext's and an insert's rotation, an arc's angles, an ellipse's major \
                         axis, a patterned hatch's angle and origin and a linear dimension's \
                         angle. A multileader's and a leader's text stays level, laid out again \
                         where its landing puts it, and a dimension's text reads as its rules \
                         lay it out. A turn by a multiple of 90 degrees lands exactly. A table, \
                         which has no rotation, and a viewport are not turned, nor an ordinate \
                         dimension but by a multiple of 90 degrees, which swaps its axis at a \
                         quarter turn. Each keeps its ID. AutoCAD's ROTATE."
            .to_owned(),
        Kind::Scale => "Scales objects in place, target or targets as rotate takes them, by \
                        factor, a number more than 0, about base_point, a point or a snapped \
                        point, as AutoCAD's SCALE takes them. Every point each is drawn through \
                        moves away from it or towards it, along Z too, and every length it is \
                        drawn with is scaled: a radius, a text's and an mtext's height and \
                        width, a polyline's widths, a donut's diameters, an insert's scale and \
                        a patterned hatch's pattern scale, and a multileader's and a leader's \
                        arrow size, gap, landing and text height, written as its own when its \
                        style or the sheet's default gives them. A dimension's text and arrows \
                        stay as its style draws them, and it measures its new length. A table, \
                        whose text its style sizes, and a viewport are not scaled. Each keeps \
                        its ID. AutoCAD's SCALE."
            .to_owned(),
        Kind::Mirror => "Mirrors objects, target or targets as rotate takes them, across the \
                         line through first_point and second_point, each a point or a snapped \
                         point, as AutoCAD's MIRROR takes them. As MIRROR does by default, it \
                         keeps the objects and makes mirrored copies, each an ordinary object \
                         with an ID of its own and copy_of, as copy gives them, written where \
                         its source is, listed under copies; erase_source: true mirrors the \
                         objects themselves instead, each keeping its ID. Text keeps reading \
                         forwards, as AutoCAD's MIRRTEXT 0 keeps it: its position mirrors, and \
                         it reads along its mirrored direction, or that turned a half turn, \
                         whichever reads left to right or upwards, with its own justification; \
                         an insert's attributes read forwards too, while the insert itself is \
                         mirrored, mirrored: true, which a DWG keeps as a negative X scale, and \
                         one mirrored back keeps them so with readable_attributes: true. A \
                         hatch's pattern keeps its direction, as AutoCAD's MIRRHATCH 0 keeps it. \
                         A line across or up the sheet, or at 45 degrees, mirrors exactly. A \
                         table is mirrored only across a line across or up the sheet, an \
                         ordinate dimension only across one across or up the sheet or at 45 \
                         degrees, a leader only when its landing goes to its mirror image's \
                         side, and a multileader with block content and a viewport are not \
                         mirrored. \
                         AutoCAD's MIRROR."
            .to_owned(),
        Kind::Table(op, table) => {
            let word = table.word();
            let what = match op {
                TableOp::Add => format!(
                    "Adds a {word} named name to a template, written with the keys a {word} takes \
                     in a sheet description, so that every sheet built from the template has it"
                ),
                TableOp::Change => format!(
                    "Sets keys of the {word} named name in a template: each key under set takes \
                     its new value, and null takes a key away"
                ),
                TableOp::Rename => format!(
                    "Renames the {word} named name in a template to to, and everything in the \
                     template, and in the rows of the sheets built from it, that names it"
                ),
                TableOp::Delete => format!(
                    "Deletes the {word} named name from a template; refused while anything in the \
                     template, or in the rows of the sheets built from it, uses it, and the error \
                     lists what does"
                ),
            };
            let builtin = match (op, table) {
                (TableOp::Rename | TableOp::Delete, _) => format!(
                    " {} is every drawing's own and is never renamed or deleted.",
                    table.built_in()[0]
                ),
                _ => String::new(),
            };
            let alias = if spec.alias.is_empty() {
                String::new()
            } else {
                format!(" AutoCAD's {}.", spec.alias)
            };
            format!(
                "{what}. The template is named by template, as the rows write its path, or by \
                 sheet, a sheet built from it; every sheet built from it follows, all or \
                 nothing.{builtin}{alias}"
            )
        }
        Kind::Block(op) => {
            let what = match op {
                BlockOp::Add => {
                    "Adds a block named name to a block library: its base_point, which may be \
                     left out for [0, 0, 0], and its objects, each written as a block library \
                     writes a block's objects, attribute definitions (attdef) and inserts of the \
                     library's other blocks among them. A name the library has already, in any \
                     case, is refused, since AutoCAD ignores case in names. AutoCAD's BLOCK."
                }
                BlockOp::Redefine => {
                    "Replaces the whole definition of the block named name in a block library: \
                     its base_point, [0, 0, 0] when left out, and its full list of objects, \
                     written as for add_block. As it lands it brings every insert of the block \
                     into line, as attsync does: a value an insert gives for a tag the block no \
                     longer carries is removed from its record, and the reply lists each under \
                     attributes_removed. AutoCAD's BEDIT."
                }
                BlockOp::Sync => {
                    "Brings every insert of the block named name, or of every block of the \
                     library when name is left out, into line with the block's definition as \
                     the library writes it now, such as after the library was edited by hand: a \
                     value an insert gives for a tag the block no longer carries, because it no \
                     longer defines it or defines it constant, is removed from its record, in a \
                     template, a row or another block of the library; a new tag takes its \
                     default. The reply lists the inserts it reached under inserts and each \
                     value removed under attributes_removed. AutoCAD's ATTSYNC."
                }
                BlockOp::Import => {
                    "Imports the blocks of a DWG or a DXF, from, its path relative to the set's \
                     folder or absolute, into a block library, as AutoCAD's DesignCenter does: \
                     every named block, or those blocks names, each with every block it nests, \
                     with their objects, attribute definitions, and the layers, linetypes, hatch \
                     patterns and text styles they use that the library lacks; one it defines \
                     already it keeps as it defines it. The drawing is only read, and is in the \
                     set's unit, or states none and units names it. A block the library holds \
                     already is refused, unless replace, which replaces its whole definition and \
                     brings its inserts into line as redefine_block does. What the core cannot \
                     carry, such as a dimension, is left out and counted block by block under \
                     imported.not_carried. AutoCAD's ADCENTER."
                }
            };
            format!(
                "{what} The library is named by library, its path in the set as the rows write \
                 a template's path, such as blocks.yaml, which may be left out when every \
                 template of the set names the same library. It reaches every sheet whose \
                 template names the library, all or nothing."
            )
        }
        Kind::Grid(op) => grid_does(op),
        Kind::LeaderLine(LeaderLineOp::Add) => "Adds a leader line to a multileader, target, by \
             its ID or its name: points, from its arrow's point through any bends, up to but not \
             including the point where the multileader's leader lines meet its landing, which \
             the operation appends, so that every leader line meets the landing at the same \
             point, each a point or a snapped point. At least one point; each apart from the one before and from that point; \
             the first segment at least as long as the arrow, unless the arrowhead is none. A \
             multileader of a template is edited in the template, so every sheet built from it \
             follows, all or nothing. AutoCAD's MLEADEREDIT, adding leaders."
            .to_owned(),
        Kind::DimensionFrom(from) => {
            let (what, how) = match from {
                FromOp::Baseline => (
                    "a baseline dimension",
                    "from the same first extension line as target, its dimension line \
                     further out by target's style's baseline_spacing, AutoCAD's DIMDLI; an \
                     angular dimension's arc further out by it",
                ),
                FromOp::Continued => (
                    "a continued dimension",
                    "from target's second extension line, on the same dimension line or arc, \
                     marked continued, as AutoCAD marks it",
                ),
            };
            format!(
                "Adds {what} from target, a linear, aligned, three-point angular or ordinate \
                 dimension, by its ID or its name: {how}, to point, the new second extension \
                 line's origin, or an ordinate dimension's new feature, its leader ending level \
                 with target's. A dimension from an aligned one is linear, at the aligned one's \
                 angle turned half round, as AutoCAD makes it; an ordinate dimension's is the \
                 same either way. It takes target's kind, style and layer, and is added where \
                 target is written, so a template's reaches every sheet built from it. point is \
                 a point or a snapped point. Without an id it is given one, dimension-1 or the \
                 next number free. AutoCAD's {}.",
                spec.alias
            )
        }
        Kind::LeaderLine(LeaderLineOp::Delete) => {
            "Deletes a leader line of a multileader, target, \
             by its ID or its name: leader, its place among the multileader's leader lines, \
             counted from 0 in the order they are written. A multileader's last leader line is \
             not deleted: delete the multileader instead. Deleting the first leader line makes \
             the next the first, whose last segment then sets the side the landing runs to. A \
             multileader of a template is edited in the template, so every sheet built from it \
             follows, all or nothing. AutoCAD's MLEADEREDIT, removing leaders."
                .to_owned()
        }
        Kind::SetAttribute => "Sets the value of the attribute tag of an insert. When the \
                               insert's block defines the tag more than once, occurrence names \
                               which, counted from 1 in the order the block defines them, and \
                               is needed: without it the operation fails, listing each \
                               occurrence with its value. A tag written in lowercase is taken \
                               in capitals, as AutoCAD keeps it. When the insert's template \
                               takes that attribute from the row, the sheet's row value is set \
                               instead, on that sheet alone, and the reply names every place on \
                               the sheet the value fills. A value the template joins from a list \
                               of parts, plain text and values from the row, is matched to the \
                               parts' fixed text, and each row value it holds is set; it is \
                               refused when the fixed text differs or the value splits more \
                               than one way, and row_value sets one part alone. AutoCAD's \
                               ATTEDIT."
            .to_owned(),
    }
}

/// What an operation on a table's grid does, in words.
fn grid_does(op: GridOp) -> String {
    const GRID: &str = "The table is named by target, by its ID or its name. Cells, rows and \
                        columns count from 0 from the top left, a cell [row, column]. A table of \
                        a template is edited in the template, so every sheet built from it \
                        follows, all or nothing.";
    match op {
        GridOp::SetCell => format!(
            "Sets keys of the cell [row, column] of a table: each key under set, text, style, \
             height, justify or merge, takes its new value as a sheet description writes a \
             cell's, and null takes it away. A cell inside another cell's merge is refused, \
             naming that cell; a merge keeps within the table and off other merges, and the \
             cells it covers must be empty. A cell's text its template takes from the row is \
             set in that sheet's row, on that sheet alone. {GRID} AutoCAD's TABLEDIT."
        ),
        GridOp::Add(axis) => {
            let (word, size, across) = match axis {
                Axis::Row => ("row", "height", "column"),
                Axis::Column => ("column", "width", "row"),
            };
            let head = match axis {
                Axis::Row => {
                    " A row is not added above a title or header row, which are the table's \
                     first rows."
                }
                Axis::Column => "",
            };
            format!(
                "Adds a {word} to a table at at, its place counted from 0, or after the last \
                 without it; {size} high or wide, or as the {word} before it without it, or the \
                 first {word} when added at 0; with cells, one for each {across}, each as a sheet \
                 description writes a cell, or empty without them. A merge it falls inside, \
                 between its first and last {word}s, grows by one {word}.{head} {GRID}"
            )
        }
        GridOp::Delete(axis) => {
            let word = axis.word();
            let flags = match axis {
                Axis::Row => " Deleting the title or header row leaves the table without one.",
                Axis::Column => "",
            };
            format!(
                "Deletes the {word} of a table that {word} names, counted from 0. A merge it cuts through \
                 shrinks by one {word}; a {word} that holds a merged cell spanning more {word}s \
                 than it is refused, naming the cell, so take its merge away first with set_cell. \
                 The last {word} is not deleted.{flags} {GRID}"
            )
        }
    }
}

/// The change set's schema: a JSON Schema (draft 2020-12) of every key a
/// change set may hold, with what each means.
pub fn change_set_schema() -> Value {
    let defs = definitions();
    let choices: Vec<Value> = SPECS
        .iter()
        .map(|spec| operation_schema(spec, &defs))
        .collect();
    let [operations_key, fixes_key, guide_key] = CHANGE_SET_KEYS;
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter change set",
        "description": HOW,
        "type": "object",
        "required": [operations_key],
        "additionalProperties": false,
        "properties": {
            operations_key: {
                "description": "The operations, applied in order.",
                "type": "array",
                "items": { "oneOf": choices },
            },
            fixes_key: {
                "description": "The sheets of the set's inconsistent mark this change set fixes. \
                                Each is cleared from the mark when the change set lands on it, \
                                whether or not an operation edits it; the sheet is built again.",
                "type": "array",
                "items": { "$ref": "#/$defs/sheet_name" },
            },
            guide_key: super::guides::mark_schema(),
        },
    });
    with_definitions(schema, &defs)
}

/// Every definition an operation's schema may refer to: the sheet
/// description's, a sheet's name and a target.
fn definitions() -> Map<String, Value> {
    let mut defs = match sheet_description_schema()["$defs"].take() {
        Value::Object(defs) => defs,
        _ => Map::new(),
    };
    defs.insert(
        "sheet_name".into(),
        rows_schema()["$defs"]["sheet_name"].take(),
    );
    defs.insert(
        "template_path".into(),
        json!({
            "description": "A template of the set, as its rows write its path, such as \
                            templates/ansi-b.yaml.",
            "type": "string",
            "minLength": 1,
        }),
    );
    add_block_definitions(&mut defs);
    // A table's cell as a template writes it: the cell, or its text, may be
    // a value from the row, which the reader takes only in a template.
    let mut template_defs = template_schema()["$defs"].take();
    for name in [
        "row_value",
        "value_name",
        "parts",
        "text_parts",
        "attribute_parts",
    ] {
        defs.insert(name.into(), template_defs[name].take());
    }
    let mut cell = template_defs["table_cell"].take();
    cell["description"] = json!(
        "A cell, as a sheet description writes one: its text alone, '' or null for an empty \
         cell, or a mapping that gives its text and what it draws with. In a template's table, \
         the cell or its text may be a value from the row, {row: NAME}; a row's own table takes \
         none."
    );
    defs.insert("template_cell".into(), cell);
    add_snap_definitions(&mut defs);
    defs.insert(
        "target".into(),
        json!({
            "description": "The object an operation edits: by its ID, or by its name and, when \
                            the name is on more than one sheet, its sheet. Never by coordinates.",
            "oneOf": [
                {
                    "type": "object",
                    "required": ["id"],
                    "additionalProperties": false,
                    "properties": {
                        "id": {
                            "description": "The object's ID: its sheet's name, /, and the id \
                                            its record gives it or its place, such as \
                                            DEMO-001/border or DEMO-001/objects[2]. Matched in \
                                            any case.",
                            "type": "string",
                            "pattern": "^[^/]+/.+$",
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["name"],
                    "additionalProperties": false,
                    "properties": {
                        "name": {
                            "description": "The object's name, matched in any case.",
                            "type": "string",
                            "minLength": 1,
                        },
                        "sheet": { "$ref": "#/$defs/sheet_name" },
                    },
                },
            ],
        }),
    );
    defs
}

/// Adds to `defs` the snapped point an operation takes wherever it takes a
/// point, `snapped_point`, the snap it names, `snap_ref`, and, for each
/// object an add writes and each definition it holds that holds a point,
/// its definition as an operation takes it, under `op_` and its name, such
/// as `op_line`: a snapped point wherever the sheet description takes a
/// point, but a direction, such as an ellipse's `major_axis`. The sheet
/// description's own definitions stay as they are: a description holds
/// coordinates, never a snap.
fn add_snap_definitions(defs: &mut Map<String, Value>) {
    let kinds: Vec<&str> = SnapKind::ALL.iter().map(|k| k.name()).collect();
    let object = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "description": "The object's ID.", "type": "string", "minLength": 1 },
            "name": {
                "description": "The object's name, on the sheet the operation reaches.",
                "type": "string",
                "minLength": 1,
            },
        },
        // By its ID or by its name, and not both, as the reader takes it.
        "oneOf": [
            { "required": ["id"], "not": { "required": ["name"] } },
            { "required": ["name"], "not": { "required": ["id"] } },
        ],
    });
    let mut snap = object.clone();
    snap["description"] = json!(
        "A point one of AutoCAD's object snaps finds, as query snap names it: an object by id \
         or by name, on the sheet the operation reaches, its snap, and which of its points of \
         that snap, as query snap's reply names it, when it has several. An intersection or an \
         apparent intersection takes the other object as with; a perpendicular, a tangent or a \
         nearest point the point it is taken from as from, a snap of another object that names \
         one point. It is worked out when the operation applies, on the sheet as it stands \
         then, earlier operations of the change set included, and must name one point."
    );
    snap["required"] = json!(["snap"]);
    let properties = snap["properties"].as_object_mut().expect("properties");
    properties.insert(
        "snap".into(),
        json!({
            "description": "The snap: endpoint, midpoint, center, quadrant, insertion or node of \
                            the object by itself; intersection or apparent_intersection with \
                            with; perpendicular, tangent or nearest from from.",
            "enum": kinds,
        }),
    );
    properties.insert(
        "which".into(),
        json!({
            "description": "Which of the object's points of the snap, as query snap's reply \
                            names it: points[2], segments[1], 90, objects[3].to of an insert's \
                            block, 0 of several intersections. Needed when the snap finds more \
                            than one point.",
            "type": "string",
            "minLength": 1,
        }),
    );
    let mut with = object;
    with["description"] =
        json!("The other object of an intersection or an apparent intersection, by id or by name.");
    properties.insert("with".into(), with);
    properties.insert(
        "from".into(),
        json!({
            "description": "The point a perpendicular, a tangent or a nearest point is taken \
                            from: a snap of another object that names one point.",
            "$ref": "#/$defs/snap_ref",
        }),
    );
    // `with` just for the two intersections, which need it, and `from`
    // just for a perpendicular, a tangent and a nearest point, which need
    // it, as the reader takes them.
    let only = |takes: fn(SnapKind) -> bool, key: &str| {
        let kinds: Vec<&str> = SnapKind::ALL
            .iter()
            .filter(|k| takes(**k))
            .map(|k| k.name())
            .collect();
        json!({
            "if": { "properties": { "snap": { "enum": kinds } } },
            "then": { "required": [key] },
            "else": { "not": { "required": [key] } },
        })
    };
    snap["allOf"] = json!([
        only(SnapKind::takes_with, "with"),
        only(SnapKind::takes_from, "from"),
    ]);
    defs.insert("snap_ref".into(), snap);
    defs.insert(
        "snapped_point".into(),
        json!({
            "description": "A point: [x, y, z], three lengths, or a snapped point, \
                            {\"snap\": {...}}, which the record writes as the coordinates it \
                            names, in the declared unit.",
            "oneOf": [
                { "$ref": "#/$defs/point" },
                {
                    "type": "object",
                    "required": ["snap"],
                    "additionalProperties": false,
                    "properties": { "snap": { "$ref": "#/$defs/snap_ref" } },
                },
            ],
        }),
    );
    let mut made: Map<String, Value> = Map::new();
    for spec in &SPECS {
        if let Kind::Add(object, _) = spec.kind {
            snapped_def(object, defs, &mut made);
        }
    }
    for (name, def) in made {
        if !def.is_null() {
            defs.insert(name, def);
        }
    }
}

/// Makes the definition `name` as an operation takes it, `op_` and its
/// name, into `made`, when it holds a point, and gives whether it does. A
/// definition that holds none is marked null in `made`.
fn snapped_def(name: &str, defs: &Map<String, Value>, made: &mut Map<String, Value>) -> bool {
    let key = format!("op_{name}");
    if let Some(done) = made.get(&key) {
        return !done.is_null();
    }
    made.insert(key.clone(), Value::Null);
    let Some(def) = defs.get(name) else {
        return false;
    };
    let (def, holds) = snapify(def, None, defs, made);
    if holds {
        made.insert(key, def);
    }
    holds
}

/// `value`, a schema or a part of one, with a snapped point wherever it
/// takes a point but under `major_axis`, which is a direction, and whether
/// it takes one. `key` is the property `value` is the schema of.
fn snapify(
    value: &Value,
    key: Option<&str>,
    defs: &Map<String, Value>,
    made: &mut Map<String, Value>,
) -> (Value, bool) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                let name = reference.strip_prefix("#/$defs/").unwrap_or_default();
                let to = if name == "point" && key != Some("major_axis") {
                    Some("snapped_point".to_owned())
                } else if name != "point" && snapped_def(name, defs, made) {
                    Some(format!("op_{name}"))
                } else {
                    None
                };
                let mut map = map.clone();
                let holds = to.is_some();
                if let Some(to) = to {
                    map.insert("$ref".into(), Value::String(format!("#/$defs/{to}")));
                }
                return (Value::Object(map), holds);
            }
            let mut out = Map::new();
            let mut holds = false;
            for (k, v) in map {
                let (v, h) = if k == "properties" {
                    match v {
                        Value::Object(properties) => {
                            let mut each = Map::new();
                            let mut any = false;
                            for (pk, pv) in properties {
                                let (pv, h) = snapify(pv, Some(pk), defs, made);
                                any |= h;
                                each.insert(pk.clone(), pv);
                            }
                            (Value::Object(each), any)
                        }
                        other => (other.clone(), false),
                    }
                } else {
                    snapify(v, None, defs, made)
                };
                holds |= h;
                out.insert(k.clone(), v);
            }
            (Value::Object(out), holds)
        }
        Value::Array(items) => {
            let mut holds = false;
            let items = items
                .iter()
                .map(|v| {
                    let (v, h) = snapify(v, None, defs, made);
                    holds |= h;
                    v
                })
                .collect();
            (Value::Array(items), holds)
        }
        other => (other.clone(), false),
    }
}

/// Adds the block library's definition of a block to `defs`, as
/// `block_definition`, with each object a block holds as the library's
/// schema defines it, under `block_` and its type, such as `block_line`,
/// and the cell. A block's objects differ from a sheet's: they carry no
/// `id` or `name`, and a text or an attribute definition takes a cell.
fn add_block_definitions(defs: &mut Map<String, Value>) {
    let mut library = match block_library_schema()["$defs"].take() {
        Value::Object(defs) => defs,
        _ => Map::new(),
    };
    let mut block = library.remove("block").unwrap_or(Value::Null);
    let mut types = Vec::new();
    if let Some(choices) = block["properties"]["objects"]["items"]["oneOf"].as_array_mut() {
        for choice in choices {
            let Some(properties) = choice["properties"].as_object_mut() else {
                continue;
            };
            for (kind, object) in properties.iter_mut() {
                object["$ref"] = Value::String(format!("#/$defs/block_{kind}"));
                types.push(kind.clone());
            }
        }
    }
    defs.insert("block_definition".into(), block);
    for kind in types {
        if let Some(def) = library.remove(&kind) {
            defs.insert(format!("block_{kind}"), def);
        }
    }
    if let Some(cell) = library.remove("cell") {
        defs.insert("cell".into(), cell);
    }
}

/// The schema of one operation, referring to `defs`.
fn operation_schema(spec: &Spec, defs: &Map<String, Value>) -> Value {
    let op = if spec.alias.is_empty() {
        json!({
            "description": format!("The operation: {}.", spec.name),
            "const": spec.name,
        })
    } else if spec.other_aliases().is_empty() {
        json!({
            "description": format!(
                "The operation: {}, or its AutoCAD alias {} in any case.",
                spec.name, spec.alias
            ),
            "anyOf": [
                { "const": spec.name },
                { "type": "string", "pattern": format!("^{}$", any_case(spec.alias)) },
            ],
        })
    } else {
        let all: Vec<&str> = std::iter::once(spec.alias)
            .chain(spec.other_aliases().iter().copied())
            .collect();
        json!({
            "description": format!(
                "The operation: {}, or its AutoCAD aliases {} in any case.",
                spec.name,
                all.join(", ")
            ),
            "anyOf": [
                { "const": spec.name },
                { "type": "string", "pattern": any_case_of(&all) },
            ],
        })
    };
    let target = json!({ "$ref": "#/$defs/target" });
    let mut schema = match spec.kind {
        Kind::Add(object, kind) => {
            let mut schema = defs
                .get(&format!("op_{object}"))
                .unwrap_or(&defs[object])
                .clone();
            // The engine writes copy_of on a copy; an add does not take it.
            if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
                properties.remove(COPY_OF);
            }
            if let Some(kind) = kind {
                // The operation gives the dimension its kind.
                let own = schema.as_object_mut().expect("an object's definition");
                own["description"] = json!(format!(
                    "{} The operation adds one of kind {kind}, and takes no kind of its own.",
                    own["description"].as_str().unwrap_or_default()
                ));
                // It lists exactly its kind's keys: those it requires, and
                // none of another kind's.
                let (needs, _, others) = kind_keys(dimension_kind(kind));
                if let Some(required) = own.get_mut("required").and_then(Value::as_array_mut) {
                    required.retain(|k| k != DIMENSION_KIND_KEY);
                    required.extend(needs.iter().map(|k| json!(k)));
                }
                if let Some(properties) = own.get_mut("properties").and_then(Value::as_object_mut) {
                    properties.remove(DIMENSION_KIND_KEY);
                    for key in others {
                        properties.remove(*key);
                    }
                }
                own.remove("oneOf");
            }
            let properties = schema["properties"]
                .as_object_mut()
                .expect("an object's definition has properties");
            properties.insert(
                "sheet".into(),
                json!({
                    "description": "The sheet to add it to, in its row: it is on this sheet \
                                    alone. On a drawing set give sheet or template; on an \
                                    original `.dwg`, omit sheet to target that drawing.",
                    "$ref": "#/$defs/sheet_name",
                }),
            );
            properties.insert(
                "template".into(),
                json!({
                    "description": "The template to add it to: it is on every sheet built \
                                    from it. On a drawing set give sheet or template.",
                    "$ref": "#/$defs/template_path",
                }),
            );
            // Drawing sets require `sheet` or `template` (enforced at runtime
            // by `read_change_set`); original `.dwg` revisions allow omitting
            // both to default to the original's sheet. Giving both is always
            // forbidden.
            let place = json!({ "not": { "required": ["sheet", "template"] } });
            let own = schema.as_object_mut().expect("an object's definition");
            let mut all_of = vec![place];
            if let Some(one_of) = own.remove("oneOf") {
                all_of.push(json!({ "oneOf": one_of }));
            }
            if let Some(sheet_alone) = template_parts(object, own) {
                all_of.push(sheet_alone);
            }
            own.insert("allOf".into(), Value::Array(all_of));
            schema
        }
        Kind::Change => json!({
            "type": "object",
            "required": ["target", "set"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "set": {
                    "description": "The keys to set, each to its new value as a sheet \
                                    description writes it, such as {\"height\": 0.3} or \
                                    {\"layer\": \"notes\"}; null takes an optional key away. \
                                    Any key the object's type takes, name, and, on a sheet \
                                    drawn in a layout, space, which moves the object to the \
                                    other space as it stands, unscaled; not id, nor an \
                                    insert's attributes, nor copy_of, which copy and array \
                                    write and the engine keeps.",
                    "type": "object",
                    "minProperties": 1,
                    "propertyNames": { "not": { "enum": ["id", "copy_of"] } },
                },
                ROW_VALUE_KEY: row_value_key(),
            },
        }),
        Kind::ChangeViewport => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "anyOf": [
                { "required": ["scale"] },
                { "required": ["view_center"] },
                { "required": ["locked"] },
            ],
            "properties": {
                "target": target,
                "scale": {
                    "description": "The viewport's new scale, paper to model: a ratio such as \
                                    `1:4` or `2:1`, or a number more than 0 such as 0.25.",
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0 },
                        { "type": "string" },
                    ],
                },
                "view_center": {
                    "description": "The point of model space at the viewport's middle: a point \
                                    [x, y, 0] or a snapped point on model space.",
                    "$ref": "#/$defs/snapped_point",
                },
                "locked": {
                    "description": "Whether the viewport's view is locked.",
                    "type": "boolean",
                },
            },
        }),
        Kind::ClipViewport => json!({
            "type": "object",
            "required": ["target", "clip"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "clip": {
                    "description": "The boundary to clip the viewport to, on the paper, or \
                                    null to take its boundary away.",
                    "anyOf": [
                        { "$ref": "#/$defs/viewport_clip" },
                        { "type": "null" },
                    ],
                },
            },
        }),
        Kind::SetFrozenLayers => json!({
            "type": "object",
            "required": ["target", "frozen_layers"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "frozen_layers": {
                    "description": "The layers frozen in the viewport, each layer 0 or one of \
                                    the sheet's, named exactly as it is defined, and each once \
                                    whatever its case: the viewport's new list, empty to thaw \
                                    them all.",
                    "type": "array",
                    "items": { "type": "string" },
                },
            },
        }),
        Kind::Move => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "oneOf": [
                { "required": ["by"], "not": { "anyOf": [{ "required": ["from"] }, { "required": ["to"] }] } },
                { "required": ["from", "to"], "not": { "required": ["by"] } },
            ],
            "properties": {
                "target": target,
                "by": {
                    "description": "How far to move it, [dx, dy, dz], as lengths in the set's \
                                    declared unit or carrying their own. Give by, or from and \
                                    to.",
                    "$ref": "#/$defs/point",
                },
                "from": {
                    "description": "The base point, as AutoCAD's MOVE takes it: a point or a \
                                    snapped point, such as a point of the object moved. It \
                                    moves by to less from.",
                    "$ref": "#/$defs/snapped_point",
                },
                "to": {
                    "description": "The second point, where the base point lands: a point or \
                                    a snapped point, such as a point of another object.",
                    "$ref": "#/$defs/snapped_point",
                },
            },
        }),
        Kind::Delete => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "properties": { "target": target },
        }),
        Kind::Copy => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "allOf": [
                    copied_objects_rule(),
                    {
                        "oneOf": [
                            { "required": ["by"], "not": { "anyOf": [{ "required": ["offsets"] }, { "required": ["from"] }, { "required": ["to"] }] } },
                            { "required": ["offsets"], "not": { "anyOf": [{ "required": ["by"] }, { "required": ["from"] }, { "required": ["to"] }] } },
                            { "required": ["from", "to"], "not": { "anyOf": [{ "required": ["by"] }, { "required": ["offsets"] }] } },
                        ],
                    },
                ],
                "properties": {
                    "by": {
                        "description": "How far to copy them, [dx, dy, dz], as lengths in the \
                                        set's declared unit or carrying their own: one copy. \
                                        Give by, offsets, or from and to.",
                        "$ref": "#/$defs/point",
                    },
                    "offsets": {
                        "description": "How far to copy them, once for each: a list of \
                                        [dx, dy, dz], as AutoCAD's COPY makes several copies.",
                        "type": "array",
                        "items": { "$ref": "#/$defs/point" },
                        "minItems": 1,
                    },
                    "from": {
                        "description": "The base point, as AutoCAD's COPY takes it: a point or \
                                        a snapped point. The copy is to less from away.",
                        "$ref": "#/$defs/snapped_point",
                    },
                    "to": {
                        "description": "The second point, where the base point's copy lands: \
                                        a point or a snapped point.",
                        "$ref": "#/$defs/snapped_point",
                    },
                },
            });
            copied_objects_properties(&mut schema);
            schema
        }
        Kind::Array => array_schema(),
        Kind::Rotate | Kind::Scale | Kind::Mirror => transform_schema(spec.kind),
        Kind::Table(op, table) => table_schema(op, table, defs),
        Kind::Block(op) => block_schema(op, defs),
        Kind::Grid(op) => grid_schema(op, defs),
        Kind::LeaderLine(LeaderLineOp::Add) => json!({
            "type": "object",
            "required": ["target", "points"],
            "additionalProperties": false,
            "properties": {
                "target": {
                    "description": "The multileader, by its ID or its name.",
                    "$ref": "#/$defs/target",
                },
                "points": {
                    "description": "The new leader line's points, from its arrow's point up to, \
                                    not including, the point where the multileader's leader \
                                    lines meet its landing, which the operation appends: at least \
                                    one, as lengths in the set's declared unit or carrying their \
                                    own, or snapped points.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/snapped_point" },
                    "minItems": 1,
                },
            },
        }),
        Kind::DimensionFrom(_) => json!({
            "type": "object",
            "required": ["target", "point"],
            "additionalProperties": false,
            "properties": {
                "target": {
                    "description": "The dimension it is added from, by its ID or its name: a \
                                    linear, aligned, three-point angular or ordinate one.",
                    "$ref": "#/$defs/target",
                },
                "point": {
                    "description": "The new dimension's second extension line's origin, or an \
                                    ordinate dimension's feature: a point, as lengths in the \
                                    set's declared unit or carrying their own, or a snapped \
                                    point.",
                    "$ref": "#/$defs/snapped_point",
                },
                "id": {
                    "description": "The new dimension's ID in its file. Without it, \
                                    dimension-1 or the next number free.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "The new dimension's name, which an operation or a query \
                                    may name it by.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::LeaderLine(LeaderLineOp::Delete) => json!({
            "type": "object",
            "required": ["target", "leader"],
            "additionalProperties": false,
            "properties": {
                "target": {
                    "description": "The multileader, by its ID or its name.",
                    "$ref": "#/$defs/target",
                },
                "leader": {
                    "description": "The leader line to delete: its place among the \
                                    multileader's leader lines, counted from 0.",
                    "type": "integer",
                    "minimum": 0,
                },
            },
        }),
        Kind::SetAttribute => json!({
            "type": "object",
            "required": ["target", "tag", "value"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "tag": { "$ref": "#/$defs/attribute_tag" },
                "occurrence": {
                    "description": "Which of the attributes with this tag to set, counted from 1 \
                                    in the order the insert's block defines them: needed when \
                                    the block defines the tag more than once, and 1 or left out \
                                    when it defines it once.",
                    "type": "integer",
                    "minimum": 1,
                },
                "value": { "$ref": "#/$defs/attribute_value" },
                ROW_VALUE_KEY: row_value_key(),
            },
        }),
    };
    schema["description"] = json!(does(spec));
    schema["properties"][OP_KEY] = op;
    let required = schema["required"].as_array_mut().expect("a list");
    required.insert(0, json!(OP_KEY));
    schema
}

/// The rule that a copy or an array names its objects by target, one, or
/// by targets, several, and not both.
fn copied_objects_rule() -> Value {
    json!({
        "oneOf": [
            { "required": ["target"], "not": { "required": ["targets"] } },
            { "required": ["targets"], "not": { "required": ["target"] } },
        ],
    })
}

/// Adds to a copy's or an array's `schema` the keys that name the objects
/// it copies.
fn copied_objects_properties(schema: &mut Value) {
    schema["properties"]["target"] = json!({
        "description": "The object to copy, by its ID or its name. Give target or targets.",
        "$ref": "#/$defs/target",
    });
    schema["properties"]["targets"] = json!({
        "description": "The objects to copy, each by its ID or its name, all of one sheet: each \
                        is copied to every place, in the order listed.",
        "type": "array",
        "items": { "$ref": "#/$defs/target" },
        "minItems": 1,
    });
}

/// The schema of `rotate`, `scale` or `mirror`: the objects, by target or
/// targets, and the base point and angle, the base point and factor, or
/// the mirror line's two points.
fn transform_schema(kind: Kind) -> Value {
    let verb = match kind {
        Kind::Rotate => "turn",
        Kind::Scale => "scale",
        _ => "mirror",
    };
    let (required, properties) = match kind {
        Kind::Rotate => (
            json!(["base_point", "angle"]),
            json!({
                "base_point": {
                    "description": "The point they turn about, as AutoCAD's ROTATE takes it: a \
                                    point or a snapped point.",
                    "$ref": "#/$defs/snapped_point",
                },
                "angle": {
                    "description": "How far they turn, in degrees anticlockwise, negative \
                                    clockwise, a number. A multiple of 90 lands exactly.",
                    "type": "number",
                },
            }),
        ),
        Kind::Scale => (
            json!(["base_point", "factor"]),
            json!({
                "base_point": {
                    "description": "The point they are scaled about, which stays where it is, \
                                    as AutoCAD's SCALE takes it: a point or a snapped point.",
                    "$ref": "#/$defs/snapped_point",
                },
                "factor": {
                    "description": "How much they are enlarged, the same along every axis: a \
                                    number more than 0, less than 1 to make them smaller.",
                    "type": "number",
                    "exclusiveMinimum": 0,
                },
            }),
        ),
        _ => (
            json!(["first_point", "second_point"]),
            json!({
                "first_point": {
                    "description": "The first point of the mirror line, as AutoCAD's MIRROR \
                                    takes it: a point or a snapped point.",
                    "$ref": "#/$defs/snapped_point",
                },
                "second_point": {
                    "description": "The second point of the mirror line, apart from the first \
                                    seen from above: a point or a snapped point. The mirror is \
                                    upright through the line, so every point keeps its Z.",
                    "$ref": "#/$defs/snapped_point",
                },
                "erase_source": {
                    "description": "Whether the objects themselves are mirrored, each keeping \
                                    its ID, as MIRROR's Erase source objects Yes leaves them, \
                                    rather than kept with mirrored copies beside them: true or \
                                    false. Without it, false.",
                    "type": "boolean",
                },
            }),
        ),
    };
    let mut schema = json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "allOf": [copied_objects_rule()],
        "properties": properties,
    });
    schema["properties"]["target"] = json!({
        "description": format!(
            "The object to {verb}, by its ID or its name. Give target or targets."
        ),
        "$ref": "#/$defs/target",
    });
    schema["properties"]["targets"] = json!({
        "description": format!(
            "The objects to {verb}, each by its ID or its name, all of one sheet, in the order \
             listed."
        ),
        "type": "array",
        "items": { "$ref": "#/$defs/target" },
        "minItems": 1,
    });
    schema
}

/// The schema of `array`: the keys of each kind, rectangular, polar and
/// path, which `kind` names, or which the alias it is sent as, ARRAYRECT,
/// ARRAYPOLAR or ARRAYPATH, gives it.
fn array_schema() -> Value {
    let count = |what: &str, least: usize| {
        json!({
            "description": what,
            "type": "integer",
            "minimum": least,
            "maximum": MOST_COPIES + 1,
        })
    };
    let kinds: Vec<Value> = ARRAY_KINDS
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            let others: Vec<&str> = ARRAY_ALIASES
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, a)| *a)
                .collect();
            // Every key of another kind, but one it shares, is refused.
            let foreign: Vec<Value> = ARRAY_KEYS
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .flat_map(|(_, keys)| keys.iter())
                .filter(|k| !ARRAY_KEYS[i].contains(k))
                .map(|k| json!({ "required": [k] }))
                .collect();
            // The kind is named, and no alias names another; or it is left
            // out, and the operation is sent as the kind's alias.
            let named = json!({
                "required": ["kind"],
                "properties": {
                    "kind": { "const": kind },
                    "op": { "not": { "type": "string", "pattern": any_case_of(&others) } },
                },
            });
            let by_alias = json!({
                "not": { "required": ["kind"] },
                "required": ["op"],
                "properties": {
                    "op": { "type": "string", "pattern": any_case_of(&[ARRAY_ALIASES[i]]) },
                },
            });
            let mut all = vec![
                json!({ "anyOf": [named, by_alias] }),
                json!({ "not": { "anyOf": foreign } }),
            ];
            match *kind {
                "rectangular" => {
                    // More than the objects alone: 2 rows or 2 columns at
                    // least, each spaced when there is more than one.
                    let many = |key: &str| {
                        json!({ "required": [key], "properties": { key: { "minimum": 2 } } })
                    };
                    all.push(json!({ "anyOf": [many("rows"), many("columns")] }));
                    all.push(json!({ "if": many("rows"), "then": { "required": ["row_spacing"] } }));
                    all.push(
                        json!({ "if": many("columns"), "then": { "required": ["column_spacing"] } }),
                    );
                }
                "polar" => {
                    all.push(json!({ "required": ["center", "count"] }));
                    // A base point places copies that are not turned.
                    all.push(json!({
                        "if": { "required": ["base_point"] },
                        "then": {
                            "required": ["rotate"],
                            "properties": { "rotate": { "const": false } },
                        },
                    }));
                }
                _ => all.push(json!({ "required": ["path", "count"] })),
            }
            json!({ "allOf": all })
        })
        .collect();
    let mut schema = json!({
        "type": "object",
        "required": [],
        "additionalProperties": false,
        "allOf": [copied_objects_rule(), { "oneOf": kinds }],
        "properties": {
            "kind": {
                "description": "The kind of array: rectangular, polar or path. It may be left \
                                out when the operation is sent as ARRAYRECT, ARRAYPOLAR or \
                                ARRAYPATH, which give it.",
                "enum": ARRAY_KINDS,
            },
            "rows": count(
                "A rectangular array's rows, the objects' among them, counting up by \
                 row_spacing: 1 when left out. Rows times columns is at least 2.",
                1,
            ),
            "columns": count(
                "A rectangular array's columns, the objects' among them, counting across by \
                 column_spacing: 1 when left out.",
                1,
            ),
            "row_spacing": {
                "description": "How far apart a rectangular array's rows are, along Y: a \
                                length, negative to count down. Needed with more than one row.",
                "$ref": "#/$defs/length",
            },
            "column_spacing": {
                "description": "How far apart its columns are, along X: a length, negative to \
                                count leftwards. Needed with more than one column.",
                "$ref": "#/$defs/length",
            },
            "center": {
                "description": "The centre a polar array goes round: a point or a snapped \
                                point.",
                "$ref": "#/$defs/snapped_point",
            },
            "count": count(
                "How many items a polar or a path array holds, the objects themselves the \
                 first: at least 2.",
                2,
            ),
            "angle": {
                "description": "The angle a polar array fills, in degrees, a number: \
                                anticlockwise, or clockwise when negative, more than 0 and at \
                                most 360 either way, 360 when left out. 360 is a whole turn, \
                                which spaces count items evenly round it; less puts the last \
                                at its end. 0 fills nothing.",
                "type": "number",
                "minimum": -360,
                "maximum": 360,
                "not": { "const": 0 },
            },
            "rotate": {
                "description": "Whether a polar array's copies are turned as they go round, \
                                as AutoCAD's ARRAYPOLAR turns them by default: true when left \
                                out.",
                "type": "boolean",
            },
            "base_point": {
                "description": "The point of the objects that goes round when the copies are \
                                not turned, a point or a snapped point: the centre of the box \
                                round what they draw when left out. Only with rotate false.",
                "$ref": "#/$defs/snapped_point",
            },
            "path": {
                "description": "The path a path array runs along, by its ID or its name: a \
                                line, a polyline, an arc, a circle, an ellipse or a spline of \
                                the objects' sheet. It runs as it is drawn: a line from its \
                                from to its to, a polyline through its points in order, an \
                                arc, a circle from angle 0 and an ellipse anticlockwise, a \
                                spline from its first point.",
                "$ref": "#/$defs/target",
            },
            "spacing": {
                "description": "How far apart a path array's items are, along the path: a \
                                length more than 0. Without it, the items are spaced evenly \
                                from the path's start to its end, or round a closed path.",
                "$ref": "#/$defs/length_more_than_0",
            },
            "align": {
                "description": "Whether a path array's copies are turned with the path's \
                                direction, as AutoCAD's ARRAYPATH aligns them by default, each \
                                as the first is to the path at its start: true when left out.",
                "type": "boolean",
            },
        },
    });
    copied_objects_properties(&mut schema);
    schema
}

/// Lets an add of a text, an mtext or an insert, whose schema is `own`,
/// take a list of parts (#102) where a template's reader takes one: a
/// text's or an mtext's contents, and an insert's attribute values. Gives
/// the condition that refuses one on an add to a sheet's row, which writes
/// its values itself; none for another object.
fn template_parts(object: &str, own: &mut Map<String, Value>) -> Option<Value> {
    let properties = own.get_mut("properties")?.as_object_mut()?;
    let parts = match object {
        "text" => "#/$defs/text_parts",
        "mtext" => "#/$defs/parts",
        "insert" => {
            let attributes = properties.get_mut("attributes")?;
            let values = attributes.get_mut("additionalProperties")?;
            let written = values.take();
            *values = json!({ "anyOf": [written, { "$ref": "#/$defs/attribute_parts" }] });
            let said = attributes["description"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            attributes["description"] = json!(format!(
                "{said} Added to a template, an attribute its block defines once may take a list \
                 of parts instead, plain text and values from the row, {{row: NAME}}, joined in \
                 order; added to a sheet's row, none."
            ));
            return Some(json!({
                "if": { "required": ["sheet"] },
                "then": {
                    "properties": {
                        "attributes": {
                            "additionalProperties": {
                                "not": {
                                    "type": "array",
                                    "contains": { "$ref": "#/$defs/row_value" },
                                },
                            },
                        },
                    },
                },
            }));
        }
        _ => return None,
    };
    let contents = properties.get_mut("contents")?;
    let written = contents.take();
    let said = written["description"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    *contents = json!({
        "description": format!(
            "{said} Added to a template, it may be a list of parts instead, plain text and \
             values from the row, {{row: NAME}}, joined in order; added to a sheet's row, it may \
             not."
        ),
        "anyOf": [written, { "$ref": parts }],
    });
    Some(json!({
        "if": { "required": ["sheet"] },
        "then": { "properties": { "contents": { "not": { "type": "array" } } } },
    }))
}

/// The key of an operation that sets one row value of a list of parts.
pub(crate) const ROW_VALUE_KEY: &str = "row_value";

/// The schema of an operation's `row_value`.
fn row_value_key() -> Value {
    json!({
        "description": "When the template joins this value from a list of parts, plain text \
                        and values from the row, the one row value of the parts to set, by \
                        name, to the text sent, on this sheet alone. Without it the text sent \
                        is the whole value, matched to the parts' fixed text to find each row \
                        value's. The name of one of the row's values: ASCII letters, \
                        digits, _ and -, such as PANEL.",
        "type": "string",
        "pattern": VALUE_NAME,
    })
}

/// The schema of a table operation, before its `op`.
fn table_schema(op: TableOp, table: Table, defs: &Map<String, Value>) -> Value {
    let word = table.word();
    let mut properties = Map::new();
    properties.insert(
        "name".into(),
        json!({
            "description": format!("The {word}'s name, written exactly as the template writes it."),
            "type": "string",
            "minLength": 1,
        }),
    );
    properties.insert(
        "sheet".into(),
        json!({
            "description": "A sheet built from the template to edit: the template is that \
                            sheet's. Give sheet or template.",
            "$ref": "#/$defs/sheet_name",
        }),
    );
    properties.insert(
        "template".into(),
        json!({
            "description": "The template to edit. Give sheet or template.",
            "$ref": "#/$defs/template_path",
        }),
    );
    let mut required = vec![json!("name")];
    match op {
        TableOp::Add => {
            let def = &defs[table.def()];
            if let Some(own) = def["properties"].as_object() {
                for (key, value) in own {
                    properties.insert(key.clone(), value.clone());
                }
            }
            if let Some(own) = def["required"].as_array() {
                required.extend(own.iter().cloned());
            }
        }
        TableOp::Change => {
            properties.insert(
                "set".into(),
                json!({
                    "description": format!(
                        "The keys to set, each to its new value as a sheet description writes it; \
                         null takes a key away. Any key a {word} takes: {}.",
                        table.entry_keys().join(", ")
                    ),
                    "type": "object",
                    "minProperties": 1,
                    "propertyNames": { "enum": table.entry_keys() },
                }),
            );
            required.push(json!("set"));
        }
        TableOp::Rename => {
            properties.insert(
                "to".into(),
                json!({
                    "description": format!("The {word}'s new name."),
                    "type": "string",
                    "minLength": 1,
                }),
            );
            required.push(json!("to"));
        }
        TableOp::Delete => {}
    }
    let mut schema = json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
    });
    if matches!((op, table), (TableOp::Add | TableOp::Change, Table::Layer)) {
        schema["not"] = json!({ "required": ["sheet", "template"] });
    } else {
        schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
    }
    schema
}

/// The schema of an operation on a table's grid, before its `op`.
fn grid_schema(op: GridOp, defs: &Map<String, Value>) -> Value {
    let place = |what: &str| {
        json!({
            "description": what,
            "type": "integer",
            "minimum": 0,
        })
    };
    // An optional key sent as null is left out.
    let or_null = |mut schema: Value| {
        let description = schema
            .as_object_mut()
            .and_then(|own| own.remove("description"));
        let mut optional = json!({ "anyOf": [schema, { "type": "null" }] });
        if let Some(description) = description {
            optional["description"] = description;
        }
        optional
    };
    let mut properties = Map::new();
    properties.insert(
        "target".into(),
        json!({
            "description": "The table, by its ID or its name.",
            "$ref": "#/$defs/target",
        }),
    );
    let mut required = vec![json!("target")];
    match op {
        GridOp::SetCell => {
            // A cell's keys, as a cell written as a mapping takes them, each
            // of which null takes away.
            let mut keys = Map::new();
            let cell = &defs["template_cell"]["anyOf"];
            if let Some(own) = cell
                .as_array()
                .and_then(|choices| choices.iter().find_map(|c| c["properties"].as_object()))
            {
                for (key, value) in own {
                    let mut value = value.clone();
                    let description = value.as_object_mut().and_then(|v| v.remove("description"));
                    let mut optional = json!({ "anyOf": [value, { "type": "null" }] });
                    if let Some(description) = description {
                        optional["description"] = description;
                    }
                    if key == "text" {
                        optional["description"] = json!(
                            "What the cell says, exactly as it shows; null takes it away. In a \
                             template's table it may be a value from the row, {row: NAME}, \
                             which only a template takes. Text sent for a cell whose template \
                             takes its text from the row sets that sheet's row value instead."
                        );
                    }
                    keys.insert(key.clone(), optional);
                }
            }
            properties.insert(
                "cell".into(),
                json!({
                    "description": "The cell, [row, column], each counted from 0 from the top \
                                    left.",
                    "type": "array",
                    "items": { "type": "integer", "minimum": 0 },
                    "minItems": 2,
                    "maxItems": 2,
                }),
            );
            properties.insert(
                "set".into(),
                json!({
                    "description": "The cell's keys to set, each to its new value as a sheet \
                                    description writes a cell's, such as {\"text\": \"C-104\"} \
                                    or {\"merge\": [1, 2]}; null takes a key away.",
                    "type": "object",
                    "minProperties": 1,
                    "additionalProperties": false,
                    "properties": keys,
                }),
            );
            required.extend([json!("cell"), json!("set")]);
        }
        GridOp::Add(axis) => {
            let (word, across) = match axis {
                Axis::Row => ("row", "column"),
                Axis::Column => ("column", "row"),
            };
            properties.insert(
                "at".into(),
                or_null(place(&format!(
                    "The new {word}'s place, counted from 0: the {word} now there and those \
                     after it move along. Without it, or null, after the last."
                ))),
            );
            properties.insert(
                axis.size_key().into(),
                or_null(json!({
                    "description": format!(
                        "The new {word}'s {}: a length more than 0. Without it, or null, the {} \
                         of the {word} before it, or of the first {word} when it is added at 0.",
                        axis.size_key(),
                        axis.size_key()
                    ),
                    "$ref": "#/$defs/length_more_than_0",
                })),
            );
            properties.insert(
                "cells".into(),
                or_null(json!({
                    "description": format!(
                        "The new {word}'s cells, one for each {across}, each as a sheet \
                         description writes a cell, or in a template's table a value from the \
                         row, {{row: NAME}}, as the cell or its text. Without them, or null, \
                         empty."
                    ),
                    "type": "array",
                    "items": { "$ref": "#/$defs/template_cell" },
                })),
            );
        }
        GridOp::Delete(axis) => {
            let word = axis.word();
            properties.insert(
                word.into(),
                place(&format!("The {word} to delete, counted from 0.")),
            );
            required.push(json!(word));
        }
    }
    json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
    })
}

/// The schema of a block operation, before its `op`. A block's objects are
/// those the block library's schema gives a block, whose text and
/// attribute definitions take a cell, as `block_text` and `block_attdef`.
fn block_schema(op: BlockOp, defs: &Map<String, Value>) -> Value {
    let block = &defs["block_definition"];
    let mut properties = Map::new();
    properties.insert(
        "library".into(),
        json!({
            "description": "The block library, by its path in the set, relative to the set's \
                            folder as the rows write a template's path, such as blocks.yaml. It \
                            may be left out when every template of the set names the same \
                            library.",
            "type": "string",
            "minLength": 1,
        }),
    );
    if op == BlockOp::Import {
        properties.insert(
            "from".into(),
            json!({
                "description": "The DWG or DXF to import the blocks of, told apart by its \
                                extension: its path relative to the set's folder, or absolute.",
                "type": "string",
                "minLength": 1,
            }),
        );
        properties.insert(
            "blocks".into(),
            json!({
                "description": "Only the blocks of these names, in any case, each with every \
                                block it nests. Without them, every named block of the drawing.",
                "type": "array",
                "items": { "type": "string", "minLength": 1 },
            }),
        );
        properties.insert(
            "replace".into(),
            json!({
                "description": "Replace the whole definition of a block the library holds \
                                already, rather than refuse.",
                "type": "boolean",
            }),
        );
        properties.insert(
            "units".into(),
            json!({
                "description": "The unit the drawing's blocks are drawn in, for a drawing that \
                                states none a block library takes: the set's.",
                "enum": Unit::ALL.map(Unit::symbol),
            }),
        );
        return json!({
            "type": "object",
            "required": ["from"],
            "additionalProperties": false,
            "properties": properties,
        });
    }
    let name_description = match op {
        BlockOp::Add => "The new block's name, which the library does not have in any case.",
        BlockOp::Redefine => "The block to redefine, written exactly as the library writes it.",
        BlockOp::Sync => {
            "The block whose inserts to bring into line, written exactly as the library writes \
             it. Without it, every block of the library."
        }
        BlockOp::Import => unreachable!("handled above"),
    };
    properties.insert(
        "name".into(),
        json!({ "description": name_description, "$ref": "#/$defs/name" }),
    );
    let mut required = Vec::new();
    if op != BlockOp::Sync {
        for key in ["base_point", "objects"] {
            properties.insert(key.into(), block["properties"][key].clone());
        }
        required = vec![json!("name"), json!("objects")];
    }
    json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
    })
}

/// `schema` with the definitions it refers to, from `defs`, under `$defs`.
fn with_definitions(mut schema: Value, defs: &Map<String, Value>) -> Value {
    let mut wanted: Vec<String> = Vec::new();
    let mut queue = vec![schema.clone()];
    while let Some(value) = queue.pop() {
        refs(&value, &mut |name| {
            if !wanted.iter().any(|w| w == name) {
                wanted.push(name.to_owned());
                if let Some(def) = defs.get(name) {
                    queue.push(def.clone());
                }
            }
        });
    }
    wanted.sort();
    let mut used = Map::new();
    for name in wanted {
        if let Some(def) = defs.get(&name) {
            used.insert(name, def.clone());
        }
    }
    schema["$defs"] = Value::Object(used);
    schema
}

/// Calls `found` with the name of every definition `value` refers to.
fn refs(value: &Value, found: &mut dyn FnMut(&str)) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    found(name);
                }
            }
            map.values().for_each(|v| refs(v, found));
        }
        Value::Array(items) => items.iter().for_each(|v| refs(v, found)),
        _ => {}
    }
}

/// A regular expression matching any one of `words`, whole, in any case.
fn any_case_of(words: &[&str]) -> String {
    let each: Vec<String> = words.iter().map(|w| any_case(w)).collect();
    format!("^(?:{})$", each.join("|"))
}

/// A regular expression matching `word` in any case.
fn any_case(word: &str) -> String {
    word.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_uppercase(), c.to_ascii_lowercase())
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// One operation of a change set, read.
#[derive(Clone, Debug)]
pub(crate) struct Operation {
    /// Its place in the change set.
    pub index: usize,
    pub spec: &'static Spec,
    pub body: Body,
    /// For a change of a text's contents or a set_attribute whose template
    /// joins the value from a list of parts, the one row value of the parts
    /// it sets, by name, rather than matching the whole text to the parts.
    pub row_value: Option<String>,
    /// The operation as it was sent.
    pub sent: Value,
}

#[derive(Clone, Debug)]
pub(crate) enum Body {
    Add {
        to: AddTo,
        keys: Map<String, Value>,
        /// The snapped points its keys hold, to be written as coordinates.
        snaps: Vec<PointSnap>,
    },
    Change {
        target: Target,
        set: Map<String, Value>,
        /// The snapped points among its keys, each by its key, to be
        /// written as coordinates: a change_viewport's view centre.
        snaps: Vec<(String, PointSnap)>,
        /// Whether it is a change_viewport, which changes a viewport alone.
        viewport: bool,
        /// For a clip_viewport, the boundary it clips the viewport to, or
        /// null to take its boundary away: the keys it sets are worked out
        /// from the viewport as it stands.
        clip: Option<Value>,
    },
    Move {
        target: Target,
        motion: Motion,
    },
    Delete {
        target: Target,
    },
    /// Copies of objects at the places `pattern` gives: a copy or an
    /// array. Each object it copies is given with its key in the
    /// operation, `target` or `targets[N]`.
    Copy {
        targets: Vec<(Target, String)>,
        pattern: Pattern,
    },
    /// Objects turned, scaled or mirrored in place, each given with its key
    /// in the operation, `target` or `targets[N]`.
    Transform {
        targets: Vec<(Target, String)>,
        by: TransformBy,
    },
    SetAttribute {
        target: Target,
        tag: String,
        /// Which of the attributes with the tag, counted from 1, when the
        /// block defines it more than once.
        occurrence: Option<usize>,
        value: String,
    },
    /// A table operation: on the entry `name` of the template `to` names,
    /// with the keys it adds or sets, and the new name a rename gives.
    Table {
        op: TableOp,
        table: Table,
        to: AddTo,
        name: String,
        keys: Map<String, Value>,
        new_name: Option<String>,
    },
    /// An edit of a table's grid: a cell set, or a row or a column added or
    /// deleted.
    TableEdit {
        target: Target,
        edit: TableEdit,
    },
    /// A dimension added from another, `target`, to `point`, with the id
    /// and the name it is given.
    DimensionFrom {
        from: FromOp,
        target: Target,
        point: OpPoint,
        id: Option<String>,
        name: Option<String>,
    },
    /// A leader line added to a multileader, or deleted.
    LeaderLine {
        target: Target,
        edit: LeaderLineEdit,
        /// The snapped points of a leader line added, each with its place
        /// among its points, where the edit holds a stand-in.
        snaps: Vec<(usize, PointSnap)>,
    },
    /// A block operation: on the block `name`, or every block for an
    /// attsync without one, of the block library `library` names, with the
    /// definition an add or a redefinition writes, as sent.
    Block {
        op: BlockOp,
        library: Option<String>,
        name: Option<String>,
        base_point: Option<Value>,
        objects: Option<Vec<Value>>,
        /// What an import_blocks imports.
        import: Option<Import>,
    },
}

/// A snapped point an operation takes, as sent: where it is in the
/// operation, and the snap.
#[derive(Clone, Debug)]
pub(crate) struct PointSnap {
    /// Its place in an add's keys, as a JSON pointer, such as
    /// `/points/0/point`; empty elsewhere.
    pub pointer: String,
    /// Its place in the operation, for an error and the reply, such as
    /// `points[0].point` or `from`.
    pub field: String,
    pub snap: SnapRef,
}

/// How a move moves its object: by a distance, or from a base point to a
/// second point, as AutoCAD's MOVE takes them.
#[derive(Clone, Debug)]
pub(crate) enum Motion {
    By([f64; 3]),
    Between {
        from: Box<OpPoint>,
        to: Box<OpPoint>,
    },
}

/// How a rotate, a scale or a mirror in place transforms its objects.
#[derive(Clone, Debug)]
pub(crate) enum TransformBy {
    /// Turns them by `angle` degrees, anticlockwise, about `base_point`.
    Rotate {
        base_point: Box<OpPoint>,
        angle: f64,
    },
    /// Scales them by `factor` about `base_point`.
    Scale {
        base_point: Box<OpPoint>,
        factor: f64,
    },
    /// Mirrors them across the line through `first` and `second`.
    Mirror {
        first: Box<OpPoint>,
        second: Box<OpPoint>,
    },
}

/// Where a copy or an array puts its copies.
#[derive(Clone, Debug)]
pub(crate) enum Pattern {
    /// A copy: once by each motion, as AutoCAD's COPY takes them.
    Offsets(Vec<Motion>),
    /// Rows and columns, the objects the first row's first column, rows
    /// counting up along Y by `row_spacing` and columns across along X by
    /// `column_spacing`.
    Rectangular {
        rows: usize,
        columns: usize,
        row_spacing: f64,
        column_spacing: f64,
    },
    /// `count` items round `center`, the objects the first, over `angle`
    /// degrees, anticlockwise when positive: turned with it when `rotate`,
    /// or else moved so that `base_point` goes round.
    Polar {
        center: Box<OpPoint>,
        count: usize,
        angle: f64,
        rotate: bool,
        base_point: Option<Box<OpPoint>>,
    },
    /// `count` items along the object `path` names, the objects the first,
    /// `spacing` apart, or spaced evenly over it; turned with the path's
    /// direction when `align`.
    Path {
        path: Target,
        count: usize,
        spacing: Option<f64>,
        align: bool,
    },
    /// One copy mirrored across the line through `first` and `second`: a
    /// mirror that keeps its source, as AutoCAD's MIRROR does by default.
    Mirror {
        first: Box<OpPoint>,
        second: Box<OpPoint>,
    },
}

impl Pattern {
    /// How many places it puts copies at.
    pub(crate) fn places(&self) -> usize {
        match self {
            Pattern::Mirror { .. } => 1,
            Pattern::Offsets(motions) => motions.len(),
            Pattern::Rectangular { rows, columns, .. } => rows * columns - 1,
            Pattern::Polar { count, .. } | Pattern::Path { count, .. } => count - 1,
        }
    }
}

/// A point an operation takes: coordinates, or a snapped point.
#[derive(Clone, Debug)]
pub(crate) enum OpPoint {
    At([f64; 3]),
    Snap(PointSnap),
}

/// What an import_blocks operation imports: the blocks of the drawing at
/// `from`, as sent, those `blocks` names or every one, replacing those the
/// library holds when `replace`, in `units` when the drawing states none.
#[derive(Clone, Debug)]
pub(crate) struct Import {
    pub from: String,
    pub blocks: Vec<String>,
    pub replace: bool,
    pub units: Option<Unit>,
}

/// Where an object is added.
#[derive(Clone, Debug)]
pub(crate) enum AddTo {
    /// To the sheet's row, by the sheet's name.
    Sheet(String),
    /// To the template, as the rows write its path.
    Template(String),
}

/// How an operation names the object it edits.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// By its ID: the sheet's name and the ID in its file.
    Id { sheet: String, id: String },
    /// By its name, on the sheet given or on the one sheet it is on.
    Name { name: String, sheet: Option<String> },
}

impl Target {
    pub(crate) fn words(&self) -> String {
        match self {
            Target::Id { sheet, id } => format!("`{sheet}/{id}`"),
            Target::Name {
                name,
                sheet: Some(sheet),
            } => format!("the object named `{name}` on sheet `{sheet}`"),
            Target::Name { name, sheet: None } => format!("the object named `{name}`"),
        }
    }
}

/// A change set as sent: its operations and the sheets it fixes.
pub(crate) struct ChangeSet {
    pub operations: Vec<Operation>,
    pub fixes: Vec<(String, usize)>,
}

/// Reads a change set sent as plain data, with lengths in `units`. A change
/// set that is wrong in itself, such as one naming an operation that does
/// not exist, is refused whole: the error names the operation and lists the
/// valid choices.
pub(crate) fn read_change_set(value: &Value, units: Unit) -> Result<ChangeSet, Error> {
    read_change_set_inner(value, units, None)
}

/// Reads a change set sent against an original DWG whose sheet name is
/// `default_sheet`: operations that take a `sheet` (`add_*`, `add_layer`,
/// `change_layer`, etc.) may omit `sheet`, defaulting to `default_sheet`.
pub(crate) fn read_change_set_for_original(
    value: &Value,
    units: Unit,
    default_sheet: &str,
) -> Result<ChangeSet, Error> {
    read_change_set_inner(value, units, Some(default_sheet))
}

fn read_change_set_inner(
    value: &Value,
    units: Unit,
    default_sheet: Option<&str>,
) -> Result<ChangeSet, Error> {
    let Value::Object(map) = value else {
        return Err(Error::new(
            ErrorCode::WrongType,
            "a change set is a mapping, {\"operations\": [...]}",
        ));
    };
    if let Some(key) = map.keys().find(|k| !CHANGE_SET_KEYS.contains(&k.as_str())) {
        return Err(Error::new(
            ErrorCode::UnknownKey,
            format!(
                "unknown key `{key}`; a change set takes {}",
                CHANGE_SET_KEYS.join(", ")
            ),
        )
        .at_field(key.clone()));
    }
    let Some(list) = map.get("operations") else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "missing key `operations`; a change set lists its operations under it",
        )
        .at_field("operations"));
    };
    let Value::Array(items) = list else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected a list of operations")
                .at_field("operations"),
        );
    };
    // What each object an add writes requires, as the schema states it, and
    // where each takes a point, which may be snapped.
    let objects = sheet_description_schema()["$defs"].take();
    let defs = definitions();
    let operations = items
        .iter()
        .enumerate()
        .map(|(index, item)| operation(index, item, units, &objects, &defs, default_sheet))
        .collect::<Result<_, _>>()?;
    let fixes = match map.get("fixes") {
        None => Vec::new(),
        Some(Value::Array(sheets)) => sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| match sheet {
                Value::String(name) => Ok((name.clone(), i)),
                _ => Err(Error::new(ErrorCode::WrongType, "expected a sheet's name")
                    .at_field(format!("fixes[{i}]"))),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(
                Error::new(ErrorCode::WrongType, "expected a list of sheets' names")
                    .at_field("fixes"),
            )
        }
    };
    if let Some(mark) = map.get("guide") {
        super::guides::read_mark(mark)?;
    }
    Ok(ChangeSet { operations, fixes })
}

/// What an import_blocks operation sent as `map` imports, or the code,
/// message and key of what is wrong with it.
fn read_import(map: &Map<String, Value>) -> Result<Import, (ErrorCode, String, &'static str)> {
    let from = match map.get("from") {
        Some(Value::String(from)) if !from.is_empty() => from.clone(),
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected the drawing's path, a DWG or a DXF, relative to the set's folder or \
                 absolute"
                    .to_owned(),
                "from",
            ))
        }
        None => {
            return Err((
                ErrorCode::MissingKey,
                "missing key `from`, the DWG or DXF to import the blocks of".to_owned(),
                "from",
            ))
        }
    };
    let blocks = match map.get("blocks") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(name) if !name.is_empty() => Ok(name.clone()),
                _ => Err((
                    ErrorCode::WrongType,
                    "expected a list of the blocks' names".to_owned(),
                    "blocks",
                )),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected a list of the blocks' names".to_owned(),
                "blocks",
            ))
        }
    };
    let replace = match map.get("replace") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected true or false".to_owned(),
                "replace",
            ))
        }
    };
    let units = match map.get("units") {
        None => None,
        Some(Value::String(symbol)) if Unit::from_symbol(symbol).is_some() => {
            Unit::from_symbol(symbol)
        }
        Some(_) => {
            return Err((
                ErrorCode::UnknownUnit,
                format!("expected a unit, one of {}", Unit::symbols()),
                "units",
            ))
        }
    };
    Ok(Import {
        from,
        blocks,
        replace,
        units,
    })
}

/// The edit an operation on a table's grid sent as `map` makes, or the
/// code, message and key of what is wrong with it.
fn read_grid_edit(
    op: GridOp,
    map: &Map<String, Value>,
) -> Result<TableEdit, (ErrorCode, String, String)> {
    let place = |key: &str, what: &str, required: bool| match map.get(key) {
        None | Some(Value::Null) if !required => Ok(None),
        Some(Value::Number(n)) if whole(n).is_some() => {
            Ok(whole(n).and_then(|n| usize::try_from(n).ok()))
        }
        None => Err((
            ErrorCode::MissingKey,
            format!("missing key `{key}`, {what}"),
            key.to_owned(),
        )),
        Some(_) => Err((
            ErrorCode::WrongType,
            format!("expected {what}, a whole number from 0"),
            key.to_owned(),
        )),
    };
    Ok(match op {
        GridOp::SetCell => {
            let shape = "the cell, [row, column], two whole numbers from 0, counted from the top \
                         left";
            let cell = match map.get("cell") {
                Some(Value::Array(items)) if items.len() == 2 => {
                    let mut cell = [0; 2];
                    for (i, item) in items.iter().enumerate() {
                        cell[i] = match item {
                            Value::Number(n) => whole(n).and_then(|n| usize::try_from(n).ok()),
                            _ => None,
                        }
                        .ok_or_else(|| {
                            (
                                ErrorCode::WrongType,
                                format!("expected {shape}"),
                                format!("cell[{i}]"),
                            )
                        })?;
                    }
                    cell
                }
                Some(_) => {
                    return Err((
                        ErrorCode::WrongType,
                        format!("expected {shape}"),
                        "cell".to_owned(),
                    ))
                }
                None => {
                    return Err((
                        ErrorCode::MissingKey,
                        format!("missing key `cell`, {shape}"),
                        "cell".to_owned(),
                    ))
                }
            };
            let set = match map.get("set") {
                Some(Value::Object(set)) if !set.is_empty() => {
                    if let Some(key) = set.keys().find(|k| !CELL_KEYS.contains(&k.as_str())) {
                        return Err((
                            ErrorCode::UnknownKey,
                            format!("unknown key `{key}`; a cell takes {}", CELL_KEYS.join(", ")),
                            format!("set.{key}"),
                        ));
                    }
                    set.clone()
                }
                Some(_) => {
                    return Err((
                        ErrorCode::WrongType,
                        "expected a mapping of at least one of the cell's keys to its new value, \
                         such as {\"text\": \"C-104\"}"
                            .to_owned(),
                        "set".to_owned(),
                    ))
                }
                None => {
                    return Err((
                        ErrorCode::MissingKey,
                        "missing key `set`, the cell's keys to set and their new values".to_owned(),
                        "set".to_owned(),
                    ))
                }
            };
            TableEdit::SetCell { cell, set }
        }
        GridOp::Add(axis) => {
            let word = axis.word();
            let at = place("at", &format!("the new {word}'s place"), false)?;
            let size = map.get(axis.size_key()).filter(|v| !v.is_null()).cloned();
            let cells = match map.get("cells") {
                None | Some(Value::Null) => None,
                Some(Value::Array(items)) => Some(items.clone()),
                Some(_) => {
                    return Err((
                        ErrorCode::WrongType,
                        format!("expected the new {word}'s cells, a list of cells"),
                        "cells".to_owned(),
                    ))
                }
            };
            TableEdit::Add {
                axis,
                at,
                size,
                cells,
            }
        }
        GridOp::Delete(axis) => {
            let word = axis.word();
            let index = place(word, &format!("the {word} to delete"), true)?.unwrap_or_default();
            TableEdit::Delete { axis, index }
        }
    })
}

/// The error for operation `index`, named `name`, at `field` within it.
pub(crate) fn op_error(
    index: usize,
    name: &str,
    code: ErrorCode,
    message: impl AsRef<str>,
    field: &str,
) -> Error {
    let path = if field.is_empty() {
        format!("operations[{index}]")
    } else {
        format!("operations[{index}].{field}")
    };
    Error::new(
        code,
        format!("operation {index} (`{name}`): {}", message.as_ref()),
    )
    .at_field(path)
}

/// Reads operation `index`, whose lengths are in `units`. `objects` holds
/// the schema of each object an add writes, whose required keys an add
/// must give: an operation that leaves one out is wrong in itself, so the
/// whole change set is refused before anything is applied.
fn operation(
    index: usize,
    value: &Value,
    units: Unit,
    objects: &Value,
    defs: &Map<String, Value>,
    default_sheet: Option<&str>,
) -> Result<Operation, Error> {
    let Value::Object(map) = value else {
        return Err(Error::new(
            ErrorCode::WrongType,
            format!("operation {index} is not a mapping; an operation names itself under op"),
        )
        .at_field(format!("operations[{index}]")));
    };
    let listed = || {
        SPECS
            .iter()
            .map(Spec::listed)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let name = match map.get(OP_KEY) {
        Some(Value::String(name)) => name,
        _ => {
            return Err(Error::new(
                ErrorCode::MissingKey,
                format!(
                    "operation {index} names no operation under op; the operations: {}",
                    listed()
                ),
            )
            .at_field(format!("operations[{index}].op")))
        }
    };
    let Some(spec) = SPECS.iter().find(|s| s.named(name)) else {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "operation {index}: there is no operation `{name}`; the operations, with their \
                 AutoCAD aliases: {}",
                listed()
            ),
        )
        .at_field(format!("operations[{index}].op")));
    };
    let fail = |code: ErrorCode, message: String, field: &str| {
        op_error(index, spec.name, code, message, field)
    };
    let keys = |allowed: &[&str]| -> Result<(), Error> {
        match map
            .keys()
            .find(|k| *k != OP_KEY && !allowed.contains(&k.as_str()))
        {
            Some(key) => Err(fail(
                ErrorCode::UnknownKey,
                format!(
                    "unknown key `{key}`; {} takes op, {}",
                    spec.name,
                    allowed.join(", ")
                ),
                key,
            )),
            None => Ok(()),
        }
    };
    let target = || -> Result<Target, Error> {
        let value = map.get("target").ok_or_else(|| {
            fail(
                ErrorCode::MissingKey,
                "missing key `target`, the object to edit, by its ID, {\"id\": \
                 \"DEMO-001/border\"}, or by its name, {\"name\": \"title-block\", \"sheet\": \
                 \"DEMO-001\"}"
                    .to_owned(),
                "target",
            )
        })?;
        read_target(value).map_err(|(code, message, field)| fail(code, message, &field))
    };
    let body = match spec.kind {
        Kind::Add(object, kind) => {
            let others = kind.map_or(&[][..], |kind| kind_keys(dimension_kind(kind)).2);
            let own: Vec<&str> = objects_table(object)
                .into_iter()
                .filter(|k| *k != COPY_OF)
                .filter(|k| kind.is_none() || *k != DIMENSION_KIND_KEY)
                .filter(|k| !others.contains(k))
                .collect();
            let mut allowed = vec!["sheet", "template"];
            allowed.extend(&own);
            keys(&allowed)?;
            let mut with_kind = map.clone();
            if let Some(kind) = kind {
                with_kind.insert(DIMENSION_KIND_KEY.into(), Value::String(kind.to_owned()));
            }
            // A dimension's kind requires its own keys.
            if let Some(kind) = kind {
                let (needs, _, _) = kind_keys(dimension_kind(kind));
                if let Some(missing) = needs.iter().find(|k| !map.contains_key(**k)) {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        format!(
                            "missing key `{missing}`; {} {kind} dimension needs {}",
                            article(kind),
                            needs.join(" and ")
                        ),
                        missing,
                    ));
                }
            }
            if let Some(missing) = missing_required(&objects[object], &with_kind) {
                return Err(fail(
                    ErrorCode::MissingKey,
                    format!(
                        "missing key `{}`; {} {object} needs {}",
                        missing.first,
                        article(object),
                        missing.needs
                    ),
                    &missing.first,
                ));
            }
            let to = match (map.get("sheet"), map.get("template")) {
                (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                (Some(_), Some(_)) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give sheet or template, not both: an object added to a sheet goes in \
                         its row, and one added to a template is on every sheet built from it"
                            .to_owned(),
                        "",
                    ))
                }
                (None, None) => match default_sheet {
                    Some(sheet) => AddTo::Sheet(sheet.to_owned()),
                    None => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `sheet`: name the sheet to add it to, in its row, or \
                                 give template to add it to a template"
                                .to_owned(),
                            "sheet",
                        ))
                    }
                },
                (Some(_), None) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a sheet's name".to_owned(),
                        "sheet",
                    ))
                }
                (None, Some(_)) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a template's path".to_owned(),
                        "template",
                    ))
                }
            };
            // A key sent as `null` is kept: the record writes it as a bare
            // null, and the sheet's reader reads it as it reads one in any
            // file (rule 8 of the YAML subset), so it leaves an optional text
            // or path field out and is an error anywhere else.
            let mut keys: Map<String, Value> = Map::new();
            // A dimension's kind comes first, as a sheet description writes it.
            if let Some(kind) = kind {
                keys.insert(DIMENSION_KIND_KEY.into(), Value::String(kind.to_owned()));
            }
            keys.extend(
                map.iter()
                    .filter(|(k, _)| !matches!(k.as_str(), OP_KEY | "sheet" | "template"))
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            let schema = defs.get(&format!("op_{object}")).unwrap_or(&defs[object]);
            let mut snaps = Vec::new();
            for (key, value) in &keys {
                let slots: Vec<&Value> = schema["properties"].get(key).into_iter().collect();
                snaps_in(value, &slots, defs, &format!("/{key}"), key, &mut snaps)
                    .map_err(|(code, message, field)| fail(code, message, &field))?;
            }
            Body::Add { to, keys, snaps }
        }
        Kind::Change => {
            keys(&["target", "set", ROW_VALUE_KEY])?;
            let target = target()?;
            let set = match map.get("set") {
                Some(Value::Object(set)) if !set.is_empty() => set.clone(),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a mapping of at least one key to its new value, such as \
                         {\"height\": 0.3}"
                            .to_owned(),
                        "set",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `set`, the keys to set and their new values".to_owned(),
                        "set",
                    ))
                }
            };
            Body::Change {
                target,
                set,
                snaps: Vec::new(),
                viewport: false,
                clip: None,
            }
        }
        Kind::ChangeViewport => {
            keys(&["target", "scale", "view_center", "locked"])?;
            let target = target()?;
            let mut set = Map::new();
            let mut snaps = Vec::new();
            if let Some(scale) = map.get("scale") {
                if !matches!(scale, Value::Number(_) | Value::String(_)) {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a scale, a ratio such as `1:4` or a number such as 0.25"
                            .to_owned(),
                        "scale",
                    ));
                }
                set.insert("scale".into(), scale.clone());
            }
            if let Some(center) = map.get("view_center") {
                match op_point(center, units, "view_center")
                    .map_err(|(code, message, field)| fail(code, message, &field))?
                {
                    OpPoint::At(at) => {
                        set.insert("view_center".into(), json!(at));
                    }
                    OpPoint::Snap(snap) => {
                        // Stands in until the snap is worked out.
                        set.insert("view_center".into(), json!([0.0, 0.0, 0.0]));
                        snaps.push(("view_center".to_owned(), snap));
                    }
                }
            }
            if let Some(locked) = map.get("locked") {
                let Value::Bool(_) = locked else {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `true` or `false`".to_owned(),
                        "locked",
                    ));
                };
                set.insert("locked".into(), locked.clone());
            }
            if set.is_empty() {
                return Err(fail(
                    ErrorCode::MissingKey,
                    "missing key: change_viewport sets scale, view_center or locked".to_owned(),
                    "scale",
                ));
            }
            Body::Change {
                target,
                set,
                snaps,
                viewport: true,
                clip: None,
            }
        }
        Kind::ClipViewport => {
            keys(&["target", "clip"])?;
            let target = target()?;
            let clip =
                    match map.get("clip") {
                        Some(clip @ (Value::Object(_) | Value::Null)) => clip.clone(),
                        Some(_) => return Err(fail(
                            ErrorCode::WrongType,
                            "expected a boundary, `{points: [...]}` or `{center: [x, y, 0], radius: r}`, or \n                         null to take the viewport's boundary away"
                                .to_owned(),
                            "clip",
                        )),
                        None => return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `clip`, the boundary to clip the viewport to, or null to \
                         take its boundary away"
                                .to_owned(),
                            "clip",
                        )),
                    };
            Body::Change {
                target,
                set: Map::new(),
                snaps: Vec::new(),
                viewport: true,
                clip: Some(clip),
            }
        }
        Kind::SetFrozenLayers => {
            keys(&["target", "frozen_layers"])?;
            let target = target()?;
            let layers = match map.get("frozen_layers") {
                Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
                    Value::Array(items.clone())
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a list of layer names, such as [\"wiring\"], or [] to thaw \
                         every layer in the viewport"
                            .to_owned(),
                        "frozen_layers",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `frozen_layers`, the layers to freeze in the viewport"
                            .to_owned(),
                        "frozen_layers",
                    ))
                }
            };
            let mut set = Map::new();
            // An empty list thaws every layer: the key is taken away.
            let empty = layers.as_array().is_some_and(Vec::is_empty);
            set.insert(
                "frozen_layers".into(),
                if empty { Value::Null } else { layers },
            );
            Body::Change {
                target,
                set,
                snaps: Vec::new(),
                viewport: true,
                clip: None,
            }
        }
        Kind::Move => {
            keys(&["target", "by", "from", "to"])?;
            let target = target()?;
            let given = |key: &str| map.contains_key(key);
            let motion = match (given("by"), given("from"), given("to")) {
                (true, false, false) => match map.get("by") {
                    Some(Value::Array(items)) if items.len() == 3 => {
                        let mut by = [0.0; 3];
                        for (axis, item) in items.iter().enumerate() {
                            by[axis] = length(item, units).map_err(|message| {
                                fail(ErrorCode::BadNumber, message, &format!("by[{axis}]"))
                            })?;
                        }
                        Motion::By(by)
                    }
                    _ => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected how far to move it, three lengths [dx, dy, dz]".to_owned(),
                            "by",
                        ))
                    }
                },
                (false, true, true) => {
                    let point = |key: &str| {
                        op_point(&map[key], units, key)
                            .map_err(|(code, message, field)| fail(code, message, &field))
                    };
                    Motion::Between {
                        from: Box::new(point("from")?),
                        to: Box::new(point("to")?),
                    }
                }
                (false, false, false) => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `by`, how far to move it, [dx, dy, dz]; or give from, a \
                         base point, and to, where it lands"
                            .to_owned(),
                        "by",
                    ))
                }
                (false, true, false) | (false, false, true) => {
                    let missing = if given("from") { "to" } else { "from" };
                    return Err(fail(
                        ErrorCode::MissingKey,
                        format!(
                            "missing key `{missing}`: a move from a base point to a second \
                             point takes both, from and to"
                        ),
                        missing,
                    ));
                }
                (true, _, _) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give by, or from and to, not both: a move is by a distance, or from a \
                         base point to a second point"
                            .to_owned(),
                        "",
                    ))
                }
            };
            Body::Move { target, motion }
        }
        Kind::Delete => {
            keys(&["target"])?;
            Body::Delete { target: target()? }
        }
        Kind::Copy | Kind::Array => {
            let pattern = if spec.kind == Kind::Copy {
                keys(&["target", "targets", "by", "offsets", "from", "to"])?;
                read_offsets(map, units)
            } else {
                let mut allowed = vec!["target", "targets", "kind"];
                for kind_keys in ARRAY_KEYS {
                    for key in kind_keys {
                        if !allowed.contains(key) {
                            allowed.push(key);
                        }
                    }
                }
                keys(&allowed)?;
                read_array(map, name, units)
            }
            .map_err(|(code, message, field)| fail(code, message, &field))?;
            let targets =
                read_targets(map).map_err(|(code, message, field)| fail(code, message, &field))?;
            let copies = pattern.places().saturating_mul(targets.len());
            if copies > MOST_COPIES {
                return Err(fail(
                    ErrorCode::BadValue,
                    format!(
                        "it would make {copies} copies, {} places times {} objects; one \
                         operation makes at most {MOST_COPIES}",
                        pattern.places(),
                        targets.len()
                    ),
                    "",
                ));
            }
            Body::Copy { targets, pattern }
        }
        Kind::Rotate | Kind::Scale | Kind::Mirror => read_transform(map, spec, units)
            .map_err(|(code, message, field)| fail(code, message, &field))?,
        Kind::SetAttribute => {
            keys(&["target", "tag", "occurrence", "value", ROW_VALUE_KEY])?;
            let target = target()?;
            let occurrence =
                match map.get("occurrence") {
                    None => None,
                    // A whole number, written `1` or `1.0`, as JSON Schema's
                    // integer takes it.
                    Some(Value::Number(n)) if whole(n).is_some_and(|n| n >= 1) => {
                        whole(n).and_then(|n| usize::try_from(n).ok())
                    }
                    Some(_) => return Err(fail(
                        ErrorCode::WrongType,
                        "expected the occurrence, a whole number from 1: which of the attributes \
                         with this tag to set, in the order the block defines them"
                            .to_owned(),
                        "occurrence",
                    )),
                };
            let tag = match map.get("tag") {
                Some(Value::String(tag)) => tag.clone(),
                _ => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `tag`, the attribute tag to set, such as REV".to_owned(),
                        "tag",
                    ))
                }
            };
            let value = match map.get("value") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the attribute's value, one line of text".to_owned(),
                        "value",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `value`, the attribute's new value".to_owned(),
                        "value",
                    ))
                }
            };
            Body::SetAttribute {
                target,
                tag,
                occurrence,
                value,
            }
        }
        Kind::Grid(grid_op) => {
            let allowed: Vec<&str> = match grid_op {
                GridOp::SetCell => vec!["target", "cell", "set"],
                GridOp::Add(axis) => vec!["target", "at", axis.size_key(), "cells"],
                GridOp::Delete(axis) => vec!["target", axis.word()],
            };
            keys(&allowed)?;
            let target = target()?;
            let edit = read_grid_edit(grid_op, map)
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            Body::TableEdit { target, edit }
        }
        Kind::LeaderLine(line_op) => {
            let mut line_snaps: Vec<(usize, PointSnap)> = Vec::new();
            match line_op {
                LeaderLineOp::Add => keys(&["target", "points"])?,
                LeaderLineOp::Delete => keys(&["target", "leader"])?,
            }
            let target = target()?;
            let edit = match line_op {
                LeaderLineOp::Add => {
                    let shape = "the new leader line's points, from its arrow's point up to the \
                                 point where the leader lines meet the landing, each [x, y, z]";
                    let Some(Value::Array(items)) = map.get("points") else {
                        let code = if map.contains_key("points") {
                            ErrorCode::WrongType
                        } else {
                            ErrorCode::MissingKey
                        };
                        return Err(fail(code, format!("expected {shape}"), "points"));
                    };
                    if items.is_empty() {
                        return Err(fail(
                            ErrorCode::BadValue,
                            "a leader line takes at least one point of its own, its arrow's, \
                             before the point where it meets the landing, which the operation \
                             appends"
                                .to_owned(),
                            "points",
                        ));
                    }
                    let mut points = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let at = format!("points[{i}]");
                        match op_point(item, units, &at)
                            .map_err(|(code, message, field)| fail(code, message, &field))?
                        {
                            OpPoint::At(point) => points.push(point),
                            // A stand-in, until the snap is worked out.
                            OpPoint::Snap(snap) => {
                                points.push([0.0; 3]);
                                line_snaps.push((i, snap));
                            }
                        }
                    }
                    LeaderLineEdit::Add { points }
                }
                LeaderLineOp::Delete => {
                    let leader = match map.get("leader") {
                        Some(Value::Number(n)) => whole(n).and_then(|n| usize::try_from(n).ok()),
                        None => {
                            return Err(fail(
                                ErrorCode::MissingKey,
                                "missing key `leader`, the place of the leader line to delete, \
                                 counted from 0"
                                    .to_owned(),
                                "leader",
                            ))
                        }
                        Some(_) => None,
                    };
                    let Some(leader) = leader else {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected the leader line to delete, a whole number from 0".to_owned(),
                            "leader",
                        ));
                    };
                    LeaderLineEdit::Delete { leader }
                }
            };
            Body::LeaderLine {
                target,
                edit,
                snaps: line_snaps,
            }
        }
        Kind::DimensionFrom(from) => {
            keys(&["target", "point", "id", "name"])?;
            let target = target()?;
            let Some(sent) = map.get("point") else {
                return Err(fail(
                    ErrorCode::MissingKey,
                    "missing key `point`, the new dimension's second extension line's origin, \
                     or an ordinate dimension's feature"
                        .to_owned(),
                    "point",
                ));
            };
            let point = op_point(sent, units, "point")
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            let text = |key: &str| -> Result<Option<String>, Error> {
                match map.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
                    Some(_) => Err(fail(
                        ErrorCode::WrongType,
                        format!("expected the new dimension's {key}, a text"),
                        key,
                    )),
                }
            };
            Body::DimensionFrom {
                from,
                target,
                point,
                id: text("id")?,
                name: text("name")?,
            }
        }
        Kind::Block(op) => {
            let allowed: &[&str] = match op {
                BlockOp::Sync => &["library", "name"],
                BlockOp::Add | BlockOp::Redefine => &["library", "name", "base_point", "objects"],
                BlockOp::Import => &["library", "from", "blocks", "replace", "units"],
            };
            keys(allowed)?;
            let library = match map.get("library") {
                None => None,
                Some(Value::String(path)) if !path.is_empty() => Some(path.clone()),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block library's path in the set, such as blocks.yaml"
                            .to_owned(),
                        "library",
                    ))
                }
            };
            let name = match map.get("name") {
                Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
                None if matches!(op, BlockOp::Sync | BlockOp::Import) => None,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block's name".to_owned(),
                        "name",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `name`, the block's name".to_owned(),
                        "name",
                    ))
                }
            };
            let objects =
                match (op, map.get("objects")) {
                    (BlockOp::Sync | BlockOp::Import, _) => None,
                    (_, Some(Value::Array(items))) if !items.is_empty() => Some(items.clone()),
                    (_, Some(Value::Array(_))) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "a block holds at least one object".to_owned(),
                            "objects",
                        ))
                    }
                    (_, Some(_)) => return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block's objects, a list of mappings each with one key, its \
                         type, such as {\"line\": {\"from\": [0, 0, 0], \"to\": [1, 0, 0]}}"
                            .to_owned(),
                        "objects",
                    )),
                    (_, None) => return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `objects`, the block's objects; a block holds at least one"
                            .to_owned(),
                        "objects",
                    )),
                };
            let import = if op == BlockOp::Import {
                Some(
                    read_import(map)
                        .map_err(|(code, message, field)| fail(code, message, field))?,
                )
            } else {
                None
            };
            Body::Block {
                op,
                library,
                name,
                base_point: map.get("base_point").cloned(),
                objects,
                import,
            }
        }
        Kind::Table(op, table) => {
            let mut allowed = vec!["sheet", "template", "name"];
            match op {
                TableOp::Add => allowed.extend(table.entry_keys()),
                TableOp::Change => allowed.push("set"),
                TableOp::Rename => allowed.push("to"),
                TableOp::Delete => {}
            }
            keys(&allowed)?;
            let to =
                match (map.get("sheet"), map.get("template")) {
                    (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                    (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                    (Some(_), Some(_)) => {
                        return Err(fail(
                            ErrorCode::Usage,
                            "give sheet or template, not both: either names the template to edit"
                                .to_owned(),
                            "",
                        ))
                    }
                    (None, None) => match default_sheet {
                        Some(sheet) => AddTo::Sheet(sheet.to_owned()),
                        None => return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `template`: name the template to edit, or a sheet built \
                             from it with sheet"
                                .to_owned(),
                            "template",
                        )),
                    },
                    (Some(_), None) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a sheet's name".to_owned(),
                            "sheet",
                        ))
                    }
                    (None, Some(_)) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a template's path".to_owned(),
                            "template",
                        ))
                    }
                };
            let text = |key: &str, what: &str| -> Result<String, Error> {
                match map.get(key) {
                    Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
                    Some(_) => Err(fail(
                        ErrorCode::WrongType,
                        format!("expected {what}, a name"),
                        key,
                    )),
                    None => Err(fail(
                        ErrorCode::MissingKey,
                        format!("missing key `{key}`, {what}"),
                        key,
                    )),
                }
            };
            let name = text("name", &format!("the {}'s name", table.word()))?;
            let new_name = match op {
                TableOp::Rename => Some(text("to", &format!("the {}'s new name", table.word()))?),
                _ => None,
            };
            let keys: Map<String, Value> = match op {
                TableOp::Add => {
                    if let Some(missing) = missing_required(&objects[table.def()], map) {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            format!(
                                "missing key `{}`; a {} needs {}",
                                missing.first,
                                table.word(),
                                missing.needs
                            ),
                            &missing.first,
                        ));
                    }
                    map.iter()
                        .filter(|(k, _)| table.entry_keys().contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                }
                TableOp::Change => match map.get("set") {
                    Some(Value::Object(set)) if !set.is_empty() => {
                        if let Some(key) = set
                            .keys()
                            .find(|k| !table.entry_keys().contains(&k.as_str()))
                        {
                            return Err(fail(
                                ErrorCode::UnknownKey,
                                format!(
                                    "unknown key `{key}`; a {} takes {}",
                                    table.word(),
                                    table.entry_keys().join(", ")
                                ),
                                &format!("set.{key}"),
                            ));
                        }
                        set.clone()
                    }
                    Some(_) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a mapping of at least one key to its new value".to_owned(),
                            "set",
                        ))
                    }
                    None => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `set`, the keys to set and their new values".to_owned(),
                            "set",
                        ))
                    }
                },
                TableOp::Rename | TableOp::Delete => Map::new(),
            };
            Body::Table {
                op,
                table,
                to,
                name,
                keys,
                new_name,
            }
        }
    };
    // Only change and set_attribute take it: the keys above refuse it
    // elsewhere.
    let row_value = match map.get(ROW_VALUE_KEY) {
        None => None,
        Some(Value::String(name)) => match value_name_problem(name) {
            None => Some(name.clone()),
            Some(problem) => return Err(fail(ErrorCode::BadValue, problem, ROW_VALUE_KEY)),
        },
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected row_value, the name of the one row value to set of the parts the \
                 template joins the value from, such as PANEL"
                    .to_owned(),
                ROW_VALUE_KEY,
            ))
        }
    };
    Ok(Operation {
        index,
        spec,
        body,
        row_value,
        sent: value.clone(),
    })
}

/// The code, message and key of what is wrong with an operation's key.
type KeyError = (ErrorCode, String, String);

/// `error`, at a key `target` names inside the operation's key `at`, such
/// as `targets[1]` or `path`.
fn target_at(at: &str, (code, message, field): KeyError) -> KeyError {
    let field = match field.strip_prefix("target") {
        Some(rest) => format!("{at}{rest}"),
        None => field,
    };
    (code, message, field)
}

/// A rotate, a scale or a mirror sent as `map`, as `spec` names it: a
/// mirror that keeps its source is a copy, mirrored.
fn read_transform(map: &Map<String, Value>, spec: &Spec, units: Unit) -> Result<Body, KeyError> {
    let allowed: &[&str] = match spec.kind {
        Kind::Rotate => &["target", "targets", "base_point", "angle"],
        Kind::Scale => &["target", "targets", "base_point", "factor"],
        _ => &[
            "target",
            "targets",
            "first_point",
            "second_point",
            "erase_source",
        ],
    };
    if let Some(key) = map
        .keys()
        .find(|k| *k != OP_KEY && !allowed.contains(&k.as_str()))
    {
        return Err((
            ErrorCode::UnknownKey,
            format!(
                "unknown key `{key}`; {} takes op, {}",
                spec.name,
                allowed.join(", ")
            ),
            key.clone(),
        ));
    }
    let verb = match spec.kind {
        Kind::Rotate => "turn",
        Kind::Scale => "scale",
        _ => "mirror",
    };
    let targets = read_targets_to(map, verb)?;
    let point = |key: &str, what: &str| -> Result<Box<OpPoint>, KeyError> {
        match map.get(key) {
            Some(value) => op_point(value, units, key).map(Box::new),
            None => Err((
                ErrorCode::MissingKey,
                format!("missing key `{key}`, {what}: a point or a snapped point"),
                key.to_owned(),
            )),
        }
    };
    let number = |key: &str, what: &str| -> Result<f64, KeyError> {
        match map.get(key) {
            Some(Value::Number(n)) => match n.as_f64() {
                Some(v) if v.is_finite() => Ok(v),
                _ => Err((
                    ErrorCode::BadNumber,
                    format!("{key} is too large; expected {what}"),
                    key.to_owned(),
                )),
            },
            Some(_) => Err((
                ErrorCode::WrongType,
                format!("expected {what}"),
                key.to_owned(),
            )),
            None => Err((
                ErrorCode::MissingKey,
                format!("missing key `{key}`, {what}"),
                key.to_owned(),
            )),
        }
    };
    let by = match spec.kind {
        Kind::Rotate => TransformBy::Rotate {
            base_point: point("base_point", "the point they turn about")?,
            angle: number(
                "angle",
                "how far they turn, in degrees anticlockwise, a number such as 90",
            )?,
        },
        Kind::Scale => {
            let base_point = point("base_point", "the point they are scaled about")?;
            let factor = number(
                "factor",
                "how much they are enlarged, a number more than 0 such as 2",
            )?;
            if factor <= 0.0 {
                return Err((
                    ErrorCode::BadValue,
                    format!(
                        "factor is {factor}; a scale factor is more than 0, less than 1 to make \
                         them smaller"
                    ),
                    "factor".to_owned(),
                ));
            }
            TransformBy::Scale { base_point, factor }
        }
        _ => {
            let first = point("first_point", "the first point of the mirror line")?;
            let second = point("second_point", "the second point of the mirror line")?;
            if let (OpPoint::At(a), OpPoint::At(b)) = (first.as_ref(), second.as_ref()) {
                if a[0] == b[0] && a[1] == b[1] {
                    return Err((
                        ErrorCode::BadValue,
                        "second_point is first_point seen from above, so the two give no line \
                         to mirror across; give two points apart"
                            .to_owned(),
                        "second_point".to_owned(),
                    ));
                }
            }
            let erase = match map.get("erase_source") {
                None => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => {
                    return Err((
                        ErrorCode::WrongType,
                        "expected true or false".to_owned(),
                        "erase_source".to_owned(),
                    ))
                }
            };
            if !erase {
                return Ok(Body::Copy {
                    targets,
                    pattern: Pattern::Mirror { first, second },
                });
            }
            TransformBy::Mirror { first, second }
        }
    };
    Ok(Body::Transform { targets, by })
}

/// The objects a copy or an array sent as `map` copies: `target`, one, or
/// `targets`, several, each with its key in the operation.
fn read_targets(map: &Map<String, Value>) -> Result<Vec<(Target, String)>, KeyError> {
    read_targets_to(map, "copy")
}

/// The objects an operation sent as `map` does `verb` to, such as `copy`:
/// `target`, one, or `targets`, several, each with its key in the
/// operation.
fn read_targets_to(
    map: &Map<String, Value>,
    verb: &str,
) -> Result<Vec<(Target, String)>, KeyError> {
    match (map.get("target"), map.get("targets")) {
        (Some(_), Some(_)) => Err((
            ErrorCode::Usage,
            "give target, one object, or targets, several, not both".to_owned(),
            String::new(),
        )),
        (Some(one), None) => Ok(vec![(read_target(one)?, "target".to_owned())]),
        (None, Some(Value::Array(items))) if !items.is_empty() => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let at = format!("targets[{i}]");
                read_target(item)
                    .map(|t| (t, at.clone()))
                    .map_err(|e| target_at(&at, e))
            })
            .collect(),
        (None, Some(Value::Array(_))) => Err((
            ErrorCode::BadValue,
            "targets lists no object; list at least one".to_owned(),
            "targets".to_owned(),
        )),
        (None, Some(_)) => Err((
            ErrorCode::WrongType,
            "expected a list of objects, each {\"id\": ...} or {\"name\": ...}".to_owned(),
            "targets".to_owned(),
        )),
        (None, None) => Err((
            ErrorCode::MissingKey,
            format!(
                "missing key `target`, the object to {verb}, by its ID, {{\"id\": \
                 \"DEMO-001/border\"}}, or by its name; or targets, a list of them"
            ),
            "target".to_owned(),
        )),
    }
}

/// Three lengths `[dx, dy, dz]` at `field`, in `units`.
fn distance(value: &Value, units: Unit, field: &str) -> Result<[f64; 3], KeyError> {
    match value {
        Value::Array(items) if items.len() == 3 => {
            let mut by = [0.0; 3];
            for (axis, item) in items.iter().enumerate() {
                by[axis] = length(item, units).map_err(|message| {
                    (ErrorCode::BadNumber, message, format!("{field}[{axis}]"))
                })?;
            }
            Ok(by)
        }
        _ => Err((
            ErrorCode::WrongType,
            "expected how far, three lengths [dx, dy, dz]".to_owned(),
            field.to_owned(),
        )),
    }
}

/// Where a copy sent as `map` copies its objects to: by `by`, by each of
/// `offsets`, or from `from` to `to`.
fn read_offsets(map: &Map<String, Value>, units: Unit) -> Result<Pattern, KeyError> {
    let given = |key: &str| map.contains_key(key);
    match (given("by"), given("offsets"), given("from"), given("to")) {
        (true, false, false, false) => Ok(Pattern::Offsets(vec![Motion::By(distance(
            &map["by"], units, "by",
        )?)])),
        (false, true, false, false) => match &map["offsets"] {
            Value::Array(items) if !items.is_empty() => Ok(Pattern::Offsets(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        distance(item, units, &format!("offsets[{i}]")).map(Motion::By)
                    })
                    .collect::<Result<_, _>>()?,
            )),
            Value::Array(_) => Err((
                ErrorCode::BadValue,
                "offsets lists no distance, so it makes no copy; list at least one".to_owned(),
                "offsets".to_owned(),
            )),
            _ => Err((
                ErrorCode::WrongType,
                "expected a list of distances, each [dx, dy, dz]".to_owned(),
                "offsets".to_owned(),
            )),
        },
        (false, false, true, true) => Ok(Pattern::Offsets(vec![Motion::Between {
            from: Box::new(op_point(&map["from"], units, "from")?),
            to: Box::new(op_point(&map["to"], units, "to")?),
        }])),
        (false, false, false, false) => Err((
            ErrorCode::MissingKey,
            "missing key `by`, how far to copy, [dx, dy, dz]; or give offsets, a list of them, \
             or from, a base point, and to, where it lands"
                .to_owned(),
            "by".to_owned(),
        )),
        (false, false, true, false) | (false, false, false, true) => {
            let missing = if given("from") { "to" } else { "from" };
            Err((
                ErrorCode::MissingKey,
                format!(
                    "missing key `{missing}`: a copy from a base point to a second point takes \
                     both, from and to"
                ),
                missing.to_owned(),
            ))
        }
        _ => Err((
            ErrorCode::Usage,
            "give one of by, offsets, or from and to: a copy is by a distance, by each of \
             several, or from a base point to a second point"
                .to_owned(),
            String::new(),
        )),
    }
}

/// How many items an array's `count` holds, the objects among them.
const COUNT_WORDS: &str = "the count is the objects and their copies";

/// An array sent as `map`, with the operation's name as sent, `sent`,
/// which gives its kind when it is ARRAYRECT, ARRAYPOLAR or ARRAYPATH.
fn read_array(map: &Map<String, Value>, sent: &str, units: Unit) -> Result<Pattern, KeyError> {
    let implied = ARRAY_ALIASES
        .iter()
        .position(|a| a.eq_ignore_ascii_case(sent));
    let given = match map.get("kind") {
        None => None,
        Some(Value::String(kind)) => match ARRAY_KINDS.iter().position(|k| k == kind) {
            Some(i) => Some(i),
            None => {
                return Err((
                    ErrorCode::BadValue,
                    format!(
                        "there is no kind of array `{kind}`; the kinds: {}",
                        ARRAY_KINDS.join(", ")
                    ),
                    "kind".to_owned(),
                ))
            }
        },
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                format!(
                    "expected the kind of array, one of {}",
                    ARRAY_KINDS.join(", ")
                ),
                "kind".to_owned(),
            ))
        }
    };
    let kind = match (implied, given) {
        (Some(i), Some(g)) if i != g => {
            return Err((
                ErrorCode::Usage,
                format!(
                    "sent as {sent}, the operation makes a {} array, and kind is {}; leave kind \
                     out, or send it as array",
                    ARRAY_KINDS[i], ARRAY_KINDS[g]
                ),
                "kind".to_owned(),
            ))
        }
        (Some(i), _) | (None, Some(i)) => i,
        (None, None) => {
            return Err((
                ErrorCode::MissingKey,
                format!(
                    "missing key `kind`, the kind of array: {}; or send the operation as {}",
                    ARRAY_KINDS.join(", "),
                    ARRAY_ALIASES.join(", ")
                ),
                "kind".to_owned(),
            ))
        }
    };
    let name = ARRAY_KINDS[kind];
    let own = ARRAY_KEYS[kind];
    if let Some(key) = map.keys().find(|k| {
        !matches!(k.as_str(), OP_KEY | "target" | "targets" | "kind") && !own.contains(&k.as_str())
    }) {
        return Err((
            ErrorCode::UnknownKey,
            format!(
                "unknown key `{key}`: {} {name} array takes target or targets, kind, {}",
                article(name),
                own.join(", ")
            ),
            key.clone(),
        ));
    }
    let count = |key: &str, least: usize, what: &str| -> Result<Option<usize>, KeyError> {
        let Some(value) = map.get(key) else {
            return Ok(None);
        };
        let n = match value {
            Value::Number(n) => whole(n),
            _ => None,
        };
        let most = (MOST_COPIES + 1) as u64;
        match n {
            Some(n) if n >= least as u64 && n <= most => Ok(Some(n as usize)),
            Some(n) if n > most => Err((
                ErrorCode::BadValue,
                format!("{key} is {n}; one array makes at most {MOST_COPIES} copies"),
                key.to_owned(),
            )),
            Some(n) => Err((
                ErrorCode::BadValue,
                format!("{key} is {n}; {what}, so it is at least {least}"),
                key.to_owned(),
            )),
            None => Err((
                ErrorCode::WrongType,
                format!("expected a whole number for {key}"),
                key.to_owned(),
            )),
        }
    };
    let flag = |key: &str| -> Result<bool, KeyError> {
        match map.get(key) {
            None => Ok(true),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err((
                ErrorCode::WrongType,
                format!("expected true or false for {key}"),
                key.to_owned(),
            )),
        }
    };
    let a_length = |key: &str| -> Result<Option<f64>, KeyError> {
        map.get(key)
            .map(|v| length(v, units).map_err(|m| (ErrorCode::BadNumber, m, key.to_owned())))
            .transpose()
    };
    let items = |key: &str| -> Result<usize, KeyError> {
        count(key, 2, COUNT_WORDS)?.ok_or_else(|| {
            (
                ErrorCode::MissingKey,
                "missing key `count`, how many items the array holds, the objects among them"
                    .to_owned(),
                key.to_owned(),
            )
        })
    };
    match name {
        "rectangular" => {
            if !map.contains_key("rows") && !map.contains_key("columns") {
                return Err((
                    ErrorCode::MissingKey,
                    "missing key `rows`: a rectangular array takes rows, columns or both, each \
                     1 when left out"
                        .to_owned(),
                    "rows".to_owned(),
                ));
            }
            let rows = count("rows", 1, "the rows count the objects' own row")?.unwrap_or(1);
            let columns =
                count("columns", 1, "the columns count the objects' own column")?.unwrap_or(1);
            if rows.saturating_mul(columns) < 2 {
                return Err((
                    ErrorCode::BadValue,
                    "1 row and 1 column are the objects alone, which makes no copy; give more \
                     rows or columns"
                        .to_owned(),
                    if map.contains_key("rows") {
                        "rows"
                    } else {
                        "columns"
                    }
                    .to_owned(),
                ));
            }
            if rows.saturating_mul(columns) > MOST_COPIES + 1 {
                return Err((
                    ErrorCode::BadValue,
                    format!(
                        "{rows} rows and {columns} columns would make {} copies; one array \
                         makes at most {MOST_COPIES}",
                        rows.saturating_mul(columns) - 1
                    ),
                    "rows".to_owned(),
                ));
            }
            let spacing = |key: &str, needed: bool, what: &str| -> Result<f64, KeyError> {
                match a_length(key)? {
                    Some(d) => Ok(d),
                    None if needed => Err((
                        ErrorCode::MissingKey,
                        format!("missing key `{key}`, how far apart the {what} are"),
                        key.to_owned(),
                    )),
                    None => Ok(0.0),
                }
            };
            Ok(Pattern::Rectangular {
                rows,
                columns,
                row_spacing: spacing("row_spacing", rows > 1, "rows")?,
                column_spacing: spacing("column_spacing", columns > 1, "columns")?,
            })
        }
        "polar" => {
            let center = match map.get("center") {
                Some(value) => Box::new(op_point(value, units, "center")?),
                None => {
                    return Err((
                        ErrorCode::MissingKey,
                        "missing key `center`, the point a polar array goes round".to_owned(),
                        "center".to_owned(),
                    ))
                }
            };
            let count = items("count")?;
            let angle = match map.get("angle") {
                None => 360.0,
                Some(value) => {
                    let degrees = match value {
                        Value::Number(n) => n.as_f64(),
                        _ => None,
                    };
                    match degrees {
                        Some(d) if d.is_finite() && d != 0.0 && d.abs() <= 360.0 => d,
                        Some(d) => {
                            return Err((
                                ErrorCode::BadValue,
                                format!(
                                    "angle is {d}; the angle a polar array fills is more than 0 \
                                     and at most 360 degrees, either way round"
                                ),
                                "angle".to_owned(),
                            ))
                        }
                        None => {
                            return Err((
                                ErrorCode::WrongType,
                                "expected an angle in degrees, a number such as 180".to_owned(),
                                "angle".to_owned(),
                            ))
                        }
                    }
                }
            };
            let rotate = flag("rotate")?;
            let base_point = match map.get("base_point") {
                None => None,
                Some(_) if rotate => {
                    return Err((
                        ErrorCode::Usage,
                        "base_point places copies that are not turned; with rotate true, or \
                         left out, each copy turns about the centre, so give rotate: false with \
                         it, or leave it out"
                            .to_owned(),
                        "base_point".to_owned(),
                    ))
                }
                Some(value) => Some(Box::new(op_point(value, units, "base_point")?)),
            };
            Ok(Pattern::Polar {
                center,
                count,
                angle,
                rotate,
                base_point,
            })
        }
        _ => {
            let path =
                match map.get("path") {
                    Some(value) => read_target(value).map_err(|e| target_at("path", e))?,
                    None => return Err((
                        ErrorCode::MissingKey,
                        "missing key `path`, the line, polyline, arc, circle, ellipse or spline \
                         the array runs along, by its ID or its name"
                            .to_owned(),
                        "path".to_owned(),
                    )),
                };
            let count = items("count")?;
            let spacing = a_length("spacing")?;
            if let Some(d) = spacing.filter(|d| *d <= 0.0) {
                return Err((
                    ErrorCode::BadValue,
                    format!("spacing is {d}; the items of a path array are more than 0 apart"),
                    "spacing".to_owned(),
                ));
            }
            Ok(Pattern::Path {
                path,
                count,
                spacing,
                align: flag("align")?,
            })
        }
    }
}

/// The required keys an object leaves out: the first one missing, and what
/// the object needs, in words.
struct Missing {
    first: String,
    needs: String,
}

/// What `keys` leaves out of what the object's schema `object` requires:
/// its required keys, and one of the sets of keys it offers a choice of,
/// such as a spline's `fit_points` or `control_points`.
fn missing_required(object: &Value, keys: &Map<String, Value>) -> Option<Missing> {
    let names = |list: Option<&Value>| -> Vec<String> {
        list.and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let required = names(object.get("required"));
    if let Some(first) = required.iter().find(|k| !keys.contains_key(*k)) {
        return Some(Missing {
            first: first.clone(),
            needs: required.join(", "),
        });
    }
    let choices: Vec<Vec<String>> = object
        .get("oneOf")
        .and_then(Value::as_array)
        .map(|options| options.iter().map(|o| names(o.get("required"))).collect())
        .unwrap_or_default();
    let chosen = choices
        .iter()
        .any(|choice| choice.iter().all(|k| keys.contains_key(k)));
    if choices.is_empty() || chosen {
        return None;
    }
    let first = choices
        .iter()
        .flat_map(|choice| choice.iter().find(|k| !keys.contains_key(*k)))
        .next()?
        .clone();
    let needs = choices
        .iter()
        .map(|choice| choice.join(" and "))
        .collect::<Vec<_>>()
        .join(" or ");
    Some(Missing { first, needs })
}

/// Reads a target: `{"id": "SHEET/ID"}` or `{"name": ..., "sheet": ...}`.
fn read_target(value: &Value) -> Result<Target, (ErrorCode, String, String)> {
    let Value::Object(map) = value else {
        return Err((
            ErrorCode::WrongType,
            "expected a target, {\"id\": \"DEMO-001/border\"} or {\"name\": \"title-block\", \
             \"sheet\": \"DEMO-001\"}"
                .to_owned(),
            "target".to_owned(),
        ));
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "name" | "sheet"))
    {
        return Err((
            ErrorCode::UnknownKey,
            format!("unknown key `{key}`; a target takes id, or name and sheet"),
            format!("target.{key}"),
        ));
    }
    let text = |key: &str| match map.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err((
            ErrorCode::WrongType,
            format!("expected text under {key}"),
            format!("target.{key}"),
        )),
    };
    match (text("id")?, text("name")?, text("sheet")?) {
        (Some(id), None, None) => match id.split_once('/') {
            Some((sheet, rest)) if !sheet.is_empty() && !rest.is_empty() => Ok(Target::Id {
                sheet: sheet.to_owned(),
                id: rest.to_owned(),
            }),
            _ => Err((
                ErrorCode::BadValue,
                format!(
                    "`{id}` is not an object's ID: an ID is its sheet's name, `/`, and the id \
                     its record gives it or its place, such as DEMO-001/border"
                ),
                "target.id".to_owned(),
            )),
        },
        (Some(_), None, Some(_)) => Err((
            ErrorCode::Usage,
            "an ID names its sheet already; give id alone, or name and sheet".to_owned(),
            "target".to_owned(),
        )),
        (None, Some(name), sheet) => Ok(Target::Name { name, sheet }),
        (Some(_), Some(_), _) => Err((
            ErrorCode::Usage,
            "a target names its object by its ID or by its name, not both".to_owned(),
            "target".to_owned(),
        )),
        (None, None, _) => Err((
            ErrorCode::MissingKey,
            "a target names its object by its ID, id, or by its name, name".to_owned(),
            "target".to_owned(),
        )),
    }
}

/// A point an operation takes at `field`, as sent: three lengths `[x, y,
/// z]` in `units`, or a snapped point `{"snap": {...}}`; or the code,
/// message and key of what is wrong with it.
fn op_point(
    value: &Value,
    units: Unit,
    field: &str,
) -> Result<OpPoint, (ErrorCode, String, String)> {
    match value {
        Value::Array(xyz) if xyz.len() == 3 => {
            let mut point = [0.0; 3];
            for (axis, v) in xyz.iter().enumerate() {
                point[axis] = length(v, units).map_err(|message| {
                    (ErrorCode::BadNumber, message, format!("{field}[{axis}]"))
                })?;
            }
            Ok(OpPoint::At(point))
        }
        Value::Object(map) => Ok(OpPoint::Snap(PointSnap {
            pointer: String::new(),
            field: field.to_owned(),
            snap: read_snap(map, field)?,
        })),
        _ => Err((
            ErrorCode::WrongType,
            "expected a point, three lengths [x, y, z], or a snapped point, {\"snap\": {...}}"
                .to_owned(),
            field.to_owned(),
        )),
    }
}

/// The snap of a snapped point sent as `map` at `field`, `{"snap": {...}}`,
/// checked as a query checks one, with its snap named; or the code, message
/// and key of what is wrong with it.
pub(crate) fn read_snap(
    map: &Map<String, Value>,
    field: &str,
) -> Result<SnapRef, (ErrorCode, String, String)> {
    if let Some(key) = map.keys().find(|k| *k != "snap") {
        return Err((
            ErrorCode::UnknownKey,
            format!(
                "unknown key `{key}`; a snapped point is {{\"snap\": {{...}}}} alone, the snap \
                 written as query snap takes it"
            ),
            format!("{field}.{key}"),
        ));
    }
    let at = format!("{field}.snap");
    let Some(sent) = map.get("snap") else {
        return Err((
            ErrorCode::WrongType,
            "expected a point, three lengths [x, y, z], or a snapped point, {\"snap\": {...}}"
                .to_owned(),
            field.to_owned(),
        ));
    };
    let snap: SnapRef = serde_json::from_value(sent.clone()).map_err(|e| {
        (
            ErrorCode::WrongType,
            format!(
                "expected a snap, as query snap takes it: an object by id or name, its snap, \
                 and which, with or from where the snap takes them ({e})"
            ),
            at.clone(),
        )
    })?;
    let into = |e: Error| (e.code, e.message, e.field.unwrap_or_default());
    let mut each = Some((&snap, at));
    while let Some((snap, at)) = each {
        if snap.snap.is_none() {
            return Err(into(no_kind(&at)));
        }
        check_shape(snap, &at).map_err(into)?;
        each = snap
            .from
            .as_deref()
            .map(|from| (from, format!("{at}.from")));
    }
    Ok(snap)
}

/// Adds to `out` each snapped point `value` holds where one of `slots`, the
/// schemas that may take it, takes a point. `pointer` and `field` are its
/// place in an add's keys, as a JSON pointer and as a field. A snapped
/// point where a direction is taken, such as an ellipse's `major_axis`, is
/// an error; a mapping anywhere else is the object's own, whatever its
/// keys, such as an insert's attributes with a tag `SNAP`.
fn snaps_in(
    value: &Value,
    slots: &[&Value],
    defs: &Map<String, Value>,
    pointer: &str,
    field: &str,
    out: &mut Vec<PointSnap>,
) -> Result<(), (ErrorCode, String, String)> {
    let mut schemas: Vec<&Value> = Vec::new();
    for slot in slots {
        expand(slot, defs, &mut schemas, 0);
    }
    let refers = |to: &str| {
        schemas
            .iter()
            .any(|s| s.get("$ref").and_then(Value::as_str) == Some(to))
    };
    // Where a point is taken that may be snapped, and where a point is
    // taken that may not, a direction.
    let (takes_snap, takes_point) = (refers("#/$defs/snapped_point"), refers("#/$defs/point"));
    match value {
        Value::Object(map) if map.contains_key("snap") && takes_snap => {
            out.push(PointSnap {
                pointer: pointer.to_owned(),
                field: field.to_owned(),
                snap: read_snap(map, field)?,
            });
        }
        Value::Object(map) if map.contains_key("snap") && takes_point => {
            return Err((
                ErrorCode::BadValue,
                format!(
                    "`{field}` takes no snapped point: it is a direction, [dx, dy, dz], and a \
                     snapped point is taken where a point is"
                ),
                field.to_owned(),
            ));
        }
        Value::Object(map) => {
            for (key, item) in map {
                let inner: Vec<&Value> = schemas
                    .iter()
                    .filter_map(|s| s.get("properties").and_then(|p| p.get(key)))
                    .collect();
                snaps_in(
                    item,
                    &inner,
                    defs,
                    &format!("{pointer}/{key}"),
                    &format!("{field}.{key}"),
                    out,
                )?;
            }
        }
        Value::Array(items) => {
            let inner: Vec<&Value> = schemas.iter().filter_map(|s| s.get("items")).collect();
            for (i, item) in items.iter().enumerate() {
                snaps_in(
                    item,
                    &inner,
                    defs,
                    &format!("{pointer}/{i}"),
                    &format!("{field}[{i}]"),
                    out,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Adds `schema` to `out`, with every schema it stands for: the definition
/// it refers to, but a snapped point's, and each of its `allOf`, `oneOf`
/// and `anyOf`.
fn expand<'a>(
    schema: &'a Value,
    defs: &'a Map<String, Value>,
    out: &mut Vec<&'a Value>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    out.push(schema);
    if let Some(name) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
    {
        if name != "snapped_point" {
            if let Some(def) = defs.get(name) {
                expand(def, defs, out, depth + 1);
            }
        }
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(Value::Array(branches)) = schema.get(key) {
            for branch in branches {
                expand(branch, defs, out, depth + 1);
            }
        }
    }
}

/// A length sent as a number or as text with its own unit, in `units`.
fn length(value: &Value, units: Unit) -> Result<f64, String> {
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err("expected a length such as 1 or \"4 in\"".to_owned()),
    };
    match parse_length(&text) {
        Ok((value, None)) => Ok(value),
        Ok((value, Some(own))) => {
            let converted = own.convert(value, units);
            if converted.is_finite() {
                Ok(converted)
            } else {
                Err(format!("`{text}` is too large once converted to {units}"))
            }
        }
        Err(LengthError::UnknownUnit(unit)) => Err(format!(
            "unknown unit `{unit}`; known units: {}",
            Unit::symbols()
        )),
        Err(LengthError::OutOfRange) => Err(format!("`{text}` is too large")),
        Err(LengthError::NotANumber) => Err(format!(
            "expected a length such as 1 or \"4 in\", found `{text}`"
        )),
    }
}

/// `n` as a whole number from 0, whether JSON writes it `2` or `2.0`.
fn whole(n: &serde_json::Number) -> Option<u64> {
    n.as_u64().or_else(|| {
        n.as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64)
            .map(|f| f as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::object_types;

    #[test]
    fn every_object_a_sheet_holds_has_an_operation_that_adds_it() {
        let mut added: Vec<&str> = SPECS
            .iter()
            .filter_map(|s| match s.kind {
                Kind::Add(object, _) => Some(object),
                _ => None,
            })
            .collect();
        // A dimension is added by one operation of each kind.
        added.dedup();
        assert_eq!(added, object_types());
    }

    #[test]
    fn every_operation_has_its_own_name_and_alias() {
        for (i, a) in SPECS.iter().enumerate() {
            for b in &SPECS[i + 1..] {
                assert_ne!(a.name, b.name);
                // An operation with no AutoCAD alias has none to share.
                assert!(a.alias.is_empty() || !a.alias.eq_ignore_ascii_case(b.alias));
                for other in a.other_aliases() {
                    assert!(!b.named(other), "{other} names {} and {}", a.name, b.name);
                }
            }
        }
    }

    #[test]
    fn every_reference_in_a_listed_schema_resolves_in_it() {
        fn walk(value: &Value, defs: &Value) {
            refs(value, &mut |name| {
                assert!(defs.get(name).is_some(), "{name} resolves")
            });
        }
        for info in operations().operations {
            walk(&info.schema, &info.schema["$defs"]);
        }
        let schema = change_set_schema();
        walk(&schema, &schema["$defs"]);
    }
}
