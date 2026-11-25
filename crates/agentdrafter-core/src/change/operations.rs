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
use crate::model::{
    BooleanOp, DimensionKind, Point3, ISO_DIMENSION_STYLE, STANDARD_DIMENSION_STYLE,
};
use crate::query::snap::{check_shape, no_kind};
use crate::query::Selection;
use crate::units::{parse_length, LengthError, Unit};

use super::break_join::DEFAULT_JOIN_TOLERANCE;
use super::fillet_chamfer::{ChamferSpec, CornerEnd};
use super::replace::{TextMatchKind, TextMatcher, TextScope};
use super::trim_extend::{ExtendEnd, TrimKeep};

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
    /// Changes the draw order of objects: AutoCAD's DRAWORDER.
    DrawOrder,
    /// Brings text, dimensions, leaders, or all of them to the front:
    /// AutoCAD's TEXTTOFRONT.
    TextToFront,
    SetAttribute,
    /// Sets dynamic block parameter values on an insert: AutoCAD's PROPERTIES.
    SetBlockState,
    /// Sets an attribute on every insert a query selection matches: AutoCAD's
    /// GATTE.
    SetAttributes,
    /// Finds and replaces text across texts, mtexts and attribute values:
    /// AutoCAD's FIND.
    ReplaceText,
    /// Draws a parallel offset of a line, polyline, arc, circle, ellipse or
    /// spline: AutoCAD's OFFSET.
    Offset,
    /// Cuts an object back to one or more boundary objects: AutoCAD's TRIM.
    Trim,
    /// Carries an open object on to one or more boundary objects: AutoCAD's
    /// EXTEND.
    Extend,
    /// Rounds the corner between two objects or every corner of a polyline
    /// with an arc of a radius (or radius 0 for a sharp corner): AutoCAD's
    /// FILLET.
    Fillet,
    /// Bevels the corner between two lines or every corner of a polyline by
    /// distances or length and angle: AutoCAD's CHAMFER.
    Chamfer,
    /// Cuts a gap in an object or splits it at a point: AutoCAD's BREAK /
    /// BREAKATPOINT.
    Break,
    /// Joins lines, arcs and open polylines end to end (or collinear lines /
    /// co-circular arcs) into one object: AutoCAD's JOIN.
    Join,
    /// Moves vertices and ends of objects lying inside a box while leaving the
    /// rest where they are: AutoCAD's STRETCH.
    Stretch,
    /// Turns a compound object (insert, polyline, donut, revision cloud,
    /// hatch, dimension, multileader, leader or table) into the simpler objects
    /// that draw it: AutoCAD's EXPLODE.
    Explode,
    /// Attaches, updates, or clears an AutoCAD field on a text, mtext, or
    /// insert attribute: AutoCAD's FIELD.
    SetField,
    /// Sets a sheet's or layout's page setup (`PAGESETUP`): paper, plot area,
    /// plot scale, centring, offset, plot style table (CTB or STB) and
    /// transparency plotting.
    SetPageSetup,
    /// Attaches or overlays an external reference (DWG/DXF) and places an
    /// insert of it: AutoCAD's XATTACH / XREF.
    AttachXref,
    /// Detaches an external reference and removes its inserts.
    DetachXref,
    /// Binds an external reference into a local block definition: AutoCAD's
    /// XBIND.
    BindXref,
    /// Creates a named group of objects: AutoCAD's GROUP (G).
    AddGroup,
    /// Removes a named group without deleting its member objects: AutoCAD's
    /// UNGROUP.
    Ungroup,
    /// Edits a named group (add/remove members, rename, description,
    /// selectable): AutoCAD's GROUPEDIT.
    EditGroup,
    /// Sets the active annotation scale (`CANNOSCALE`), `ANNOALLVISIBLE`, or
    /// edits the drawing's scale list (`SCALELISTEDIT`).
    SetAnnotationScale,
    /// Adds, removes, or replaces supported annotation scales on annotative
    /// objects, or toggles their `annotative` flag (`OBJECTSCALE` / `ANNOUPDATE`).
    SetObjectScales,
    /// Sets drawing-wide point display style (`PDMODE` and/or `PDSIZE`):
    /// AutoCAD's DDPTYPE / PTYPE.
    SetPointStyle,
    /// Adds or updates a named multiline style (`mline_styles` / `ACAD_MLINESTYLE`):
    /// AutoCAD's MLSTYLE.
    AddMLineStyle,
    /// Adds a 3D box solid (`solid3d` with `kind: box`): AutoCAD's BOX.
    AddBox,
    /// Adds a 3D cylinder solid (`solid3d` with `kind: cylinder`): AutoCAD's CYLINDER.
    AddCylinder,
    /// Extrudes a closed 2D profile (closed polyline, circle, or whole ellipse) into a
    /// 3D solid: AutoCAD's EXTRUDE (EXT).
    Extrude,
    /// Revolves a closed 2D profile around a 3D axis into a 3D solid: AutoCAD's REVOLVE (REV).
    Revolve,
    /// Combines 3D solids by CSG boolean (`UNION` / `SUBTRACT` / `INTERSECT`).
    SolidBoolean(BooleanOp),
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
            Kind::DrawOrder => &["DR"],
            Kind::Offset => &["O"],
            Kind::Trim => &["TR"],
            Kind::Extend => &["EX"],
            Kind::Fillet => &["F"],
            Kind::Chamfer => &["CHA"],
            Kind::Break => &["BR", "BREAKATPOINT"],
            Kind::Join => &["J"],
            Kind::Stretch => &["S"],
            Kind::Explode => &["X"],
            Kind::AttachXref => &["XREF"],
            Kind::AddGroup => &["G"],
            Kind::SetAnnotationScale => &["SCALELISTEDIT"],
            Kind::SetObjectScales => &["ANNOUPDATE"],
            Kind::SetPointStyle => &["PTYPE", "PDMODE", "PDSIZE"],
            Kind::Add("image", None) => &["IMAGE"],
            Kind::Add("pdf_underlay", None) => &["PDFUNDERLAY"],
            Kind::Add("point", None) => &["PO"],
            Kind::Add("xline", None) => &["XL"],
            Kind::Add("mline", None) => &["ML"],
            Kind::Add("tolerance", None) => &["TOL"],
            Kind::Extrude => &["EXT"],
            Kind::Revolve => &["REV"],
            Kind::SolidBoolean(BooleanOp::Union) => &["UNI"],
            Kind::SolidBoolean(BooleanOp::Subtract) => &["SU"],
            Kind::SolidBoolean(BooleanOp::Intersect) => &["IN"],
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
pub(crate) const SPECS: [Spec; 110] = [
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
    add("add_revision_cloud", "REVCLOUD", "revision_cloud"),
    add("add_wipeout", "WIPEOUT", "wipeout"),
    add("add_image", "IMAGEATTACH", "image"),
    add("add_pdf_underlay", "PDFATTACH", "pdf_underlay"),
    add("add_point", "POINT", "point"),
    add("add_xline", "XLINE", "xline"),
    add("add_ray", "RAY", "ray"),
    add("add_mline", "MLINE", "mline"),
    add("add_tolerance", "TOLERANCE", "tolerance"),
    add("add_center_mark", "CENTERMARK", "center_mark"),
    add("add_center_line", "CENTERLINE", "center_line"),
    add("add_shape", "SHAPE", "shape"),
    add("add_solid3d", "3DSOLID", "solid3d"),
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
        name: "draw_order",
        alias: "DRAWORDER",
        kind: Kind::DrawOrder,
    },
    Spec {
        name: "text_to_front",
        alias: "TEXTTOFRONT",
        kind: Kind::TextToFront,
    },
    Spec {
        name: "set_attribute",
        alias: "ATTEDIT",
        kind: Kind::SetAttribute,
    },
    Spec {
        name: "set_block_state",
        alias: "PROPERTIES",
        kind: Kind::SetBlockState,
    },
    Spec {
        name: "set_attributes",
        alias: "GATTE",
        kind: Kind::SetAttributes,
    },
    Spec {
        name: "replace_text",
        alias: "FIND",
        kind: Kind::ReplaceText,
    },
    Spec {
        name: "offset",
        alias: "OFFSET",
        kind: Kind::Offset,
    },
    Spec {
        name: "trim",
        alias: "TRIM",
        kind: Kind::Trim,
    },
    Spec {
        name: "extend",
        alias: "EXTEND",
        kind: Kind::Extend,
    },
    Spec {
        name: "fillet",
        alias: "FILLET",
        kind: Kind::Fillet,
    },
    Spec {
        name: "chamfer",
        alias: "CHAMFER",
        kind: Kind::Chamfer,
    },
    Spec {
        name: "break",
        alias: "BREAK",
        kind: Kind::Break,
    },
    Spec {
        name: "join",
        alias: "JOIN",
        kind: Kind::Join,
    },
    Spec {
        name: "stretch",
        alias: "STRETCH",
        kind: Kind::Stretch,
    },
    Spec {
        name: "explode",
        alias: "EXPLODE",
        kind: Kind::Explode,
    },
    Spec {
        name: "set_field",
        alias: "FIELD",
        kind: Kind::SetField,
    },
    Spec {
        name: "set_page_setup",
        alias: "PAGESETUP",
        kind: Kind::SetPageSetup,
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
    Spec {
        name: "attach_xref",
        alias: "XATTACH",
        kind: Kind::AttachXref,
    },
    Spec {
        name: "detach_xref",
        alias: "XDETACH",
        kind: Kind::DetachXref,
    },
    Spec {
        name: "bind_xref",
        alias: "XBIND",
        kind: Kind::BindXref,
    },
    Spec {
        name: "add_group",
        alias: "GROUP",
        kind: Kind::AddGroup,
    },
    Spec {
        name: "ungroup",
        alias: "UNGROUP",
        kind: Kind::Ungroup,
    },
    Spec {
        name: "edit_group",
        alias: "GROUPEDIT",
        kind: Kind::EditGroup,
    },
    Spec {
        name: "set_annotation_scale",
        alias: "CANNOSCALE",
        kind: Kind::SetAnnotationScale,
    },
    Spec {
        name: "set_object_scales",
        alias: "OBJECTSCALE",
        kind: Kind::SetObjectScales,
    },
    Spec {
        name: "set_point_style",
        alias: "DDPTYPE",
        kind: Kind::SetPointStyle,
    },
    Spec {
        name: "add_mline_style",
        alias: "MLSTYLE",
        kind: Kind::AddMLineStyle,
    },
    Spec {
        name: "add_box",
        alias: "BOX",
        kind: Kind::AddBox,
    },
    Spec {
        name: "add_cylinder",
        alias: "CYLINDER",
        kind: Kind::AddCylinder,
    },
    Spec {
        name: "extrude",
        alias: "EXTRUDE",
        kind: Kind::Extrude,
    },
    Spec {
        name: "revolve",
        alias: "REVOLVE",
        kind: Kind::Revolve,
    },
    Spec {
        name: "union",
        alias: "UNION",
        kind: Kind::SolidBoolean(BooleanOp::Union),
    },
    Spec {
        name: "subtract",
        alias: "SUBTRACT",
        kind: Kind::SolidBoolean(BooleanOp::Subtract),
    },
    Spec {
        name: "intersect",
        alias: "INTERSECT",
        kind: Kind::SolidBoolean(BooleanOp::Intersect),
    },
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
                schema: with_definitions(operation_schema(spec, &defs, false), &defs),
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
        Kind::Add("revision_cloud", None) => {
            "Adds a revision cloud round a rectangle, a polygon, or an existing object's (or \
             several objects') extents with `around` and an optional `margin`, with an \
             `arc_length` and a `style` (`normal` or `calligraphy`), written as a closed \
             lightweight polyline with AutoCAD's RevcloudProps extended data so AutoCAD's \
             REVCLOUD can modify it. With `rectangle` or `polygon`, give `sheet` or `template`; \
             with `around`, `sheet` and `template` may be omitted to place the cloud on the \
             target's sheet or template, and `arc_length` defaults to a size proportional to the \
             framed extents. Without an id it is given one, revision_cloud-1 or the next number \
             free. AutoCAD's REVCLOUD."
                .to_owned()
        }
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
        Kind::DrawOrder => "Changes the draw order of target, one object, or targets, several of \
                            one sheet or template, putting them at order: front (drawn last, on \
                            top), back (drawn first, behind), {\"above\": target}, or {\"under\": \
                            target}. Each object keeps its ID; any un-named object whose place \
                            shifts is given its previous place as its id so its ID never moves. \
                            On an original DWG, updates the space's ACAD_SORTENTS table. \
                            AutoCAD's DRAWORDER (DR)."
            .to_owned(),
        Kind::TextToFront => "Brings annotations to the front of draw order so they draw on top \
                              of geometry and wipeouts: which may be text (text and mtext), \
                              dimensions, leaders (mleader and leader), or all (all of them, the \
                              default when omitted), on sheet or template (or the drawing itself \
                              when revising an original DWG). Each object keeps its ID. \
                              AutoCAD's TEXTTOFRONT."
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
        Kind::SetBlockState => "Sets one or more dynamic block parameter values (visibility \
                                state, lookup row, flip, linear distance, or rotation) on an \
                                insert, named by each parameter's name as AutoCAD's Properties \
                                palette lists it. A null value resets a parameter to its default. \
                                AutoCAD's PROPERTIES."
            .to_owned(),
        Kind::SetAttributes => "Sets an attribute's value on every insert that matches the query \
                                selection under `where`, across the sheets named under `sheets` \
                                or the whole drawing set when `sheets` is left out, listing every \
                                insert updated in the reply. Uses the same selection keys as \
                                `query attributes` (block, name, id, tag, value, text, match, \
                                space, window, crossing, touching), takes `occurrence` for a \
                                repeated tag and `row_value` for a template list of parts. \
                                AutoCAD's GATTE."
            .to_owned(),
        Kind::ReplaceText => "Finds and replaces text in texts, multiline texts and insert \
                              attribute values across the sheets named under `sheets` or the \
                              whole drawing set when `sheets` is left out, matched as `exact` \
                              literal text (the default), `whole_word`, or a regular expression \
                              `pattern` (with capture-group substitution), case-sensitively by \
                              default (`case_sensitive: false` to ignore case). Optionally \
                              restricts the search with `in` (`text`, `mtext`, `attribute`), \
                              `tag` or `block`. Lists every place changed in the reply, and a \
                              dry run shows them without writing. AutoCAD's FIND."
            .to_owned(),
        Kind::Offset => "Draws a parallel offset of a line, polyline (straight or with arc \
                         segments, corners joined as AutoCAD's OFFSETGAPTYPE 0 joins them), arc, \
                         circle, ellipse or spline named by `target`, either at `distance` on \
                         `side` (`left`, `right`, `inside`, `outside`, or a point / snapped \
                         point) or passing through `through` (a point / snapped point). Written \
                         in the same record and space as `target`, keeping its properties (with \
                         optional `layer`, `id`, `name`, and `erase_source`). AutoCAD's OFFSET \
                         (O)."
            .to_owned(),
        Kind::Trim => "Cuts `target` (a line, polyline, arc, circle, ellipse or spline) back to \
                       one or more boundary objects (`boundary` or `boundaries`, including block \
                       inserts), keeping the part named by `keep` (`start`, `end`, `between`, or \
                       `ends`). `target` keeps its ID; `keep: \"ends\"` on an open curve splits \
                       it in two, giving the second piece an ID (`new_id` or assigned) reported \
                       under `split_id`. AutoCAD's TRIM (TR)."
            .to_owned(),
        Kind::Extend => "Carries an open `target` (a line, open polyline, arc, elliptical arc or \
                         open spline) on at its `end` (`start`, `end`, or `both`) until it meets \
                         the nearest boundary object (`boundary` or `boundaries`, including block \
                         inserts). `target` keeps its ID. AutoCAD's EXTEND (EX)."
            .to_owned(),
        Kind::Fillet => "Rounds the corner between two objects (`first` and `second`: lines, \
                         arcs or circles) or every straight corner of a flat polyline (`target`) \
                         with an arc of `radius` (or `radius: 0` to square two lines to a sharp \
                         corner or square up arc corners on a polyline). Supports `trim` (true by \
                         default), `first_end`/`second_end` (`start`/`end`), `new_id` and `layer` \
                         in two-object mode. AutoCAD's FILLET (F)."
            .to_owned(),
        Kind::Chamfer => "Bevels the corner between two lines (`first` and `second`) or every \
                          straight corner of a flat polyline (`target`) by `distance` (equal \
                          cutbacks), `distance1` and `distance2`, or `length` and `angle` (in \
                          degrees; zero distances bring two lines to a sharp corner). Supports \
                          `trim` (true by default), `first_end`/`second_end` (`start`/`end`), \
                          `new_id` and `layer` in two-object mode. AutoCAD's CHAMFER (CHA)."
            .to_owned(),
        Kind::Break => "Splits `target` (a line, polyline, arc, circle, ellipse or spline) at a \
                        point `at` (optionally cutting a centered `gap`), or removes the span \
                        between `from` and `to`, each point a coordinate or snapped point. \
                        `target`'s ID carries on on the first piece; when an open curve splits \
                        in two, the second piece's new ID (`new_id` or assigned) is reported \
                        under `split_id`. AutoCAD's BREAK (BR) and BREAKATPOINT."
            .to_owned(),
        Kind::Join => "Joins two or more objects under `targets` (lines, arcs, and open \
                       polylines) into one: collinear lines merge into a line (even across a \
                       gap), co-circular arcs merge into an arc or circle, and end-to-end curves \
                       chain into a polyline (auto-closing if the ends meet). `targets[0]`'s ID \
                       carries on and the absorbed objects' IDs are reported under `deleted`. \
                       AutoCAD's JOIN (J)."
            .to_owned(),
        Kind::Stretch => "Moves endpoints, vertices and rigid objects lying inside `box` (two \
                          opposite corners, each `[x, y]`, `[x, y, z]`, or a snapped point on a \
                          named object) by `by` or from `from` to `to`, leaving vertices outside \
                          `box` fixed so connected wires and enclosures follow. Skips objects on \
                          locked layers (unless named under optional `targets`) and lists every \
                          touched object ID under `transformed`. AutoCAD's STRETCH (S)."
            .to_owned(),
        Kind::Explode => "Explodes one or more compound objects (`target` or `targets`) — an \
                          insert into its evaluated block's objects and visible attributes as \
                          text, a polyline, donut or revision cloud into lines and arcs, and a \
                          hatch, dimension, multileader, leader, table, multiline or centre mark \
                          into the objects that draw it. Removes the source object(s) (`deleted`), \
                          assigns each new object its own ID, and lists them under `created`. \
                          AutoCAD's EXPLODE (X)."
            .to_owned(),
        Kind::SetField => "Attaches, updates, or clears an AutoCAD field (`FIELD`) on a text, \
                           multiline text, or insert attribute (`tag`, with optional `occurrence` \
                           for repeated tags). Pass `field` as a single field item (`kind`: \
                           `filename`, `save_date`, `plot_date`, `create_date`, `date`, \
                           `sheet_number`, `custom_property`, `object_property`, or `code`) or a \
                           multi-field `{template, items}`, or `field: null` to convert the host \
                           back to plain text. AutoCAD's FIELD."
            .to_owned(),
        Kind::SetPageSetup => "Sets keys of a template's or layout's page setup (`PAGESETUP`): \
                               `paper`, `area` (`extents`, `window`, or `layout`), `window`, \
                               `scale` (`fit` or a ratio such as `1:2`), `center`, `offset`, \
                               `plot_style` (CTB or STB file name), and `plot_transparency`. \
                               Each key under `set` takes its new value as a sheet description \
                               writes it, and `null` clears a key back to its default. Name \
                               `layout` to target a specific layout tab (or `Model` on a DWG), or \
                               omit it for the template's top-level page setup (or the first \
                               layout on a DWG). AutoCAD's PAGESETUP."
            .to_owned(),
        Kind::AttachXref => "Attaches or overlays an external reference (DWG or DXF) under \
                             `name` pointing to `path` (`kind`: `attach` or `overlay`, default \
                             `attach`) and places an insert of it at `position` (default \
                             `[0, 0, 0]`) unless `place` is false. AutoCAD's XATTACH / XREF."
            .to_owned(),
        Kind::DetachXref => "Detaches the external reference `name` and deletes every insert \
                             that places it. AutoCAD's XDETACH (XREF Detach)."
            .to_owned(),
        Kind::BindXref => "Binds the external reference `name` into a local block definition \
                           holding the referenced drawing's model-space objects, importing its \
                           dependent linetypes, layers, text styles and nested blocks (`mode`: \
                           `bind` for `xref$0$` prefixing or `insert` for direct merging), and \
                           keeps every insert in place. AutoCAD's XBIND."
            .to_owned(),
        Kind::AddGroup => "Creates a named group `name` holding the objects listed under `targets` \
                           (with optional `description` and `selectable`, default true), written \
                           into the sheet's row or template (`groups:`) or into `ACAD_GROUP` on an \
                           original DWG, so operations can target the group as one by name and \
                           AutoCAD selects it as one. AutoCAD's GROUP (G)."
            .to_owned(),
        Kind::Ungroup => "Removes the named group `name` without deleting any of its member \
                          objects. AutoCAD's UNGROUP."
            .to_owned(),
        Kind::EditGroup => "Edits the named group `name`: adds objects (`add`), removes objects \
                            (`remove`), renames it (`rename`), or updates its `description` or \
                            `selectable` flag. AutoCAD's GROUPEDIT."
            .to_owned(),
        Kind::SetAnnotationScale => "Sets the active annotation scale (`scale`, AutoCAD's \
                                     CANNOSCALE), toggles `anno_all_visible` (ANNOALLVISIBLE), or \
                                     adds/removes named scales in `annotation_scales` \
                                     (`add_scales`, `remove_scales`, AutoCAD's SCALELISTEDIT) on a \
                                     template or original DWG. AutoCAD's CANNOSCALE / SCALELISTEDIT."
            .to_owned(),
        Kind::SetObjectScales => "Adds (`add`), removes (`remove`), or replaces (`scales`) the \
                                  supported annotation scales on one or more annotative objects \
                                  (`target` or `targets`, including group targets), or toggles \
                                  their `annotative` flag (`text`, `mtext`, `dimension`, \
                                  `mleader`, `hatch`, or `insert`). AutoCAD's OBJECTSCALE / \
                                  ANNOUPDATE."
            .to_owned(),
        Kind::SetPointStyle => "Sets the drawing-wide point display style (`pdmode` / `PDMODE`) \
                                and/or point display size (`pdsize` / `PDSIZE`: positive = drawing \
                                units, 0 = 5% of viewport height, negative = percentage of viewport \
                                height) on a template or original DWG. AutoCAD's DDPTYPE (PTYPE)."
            .to_owned(),
        Kind::AddMLineStyle => "Adds or updates a named multiline style under `mline_styles` \
                                (`ACAD_MLINESTYLE`), with its parallel `elements` (offset, colour, \
                                linetype), optional background `fill_color`, `display_joints`, \
                                `start_angle`, `end_angle`, and end-cap flags. AutoCAD's MLSTYLE."
            .to_owned(),
        Kind::AddBox => "Adds an axis-aligned 3D box solid (`solid3d` with `kind: box`), written \
                         as a native ACIS `3DSOLID` in DWG/DXF: `origin` (minimum XYZ corner, \
                         which may be a snapped point), `length` (X), `width` (Y), and `height` \
                         (Z), to a sheet's row (`sheet`) or a template (`template`). AutoCAD's BOX."
            .to_owned(),
        Kind::AddCylinder => "Adds a 3D circular cylinder solid (`solid3d` with `kind: cylinder`), \
                              written as a native ACIS `3DSOLID` in DWG/DXF: `base` (bottom circle \
                              centre, which may be a snapped point), `radius`, and `height` (+Z), \
                              to a sheet's row (`sheet`) or a template (`template`). AutoCAD's \
                              CYLINDER."
            .to_owned(),
        Kind::Extrude => "Extrudes an existing closed 2D profile (`target`: a closed polyline, \
                          circle, or whole ellipse) by `height` along optional `direction` \
                          (default `[0, 0, 1]`) with optional `taper_angle` (degrees, `-85..85`) \
                          into a native ACIS `3DSOLID` (`solid3d`). Erases the source profile by \
                          default (`erase_source: false` keeps it). AutoCAD's EXTRUDE (EXT)."
            .to_owned(),
        Kind::Revolve => "Revolves an existing closed 2D profile (`target`: a closed polyline, \
                          circle, or whole ellipse) around the 3D axis through `axis_origin` \
                          (point or snapped point) along `axis_direction` by `angle` degrees \
                          (default `360`, `(0, 360]`) into a native ACIS `3DSOLID` (`solid3d`). \
                          Erases the source profile by default (`erase_source: false` keeps it). \
                          AutoCAD's REVOLVE (REV)."
            .to_owned(),
        Kind::SolidBoolean(BooleanOp::Union) => {
            "Combines two or more 3D solids (`targets`) by CSG union into one native ACIS \
             `3DSOLID` (`solid3d`). The first solid's ID carries on (unless `id` overrides it) \
             and the remaining solids are removed. AutoCAD's UNION (UNI)."
                .to_owned()
        }
        Kind::SolidBoolean(BooleanOp::Subtract) => {
            "Subtracts one or more 3D solids (`subtract`) from a base 3D solid (`target`) by CSG \
             difference into one native ACIS `3DSOLID` (`solid3d`). The base solid's ID carries on \
             (unless `id` overrides it) and the subtracted tool solids are removed. AutoCAD's \
             SUBTRACT (SU)."
                .to_owned()
        }
        Kind::SolidBoolean(BooleanOp::Intersect) => {
            "Intersects two or more 3D solids (`targets`) by CSG intersection into one native ACIS \
             `3DSOLID` (`solid3d`) holding their shared volume. The first solid's ID carries on \
             (unless `id` overrides it) and the remaining solids are removed. AutoCAD's INTERSECT \
             (IN)."
                .to_owned()
        }
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
    build_change_set_schema(false)
}

/// JSON Schema of a change set targeting an original `.dwg` file, where
/// `add_*`, `add_layer`, and `change_layer` may omit `sheet` to default to
/// that drawing's sheet (while still forbidding `sheet` and `template`
/// together). Drawing-set commands keep [`change_set_schema`] strict.
pub(crate) fn original_change_set_schema() -> Value {
    build_change_set_schema(true)
}

fn build_change_set_schema(allow_omitted_sheet: bool) -> Value {
    let defs = definitions();
    let choices: Vec<Value> = SPECS
        .iter()
        .map(|spec| operation_schema(spec, &defs, allow_omitted_sheet))
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
            "description": "The object (or group of objects) an operation edits: by its ID, by \
                            its name, or by a group's name (with optional sheet). Never by \
                            coordinates.",
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
                {
                    "type": "object",
                    "required": ["group"],
                    "additionalProperties": false,
                    "properties": {
                        "group": {
                            "description": "The name of a group (`GROUP`), matched in any case, \
                                            targeting all of its member objects as one.",
                            "type": "string",
                            "minLength": 1,
                        },
                        "sheet": { "$ref": "#/$defs/sheet_name" },
                    },
                },
            ],
        }),
    );
    defs.insert(
        "draw_order".into(),
        json!({
            "description": "Where to place the object(s) in draw order: \"front\" (drawn last, \
                            on top), \"back\" (drawn first, behind), {\"above\": target}, or \
                            {\"under\": target}.",
            "oneOf": [
                { "enum": ["front", "back"] },
                {
                    "type": "object",
                    "required": ["above"],
                    "additionalProperties": false,
                    "properties": {
                        "above": {
                            "description": "Place immediately above this object (drawn right after it).",
                            "$ref": "#/$defs/target",
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["under"],
                    "additionalProperties": false,
                    "properties": {
                        "under": {
                            "description": "Place immediately under this object (drawn right before it).",
                            "$ref": "#/$defs/target",
                        },
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
fn operation_schema(spec: &Spec, defs: &Map<String, Value>, allow_omitted_sheet: bool) -> Value {
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
            let def_name = crate::description::schema::object_def_name(object);
            let mut schema = defs
                .get(&format!("op_{object}"))
                .unwrap_or(&defs[def_name])
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
            if object == "revision_cloud" {
                let properties = schema["properties"]
                    .as_object_mut()
                    .expect("an object's definition has properties");
                properties.insert(
                    "around".into(),
                    json!({
                        "description": "One object, or a non-empty list of objects, whose drawn \
                                        extents the cloud frames as a rectangle. Give rectangle, \
                                        polygon or around, not more than one.",
                        "oneOf": [
                            { "$ref": "#/$defs/target" },
                            {
                                "type": "array",
                                "items": { "$ref": "#/$defs/target" },
                                "minItems": 1,
                            },
                        ],
                    }),
                );
                properties.insert(
                    "margin".into(),
                    json!({
                        "$ref": "#/$defs/length_not_negative",
                        "description": "Clearance added on every side of the `around` extents, 0 \
                                        or more. Only given with `around`; defaults to half of \
                                        `arc_length`.",
                    }),
                );
                let own = schema.as_object_mut().expect("an object's definition");
                if let Some(required) = own.get_mut("required").and_then(Value::as_array_mut) {
                    required.retain(|k| k != "arc_length");
                }
                own["oneOf"] = json!([
                    {
                        "required": ["rectangle", "arc_length"],
                        "not": { "anyOf": [{ "required": ["polygon"] }, { "required": ["around"] }, { "required": ["margin"] }] },
                    },
                    {
                        "required": ["polygon", "arc_length"],
                        "not": { "anyOf": [{ "required": ["rectangle"] }, { "required": ["around"] }, { "required": ["margin"] }] },
                    },
                    {
                        "required": ["around"],
                        "not": { "anyOf": [{ "required": ["rectangle"] }, { "required": ["polygon"] }] },
                    },
                ]);
            }
            let properties = schema["properties"]
                .as_object_mut()
                .expect("an object's definition has properties");
            properties.insert(
                "sheet".into(),
                json!({
                    "description": if allow_omitted_sheet {
                        "The sheet to add it to; on an original `.dwg`, omit sheet to target that \
                         drawing."
                    } else {
                        "The sheet to add it to, in its row: it is on this sheet alone. Give \
                         sheet or template, not both."
                    },
                    "$ref": "#/$defs/sheet_name",
                }),
            );
            properties.insert(
                "template".into(),
                json!({
                    "description": "The template to add it to: it is on every sheet built \
                                    from it. Give sheet or template, not both.",
                    "$ref": "#/$defs/template_path",
                }),
            );
            properties.insert(
                "draw_order".into(),
                json!({
                    "description": "Where in draw order to place the new object: \"front\" \
                                    (default, drawn last), \"back\" (drawn first), {\"above\": \
                                    target}, or {\"under\": target}.",
                    "$ref": "#/$defs/draw_order",
                }),
            );
            let place = if allow_omitted_sheet {
                json!({ "not": { "required": ["sheet", "template"] } })
            } else if object == "revision_cloud" {
                json!({
                    "allOf": [
                        { "not": { "required": ["sheet", "template"] } },
                        {
                            "anyOf": [
                                { "required": ["sheet"] },
                                { "required": ["template"] },
                                { "required": ["around"] },
                            ]
                        },
                    ]
                })
            } else {
                json!({ "oneOf": [{ "required": ["sheet"] }, { "required": ["template"] }] })
            };
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
        Kind::DrawOrder => {
            let mut schema = json!({
                "type": "object",
                "required": ["order"],
                "additionalProperties": false,
                "allOf": [copied_objects_rule()],
                "properties": {
                    "order": {
                        "description": "Where to move the object(s) in draw order: \"front\", \
                                        \"back\", {\"above\": target}, or {\"under\": target}.",
                        "$ref": "#/$defs/draw_order",
                    },
                },
            });
            schema["properties"]["target"] = json!({
                "description": "The object to reorder, by its ID or its name. Give target or targets.",
                "$ref": "#/$defs/target",
            });
            schema["properties"]["targets"] = json!({
                "description": "The objects to reorder, each by its ID or its name, all in one \
                                record or space, preserving their relative order.",
                "type": "array",
                "items": { "$ref": "#/$defs/target" },
                "minItems": 1,
            });
            schema
        }
        Kind::TextToFront => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": if allow_omitted_sheet {
                            "The sheet whose annotations to bring to the front; on an original \
                             `.dwg`, omit sheet to target that drawing."
                        } else {
                            "The sheet whose row's annotations to bring to the front. Give sheet \
                             or template, not both."
                        },
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template whose annotations to bring to the front. \
                                        Give sheet or template, not both.",
                        "$ref": "#/$defs/template_path",
                    },
                    "which": {
                        "description": "Which annotations to bring to the front: \"text\" (text \
                                        and mtext), \"dimensions\", \"leaders\" (mleader and \
                                        leader), or \"all\" (default when omitted).",
                        "enum": TEXT_TO_FRONT_KINDS,
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::Table(op, table) => table_schema(op, table, defs, allow_omitted_sheet),
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
        Kind::SetBlockState => json!({
            "type": "object",
            "required": ["target", "state"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "state": {
                    "description": "Dynamic parameter values to set on the insert, mapped from \
                                    parameter name to value (or null to reset to default).",
                    "type": "object",
                    "minProperties": 1,
                    "additionalProperties": {
                        "anyOf": [
                            { "$ref": "#/$defs/dynamic_value" },
                            { "type": "null" },
                        ],
                    },
                },
            },
        }),
        Kind::SetAttributes => json!({
            "type": "object",
            "required": ["where", "tag", "value"],
            "additionalProperties": false,
            "properties": {
                "where": insert_selection_schema(),
                "sheets": {
                    "description": "The sheets to search and edit, by name. Left out, every \
                                    sheet of the drawing set.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/sheet_name" },
                    "minItems": 1,
                },
                "tag": { "$ref": "#/$defs/attribute_tag" },
                "occurrence": {
                    "description": "Which of the attributes with this tag to set, counted from \
                                    1 in the order the block defines them. Needed when the block \
                                    defines the tag more than once; 1 or left out when it defines \
                                    it once.",
                    "type": "integer",
                    "minimum": 1,
                },
                "value": { "$ref": "#/$defs/attribute_value" },
                ROW_VALUE_KEY: row_value_key(),
            },
        }),
        Kind::ReplaceText => json!({
            "type": "object",
            "required": ["find", "replace"],
            "additionalProperties": false,
            "properties": {
                "find": {
                    "description": "The text or regular expression pattern to find.",
                    "type": "string",
                    "minLength": 1,
                },
                "replace": {
                    "description": "The replacement text. In `pattern` mode, `$1`, `${name}` and \
                                    `$$` expand capture groups; in `exact` and `whole_word` modes \
                                    it is written literally.",
                    "type": "string",
                },
                "match": {
                    "description": "How `find` matches: `exact` (literal substring, default), \
                                    `whole_word` (bounded by non-word characters), or `pattern` \
                                    (regular expression).",
                    "enum": TextMatchKind::ALL,
                },
                "case_sensitive": {
                    "description": "Whether matching respects letter case. True when left out.",
                    "type": "boolean",
                },
                "sheets": {
                    "description": "The sheets to search and edit, by name. Left out, every \
                                    sheet of the drawing set.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/sheet_name" },
                    "minItems": 1,
                },
                "in": {
                    "description": "Which text-bearing places to search: any non-empty subset of \
                                    `text`, `mtext` and `attribute`. Left out, all three.",
                    "type": "array",
                    "items": { "enum": TextScope::ALL },
                    "minItems": 1,
                    "uniqueItems": true,
                },
                "tag": {
                    "description": "When replacing in attributes, restrict attribute matches to \
                                    this tag (case-insensitive).",
                    "$ref": "#/$defs/attribute_tag",
                },
                "block": {
                    "description": "When replacing in attributes, restrict attribute matches to \
                                    inserts of this block (case-insensitive).",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Offset => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "oneOf": [
                {
                    "required": ["distance", "side"],
                    "not": { "required": ["through"] },
                },
                {
                    "required": ["through"],
                    "not": {
                        "anyOf": [
                            { "required": ["distance"] },
                            { "required": ["side"] },
                        ],
                    },
                },
            ],
            "properties": {
                "target": target,
                "distance": {
                    "description": "Positive offset distance in the set's declared unit, or a \
                                    length carrying its own unit such as \"12 mm\". Give with \
                                    `side` (instead of `through`).",
                    "$ref": "#/$defs/length_more_than_0",
                },
                "side": {
                    "description": "Which side to offset to: `left` or `right` (looking from \
                                    start to end), `inside` or `outside` (for circles, arcs, \
                                    closed polylines, ellipses or closed splines), or a point / \
                                    snapped point on the desired side.",
                    "oneOf": [
                        { "enum": ["left", "right", "inside", "outside"] },
                        { "$ref": "#/$defs/snapped_point" },
                    ],
                },
                "through": {
                    "description": "A point or snapped point that the offset curve passes \
                                    through (AutoCAD's OFFSET Through mode). Give instead of \
                                    `distance` and `side`.",
                    "$ref": "#/$defs/snapped_point",
                },
                "id": {
                    "description": "ID for the new offset object; assigned automatically when \
                                    left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "Optional name for the new offset object.",
                    "type": "string",
                    "minLength": 1,
                },
                "layer": {
                    "description": "Layer for the new offset object; inherits `target`'s layer \
                                    when left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "erase_source": {
                    "description": "Delete `target` after creating the offset (AutoCAD's OFFSET \
                                    Erase option). False when left out.",
                    "type": "boolean",
                },
            },
        }),
        Kind::Trim => json!({
            "type": "object",
            "required": ["target", "keep"],
            "additionalProperties": false,
            "oneOf": [
                { "required": ["boundary"], "not": { "required": ["boundaries"] } },
                { "required": ["boundaries"], "not": { "required": ["boundary"] } },
            ],
            "properties": {
                "target": target,
                "boundary": {
                    "description": "The cutting-edge object, by its ID or its name. Give \
                                    `boundary` or `boundaries`.",
                    "$ref": "#/$defs/target",
                },
                "boundaries": {
                    "description": "One or more cutting-edge objects, each by its ID or its name.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/target" },
                    "minItems": 1,
                },
                "keep": {
                    "description": "Which part of `target` to keep: `start` (start to first \
                                    crossing), `end` (last crossing to end), `between` (first \
                                    to last crossing), or `ends` (removes middle, splitting an \
                                    open curve into two).",
                    "enum": TrimKeep::ALL,
                },
                "new_id": {
                    "description": "Optional ID for the second object created when `keep: \"ends\"` \
                                    splits an open curve into two.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Extend => json!({
            "type": "object",
            "required": ["target", "end"],
            "additionalProperties": false,
            "oneOf": [
                { "required": ["boundary"], "not": { "required": ["boundaries"] } },
                { "required": ["boundaries"], "not": { "required": ["boundary"] } },
            ],
            "properties": {
                "target": target,
                "boundary": {
                    "description": "The boundary object to extend to, by its ID or its name. \
                                    Give `boundary` or `boundaries`.",
                    "$ref": "#/$defs/target",
                },
                "boundaries": {
                    "description": "One or more boundary objects to extend to, each by its ID \
                                    or its name.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/target" },
                    "minItems": 1,
                },
                "end": {
                    "description": "Which end of `target` to extend: `start`, `end`, or `both`.",
                    "enum": ExtendEnd::ALL,
                },
            },
        }),
        Kind::Fillet => json!({
            "type": "object",
            "required": ["radius"],
            "additionalProperties": false,
            "oneOf": [
                {
                    "required": ["first", "second"],
                    "not": { "required": ["target"] },
                },
                {
                    "required": ["target"],
                    "not": {
                        "anyOf": [
                            { "required": ["first"] },
                            { "required": ["second"] },
                            { "required": ["first_end"] },
                            { "required": ["second_end"] },
                            { "required": ["trim"] },
                            { "required": ["new_id"] },
                            { "required": ["layer"] },
                        ],
                    },
                },
            ],
            "properties": {
                "first": {
                    "description": "First object to fillet (line, arc or circle), by ID or name. \
                                    Give `first` and `second`, or `target` for a polyline.",
                    "$ref": "#/$defs/target",
                },
                "second": {
                    "description": "Second object to fillet (line, arc or circle), by ID or name.",
                    "$ref": "#/$defs/target",
                },
                "target": {
                    "description": "A flat polyline whose corners are all filleted in place \
                                    (AutoCAD's FILLET Polyline option).",
                    "$ref": "#/$defs/target",
                },
                "radius": {
                    "description": "Non-negative fillet radius in the set's declared unit, or a \
                                    length carrying its own unit such as \"5 mm\". Use 0 to bring \
                                    two lines to a sharp corner or square up arc corners on a \
                                    polyline.",
                    "$ref": "#/$defs/length_not_negative",
                },
                "trim": {
                    "description": "Whether `first` and `second` are trimmed or extended to the \
                                    fillet arc (AutoCAD's TRIMMODE). True when left out.",
                    "type": "boolean",
                },
                "first_end": {
                    "description": "Which end of `first` to keep when disambiguating a crossing: \
                                    `start` or `end`.",
                    "enum": CornerEnd::ALL,
                },
                "second_end": {
                    "description": "Which end of `second` to keep when disambiguating a crossing: \
                                    `start` or `end`.",
                    "enum": CornerEnd::ALL,
                },
                "new_id": {
                    "description": "Optional ID for the new fillet arc created in two-object mode \
                                    when `radius > 0`.",
                    "type": "string",
                    "minLength": 1,
                },
                "layer": {
                    "description": "Optional layer for the new fillet arc; inherits `first`'s \
                                    layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Chamfer => json!({
            "type": "object",
            "required": [],
            "additionalProperties": false,
            "allOf": [
                {
                    "oneOf": [
                        {
                            "required": ["first", "second"],
                            "not": { "required": ["target"] },
                        },
                        {
                            "required": ["target"],
                            "not": {
                                "anyOf": [
                                    { "required": ["first"] },
                                    { "required": ["second"] },
                                    { "required": ["first_end"] },
                                    { "required": ["second_end"] },
                                    { "required": ["trim"] },
                                    { "required": ["new_id"] },
                                    { "required": ["layer"] },
                                ],
                            },
                        },
                    ],
                },
                {
                    "oneOf": [
                        {
                            "required": ["distance"],
                            "not": {
                                "anyOf": [
                                    { "required": ["distance1"] },
                                    { "required": ["distance2"] },
                                    { "required": ["length"] },
                                    { "required": ["angle"] },
                                ],
                            },
                        },
                        {
                            "required": ["distance1", "distance2"],
                            "not": {
                                "anyOf": [
                                    { "required": ["distance"] },
                                    { "required": ["length"] },
                                    { "required": ["angle"] },
                                ],
                            },
                        },
                        {
                            "required": ["length", "angle"],
                            "not": {
                                "anyOf": [
                                    { "required": ["distance"] },
                                    { "required": ["distance1"] },
                                    { "required": ["distance2"] },
                                ],
                            },
                        },
                    ],
                },
            ],
            "properties": {
                "first": {
                    "description": "First line to chamfer, by ID or name. Give `first` and \
                                    `second`, or `target` for a polyline.",
                    "$ref": "#/$defs/target",
                },
                "second": {
                    "description": "Second line to chamfer, by ID or name.",
                    "$ref": "#/$defs/target",
                },
                "target": {
                    "description": "A flat polyline whose corners are all chamfered in place \
                                    (AutoCAD's CHAMFER Polyline option).",
                    "$ref": "#/$defs/target",
                },
                "distance": {
                    "description": "Equal non-negative chamfer distance along both legs (0 brings \
                                    two lines to a sharp corner). Give `distance`, `distance1` + \
                                    `distance2`, or `length` + `angle`.",
                    "$ref": "#/$defs/length_not_negative",
                },
                "distance1": {
                    "description": "Non-negative chamfer distance along `first` (or each incoming \
                                    polyline segment). Give with `distance2`.",
                    "$ref": "#/$defs/length_not_negative",
                },
                "distance2": {
                    "description": "Non-negative chamfer distance along `second` (or each outgoing \
                                    polyline segment). Give with `distance1`.",
                    "$ref": "#/$defs/length_not_negative",
                },
                "length": {
                    "description": "Non-negative chamfer length along `first` (or each incoming \
                                    polyline segment). Give with `angle`.",
                    "$ref": "#/$defs/length_not_negative",
                },
                "angle": {
                    "description": "Chamfer angle in degrees, strictly between 0 and 180, measured \
                                    from `first` (or incoming polyline segment) to the bevel line. \
                                    Give with `length`.",
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "exclusiveMaximum": 180,
                },
                "trim": {
                    "description": "Whether `first` and `second` are trimmed or extended to the \
                                    chamfer line (AutoCAD's TRIMMODE). True when left out.",
                    "type": "boolean",
                },
                "first_end": {
                    "description": "Which end of `first` to keep when disambiguating a crossing: \
                                    `start` or `end`.",
                    "enum": CornerEnd::ALL,
                },
                "second_end": {
                    "description": "Which end of `second` to keep when disambiguating a crossing: \
                                    `start` or `end`.",
                    "enum": CornerEnd::ALL,
                },
                "new_id": {
                    "description": "Optional ID for the new chamfer line created in two-object \
                                    mode when chamfer distances are non-zero.",
                    "type": "string",
                    "minLength": 1,
                },
                "layer": {
                    "description": "Optional layer for the new chamfer line; inherits `first`'s \
                                    layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Break => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "oneOf": [
                {
                    "required": ["at"],
                    "not": {
                        "anyOf": [
                            { "required": ["from"] },
                            { "required": ["to"] },
                        ],
                    },
                },
                {
                    "required": ["from", "to"],
                    "not": {
                        "anyOf": [
                            { "required": ["at"] },
                            { "required": ["gap"] },
                        ],
                    },
                },
            ],
            "properties": {
                "target": target,
                "at": {
                    "description": "Point (or snapped point) at which to split `target` \
                                    (AutoCAD's BREAKATPOINT) or center `gap`.",
                    "$ref": "#/$defs/snapped_point",
                },
                "gap": {
                    "description": "Optional positive arc-length gap centered at `at` (for \
                                    example breaking a wire around a crossing wire).",
                    "$ref": "#/$defs/length_more_than_0",
                },
                "from": {
                    "description": "First break point (or snapped point) for two-point BREAK. \
                                    Give with `to` instead of `at`.",
                    "$ref": "#/$defs/snapped_point",
                },
                "to": {
                    "description": "Second break point (or snapped point) for two-point BREAK. \
                                    On closed curves, removes the anticlockwise span from `from` \
                                    to `to`.",
                    "$ref": "#/$defs/snapped_point",
                },
                "new_id": {
                    "description": "Optional ID for the second object created when `break` \
                                    splits an open curve into two pieces.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Join => json!({
            "type": "object",
            "required": ["targets"],
            "additionalProperties": false,
            "properties": {
                "targets": {
                    "description": "Two or more objects (`line`, `arc`, or open `polyline`) to \
                                    join into one. `targets[0]`'s ID, layer and properties carry \
                                    on; the rest are absorbed and reported under `deleted`.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/target" },
                    "minItems": 2,
                },
                "tolerance": {
                    "description": "Optional non-negative endpoint-matching fuzz distance in the \
                                    declared unit (or carrying its own unit). Defaults to 0.0001.",
                    "$ref": "#/$defs/length_not_negative",
                },
            },
        }),
        Kind::Stretch => {
            let corner_schema = json!({
                "anyOf": [
                    { "$ref": "#/$defs/snapped_point" },
                    {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 2,
                        "items": { "$ref": "#/$defs/length" },
                    },
                ],
            });
            let sheet_rule = if allow_omitted_sheet {
                json!({ "not": { "required": ["sheet", "template"] } })
            } else {
                json!({ "oneOf": [{ "required": ["sheet"] }, { "required": ["template"] }] })
            };
            json!({
                "type": "object",
                "required": ["box"],
                "additionalProperties": false,
                "allOf": [
                    sheet_rule,
                    {
                        "oneOf": [
                            { "required": ["by"], "not": { "anyOf": [{ "required": ["from"] }, { "required": ["to"] }] } },
                            { "required": ["from", "to"], "not": { "required": ["by"] } },
                        ],
                    },
                ],
                "properties": {
                    "sheet": {
                        "description": if allow_omitted_sheet {
                            "The sheet to stretch on; on an original `.dwg`, omit `sheet` to \
                             target that drawing."
                        } else {
                            "The sheet to stretch on. Give `sheet` or `template`, not both."
                        },
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template whose objects to stretch across every sheet \
                                        built from it. Give `sheet` or `template`, not both.",
                        "$ref": "#/$defs/template_path",
                    },
                    "space": {
                        "description": "Space to stretch in: `model` (default) or a layout name.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "targets": {
                        "description": "Optional list of objects to restrict stretching to, each \
                                        by ID or name. Left out, every unlocked object in the \
                                        space whose vertices lie inside `box`.",
                        "type": "array",
                        "items": { "$ref": "#/$defs/target" },
                        "minItems": 1,
                    },
                    "box": {
                        "description": "Two opposite corners of the stretch selection box, \
                                        `[corner1, corner2]`. Each corner is `[x, y]`, `[x, y, z]` \
                                        in the declared unit (or carrying its own), or a snapped \
                                        point on a named object.",
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 2,
                        "items": corner_schema,
                    },
                    "by": {
                        "description": "Displacement `[dx, dy, dz]` in the declared unit or \
                                        carrying its own. Give `by`, or `from` and `to`.",
                        "$ref": "#/$defs/point",
                    },
                    "from": {
                        "description": "Base point (or snapped point) for the stretch displacement.",
                        "$ref": "#/$defs/snapped_point",
                    },
                    "to": {
                        "description": "Second point (or snapped point); vertices inside `box` \
                                        move by `to` minus `from`.",
                        "$ref": "#/$defs/snapped_point",
                    },
                },
            })
        }
        Kind::Explode => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "allOf": [copied_objects_rule()],
                "properties": {
                    "id_prefix": {
                        "description": "Optional prefix for the newly created objects' local IDs \
                                        (`<prefix>-1`, `<prefix>-2`, ...). When omitted, drawing \
                                        sets derive IDs from the exploded object's ID and original \
                                        DWGs use fresh handle IDs.",
                        "type": "string",
                        "minLength": 1,
                    },
                },
            });
            copied_objects_properties(&mut schema);
            schema
        }
        Kind::SetField => json!({
            "type": "object",
            "required": ["target", "field"],
            "additionalProperties": false,
            "properties": {
                "target": { "$ref": "#/$defs/target" },
                "tag": {
                    "description": "When `target` is an insert, the attribute tag to attach or \
                                    clear the field on. Omit for a text or mtext target.",
                    "type": "string",
                    "minLength": 1,
                },
                "occurrence": {
                    "description": "Which attribute to set when the insert's block defines `tag` \
                                    more than once, counted from 1.",
                    "type": "integer",
                    "minimum": 1,
                },
                "field": {
                    "description": "The AutoCAD field definition to attach, or `null` to clear \
                                    the field and keep plain text.",
                    "anyOf": [
                        crate::description::field::field_schema(),
                        { "type": "null" },
                    ],
                },
                "value": {
                    "description": "Optional explicit display text for the host object or \
                                    attribute (overrides auto-evaluated display text, or sets \
                                    the plain text when clearing with `field: null`).",
                    "type": "string",
                },
            },
        }),
        Kind::SetPageSetup => {
            let mut schema = json!({
                "type": "object",
                "required": ["set"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit: the template is \
                                        that sheet's. Give sheet or template (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "layout": {
                        "description": "Optional layout tab name whose page setup to edit (or \
                                        `Model` on a DWG). Omit for the sheet's top-level page \
                                        setup (or the first paper-space layout on a DWG).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "set": {
                        "description": format!(
                            "Page setup keys to set, each to its new value as a sheet \
                             description writes it; null clears a key back to its default. \
                             Keys: {}.",
                            crate::description::page_setup::PAGE_SETUP_KEYS.join(", ")
                        ),
                        "type": "object",
                        "minProperties": 1,
                        "propertyNames": {
                            "enum": crate::description::page_setup::PAGE_SETUP_KEYS,
                        },
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::AttachXref => {
            let mut schema = json!({
                "type": "object",
                "required": ["name", "path"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit: the template is \
                                        that sheet's (optional on an original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The external reference's block name (e.g. `GRID` or \
                                        `TITLE_BLOCK`).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "path": {
                        "description": "Path to the referenced DWG or DXF file, relative to the \
                                        template or host drawing folder (with `/` separators).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "kind": {
                        "description": "Attachment mode: `attach` (default, carried along when \
                                        this drawing is referenced) or `overlay` (drawn in this \
                                        drawing only).",
                        "enum": crate::description::xref::XREF_KINDS,
                    },
                    "unloaded": {
                        "description": "Whether the external reference is unloaded. Default false.",
                        "type": "boolean",
                    },
                    "place": {
                        "description": "Whether to place an insert of the external reference. \
                                        Default true.",
                        "type": "boolean",
                    },
                    "position": {
                        "description": "Insertion point `[x, y, z]`. Default `[0, 0, 0]`.",
                        "type": "array",
                        "minItems": 3,
                        "maxItems": 3,
                        "items": { "type": ["number", "string"] },
                    },
                    "scale": {
                        "description": "Uniform insertion scale (> 0). Default 1.",
                        "type": "number",
                        "exclusiveMinimum": 0,
                    },
                    "rotation": {
                        "description": "Insertion rotation in degrees anticlockwise. Default 0.",
                        "type": "number",
                    },
                    "layer": {
                        "description": "Layer for the placed insert. Default `0`.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "space": {
                        "description": "Optional space (`model`, `paper`, or a layout name) for \
                                        the placed insert.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "id": {
                        "description": "Optional object ID for the placed insert.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "insert_name": {
                        "description": "Optional object name for the placed insert.",
                        "type": "string",
                        "minLength": 1,
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::DetachXref => {
            let mut schema = json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The external reference block name to detach.",
                        "type": "string",
                        "minLength": 1,
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::BindXref => {
            let mut schema = json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The external reference block name to bind into a local \
                                        block.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "mode": {
                        "description": "How xref-dependent tables are named: `bind` (default, \
                                        AutoCAD `xref$0$name` prefixing) or `insert` (merge names \
                                        directly into host tables).",
                        "enum": crate::model::XrefBindMode::ALL.map(crate::model::XrefBindMode::name),
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::AddGroup => {
            let mut schema = json!({
                "type": "object",
                "required": ["name", "targets"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "The sheet whose row holds the group (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to define the group in (shared by every \
                                        sheet built from it). Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The group's name, unique on the sheet (case-insensitive).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "targets": {
                        "description": "The member objects to include in the group, at least one.",
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/target" },
                    },
                    "description": {
                        "description": "Optional human-readable description of the group.",
                        "type": "string",
                    },
                    "selectable": {
                        "description": "Whether AutoCAD selects the group as one unit (default \
                                        true).",
                        "type": "boolean",
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::Ungroup => {
            let mut schema = json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "The sheet holding the group (optional on an original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template holding the group. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The name of the group to remove (case-insensitive).",
                        "type": "string",
                        "minLength": 1,
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::EditGroup => {
            let mut schema = json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "The sheet holding the group (optional on an original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template holding the group. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "The name of the group to edit (case-insensitive).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "add": {
                        "description": "Objects to add to the group.",
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/target" },
                    },
                    "remove": {
                        "description": "Objects to remove from the group.",
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/target" },
                    },
                    "rename": {
                        "description": "New name for the group (unique, case-insensitive).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "description": {
                        "description": "Updated human-readable description.",
                        "type": "string",
                    },
                    "selectable": {
                        "description": "Whether AutoCAD selects the group as one unit.",
                        "type": "boolean",
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::SetAnnotationScale => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "scale": {
                        "description": "New active annotation scale (`CANNOSCALE`), such as \
                                        `1:50` or a name in `annotation_scales`.",
                        "type": "string",
                        "minLength": 1,
                    },
                    "anno_all_visible": {
                        "description": "Sets `ANNOALLVISIBLE`: true draws every annotative object \
                                        regardless of viewport scale; false hides annotative \
                                        objects that do not support a viewport's scale.",
                        "type": "boolean",
                    },
                    "add_scales": {
                        "description": "Mapping from scale name to `[paper_units, drawing_units]` \
                                        to add or update in `annotation_scales` (`ACAD_SCALELIST`).",
                        "type": "object",
                        "minProperties": 1,
                        "additionalProperties": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 2,
                            "items": { "type": "number", "exclusiveMinimum": 0 },
                        },
                    },
                    "remove_scales": {
                        "description": "Names of annotation scales to remove from \
                                        `annotation_scales` (`ACAD_SCALELIST`).",
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 },
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::SetPointStyle => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "pdmode": {
                        "description": "AutoCAD PDMODE point display mode: 0 (dot), 1 (none), \
                                        2 (+), 3 (x), 4 (|), optionally plus 32 (circle) and/or \
                                        64 (square).",
                        "type": "integer",
                        "enum": [0, 1, 2, 3, 4, 32, 33, 34, 35, 36, 64, 65, 66, 67, 68, 96, 97, 98, 99, 100],
                    },
                    "pdsize": {
                        "description": "AutoCAD PDSIZE point display size: positive for absolute \
                                        drawing units, 0 for 5% of viewport height, negative for \
                                        percentage of viewport height.",
                        "type": "number",
                    },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::AddMLineStyle => {
            let mut schema = json!({
                "type": "object",
                "required": ["name"],
                "additionalProperties": false,
                "properties": {
                    "sheet": {
                        "description": "A sheet built from the template to edit (optional on an \
                                        original DWG).",
                        "$ref": "#/$defs/sheet_name",
                    },
                    "template": {
                        "description": "The template to edit. Give sheet or template.",
                        "$ref": "#/$defs/template_path",
                    },
                    "name": {
                        "description": "Name of the multiline style (e.g. `WALL`, `PIPE`, or \
                                        `Standard`).",
                        "type": "string",
                        "minLength": 1,
                    },
                    "description": { "type": "string" },
                    "elements": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "required": ["offset"],
                            "additionalProperties": false,
                            "properties": {
                                "offset": { "type": "number" },
                                "color": { "$ref": "#/$defs/object_color" },
                                "linetype": { "$ref": "#/$defs/object_linetype" },
                            },
                        },
                    },
                    "fill_color": { "$ref": "#/$defs/object_color" },
                    "display_joints": { "type": "boolean" },
                    "start_angle": { "type": "number", "minimum": 10.0, "maximum": 170.0 },
                    "end_angle": { "type": "number", "minimum": 10.0, "maximum": 170.0 },
                    "start_square_cap": { "type": "boolean" },
                    "start_round_cap": { "type": "boolean" },
                    "start_inner_arcs_cap": { "type": "boolean" },
                    "end_square_cap": { "type": "boolean" },
                    "end_round_cap": { "type": "boolean" },
                    "end_inner_arcs_cap": { "type": "boolean" },
                },
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::AddBox => {
            let mut props = Map::new();
            props.insert(
                "sheet".into(),
                json!({
                    "description": "The sheet to add the 3D box to, in its row (optional on an original DWG).",
                    "$ref": "#/$defs/sheet_name",
                }),
            );
            props.insert(
                "template".into(),
                json!({
                    "description": "The template to add the 3D box to. Give sheet or template.",
                    "$ref": "#/$defs/template_path",
                }),
            );
            props.insert(
                "draw_order".into(),
                json!({
                    "description": "Optional draw-order placement: front, back, {\"above\": target}, or {\"under\": target}.",
                    "$ref": "#/$defs/draw_order",
                }),
            );
            props.insert(
                "origin".into(),
                json!({
                    "description": "Minimum corner `[x, y, z]` of the box (or a snapped point).",
                    "$ref": "#/$defs/snapped_point",
                }),
            );
            props.insert(
                "size".into(),
                json!({
                    "description": "Positive dimensions `[length_x, width_y, height_z]` (give `size`, or `length`, `width` and `height`).",
                    "type": "array",
                    "items": { "$ref": "#/$defs/length_more_than_0" },
                    "minItems": 3,
                    "maxItems": 3,
                }),
            );
            props.insert(
                "length".into(),
                json!({
                    "description": "Box X dimension (give with `width` and `height`, or give `size`).",
                    "$ref": "#/$defs/length_more_than_0",
                }),
            );
            props.insert(
                "width".into(),
                json!({
                    "description": "Box Y dimension (give with `length` and `height`, or give `size`).",
                    "$ref": "#/$defs/length_more_than_0",
                }),
            );
            props.insert(
                "height".into(),
                json!({
                    "description": "Box Z dimension (give with `length` and `width`, or give `size`).",
                    "$ref": "#/$defs/length_more_than_0",
                }),
            );
            if let Some(solid_props) = defs["solid3d"].get("properties").and_then(Value::as_object)
            {
                for key in [
                    "layer",
                    "color",
                    "linetype",
                    "lineweight",
                    "plot_style",
                    "transparency",
                    "space",
                    "id",
                    "name",
                ] {
                    if let Some(v) = solid_props.get(key) {
                        props.insert(key.into(), v.clone());
                    }
                }
            }
            let mut schema = json!({
                "type": "object",
                "required": ["origin"],
                "additionalProperties": false,
                "properties": props,
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::AddCylinder => {
            let mut props = Map::new();
            props.insert(
                "sheet".into(),
                json!({
                    "description": "The sheet to add the 3D cylinder to, in its row (optional on an original DWG).",
                    "$ref": "#/$defs/sheet_name",
                }),
            );
            props.insert(
                "template".into(),
                json!({
                    "description": "The template to add the 3D cylinder to. Give sheet or template.",
                    "$ref": "#/$defs/template_path",
                }),
            );
            props.insert(
                "draw_order".into(),
                json!({
                    "description": "Optional draw-order placement: front, back, {\"above\": target}, or {\"under\": target}.",
                    "$ref": "#/$defs/draw_order",
                }),
            );
            props.insert(
                "base".into(),
                json!({
                    "description": "Bottom circle centre `[x, y, z]` of the cylinder (or a snapped point).",
                    "$ref": "#/$defs/snapped_point",
                }),
            );
            props.insert(
                "radius".into(),
                json!({
                    "description": "Positive radius of the cylinder.",
                    "$ref": "#/$defs/length_more_than_0",
                }),
            );
            props.insert(
                "height".into(),
                json!({
                    "description": "Positive height of the cylinder along `+Z`.",
                    "$ref": "#/$defs/length_more_than_0",
                }),
            );
            if let Some(solid_props) = defs["solid3d"].get("properties").and_then(Value::as_object)
            {
                for key in [
                    "layer",
                    "color",
                    "linetype",
                    "lineweight",
                    "plot_style",
                    "transparency",
                    "space",
                    "id",
                    "name",
                ] {
                    if let Some(v) = solid_props.get(key) {
                        props.insert(key.into(), v.clone());
                    }
                }
            }
            let mut schema = json!({
                "type": "object",
                "required": ["base", "radius", "height"],
                "additionalProperties": false,
                "properties": props,
            });
            if allow_omitted_sheet {
                schema["not"] = json!({ "required": ["sheet", "template"] });
            } else {
                schema["oneOf"] = json!([{ "required": ["sheet"] }, { "required": ["template"] }]);
            }
            schema
        }
        Kind::Extrude => json!({
            "type": "object",
            "required": ["target", "height"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "height": {
                    "description": "Non-zero extrusion distance (in drawing units).",
                    "$ref": "#/$defs/length",
                },
                "direction": {
                    "description": "Optional non-zero 3D extrusion direction vector (default `[0, 0, 1]`).",
                    "$ref": "#/$defs/point",
                },
                "taper_angle": {
                    "description": "Optional draft/taper angle in degrees between -85 and 85 (positive tapers inward; default 0).",
                    "type": "number",
                    "minimum": -85.0,
                    "maximum": 85.0,
                },
                "erase_source": {
                    "description": "Whether to delete the source 2D profile (AutoCAD DELOBJ). True when left out.",
                    "type": "boolean",
                },
                "layer": {
                    "description": "Optional layer for the extruded solid; inherits the source profile's layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "id": {
                    "description": "Optional ID for the extruded 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "Optional name for the extruded 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::Revolve => json!({
            "type": "object",
            "required": ["target", "axis_origin", "axis_direction"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "axis_origin": {
                    "description": "A point on the axis of revolution (or a snapped point).",
                    "$ref": "#/$defs/snapped_point",
                },
                "axis_direction": {
                    "description": "Non-zero 3D direction vector of the axis of revolution.",
                    "$ref": "#/$defs/point",
                },
                "angle": {
                    "description": "Sweep angle in degrees in `(0, 360]` (default 360).",
                    "type": "number",
                    "exclusiveMinimum": 0.0,
                    "maximum": 360.0,
                },
                "erase_source": {
                    "description": "Whether to delete the source 2D profile (AutoCAD DELOBJ). True when left out.",
                    "type": "boolean",
                },
                "layer": {
                    "description": "Optional layer for the revolved solid; inherits the source profile's layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "id": {
                    "description": "Optional ID for the revolved 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "Optional name for the revolved 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::SolidBoolean(BooleanOp::Union | BooleanOp::Intersect) => json!({
            "type": "object",
            "required": ["targets"],
            "additionalProperties": false,
            "properties": {
                "targets": {
                    "description": "Two or more 3D solids (`solid3d`) to combine; the first solid's ID carries on unless `id` overrides it.",
                    "type": "array",
                    "minItems": 2,
                    "items": target,
                },
                "layer": {
                    "description": "Optional layer for the resulting 3D solid; inherits the first solid's layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "id": {
                    "description": "Optional ID override for the resulting 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "Optional name for the resulting 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::SolidBoolean(BooleanOp::Subtract) => json!({
            "type": "object",
            "required": ["target", "subtract"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "subtract": {
                    "description": "One 3D solid, or a non-empty list of 3D solids, to subtract from `target`.",
                    "anyOf": [
                        { "$ref": "#/$defs/target" },
                        {
                            "type": "array",
                            "minItems": 1,
                            "items": { "$ref": "#/$defs/target" },
                        },
                    ],
                },
                "layer": {
                    "description": "Optional layer for the resulting 3D solid; inherits `target`'s layer when left out.",
                    "type": "string",
                    "minLength": 1,
                },
                "id": {
                    "description": "Optional ID override for the resulting 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
                "name": {
                    "description": "Optional name for the resulting 3D solid.",
                    "type": "string",
                    "minLength": 1,
                },
            },
        }),
        Kind::SetObjectScales => {
            let mut schema = json!({
                "type": "object",
                "required": [],
                "additionalProperties": false,
                "properties": {
                    "annotative": {
                        "description": "Turns annotative scaling on (`true`) or off (`false`) on \
                                        the targeted objects.",
                        "type": "boolean",
                    },
                    "scales": {
                        "description": "Replaces each object's supported `annotation_scales` list \
                                        with these scale names (cannot be combined with `add` or \
                                        `remove`).",
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 },
                    },
                    "add": {
                        "description": "Annotation scale names to add to each object's supported \
                                        scale list.",
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 },
                    },
                    "remove": {
                        "description": "Annotation scale names to remove from each object's \
                                        supported scale list.",
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 1 },
                    },
                },
            });
            copied_objects_properties(&mut schema);
            schema["allOf"] = json!([copied_objects_rule()]);
            schema
        }
    };
    schema["description"] = json!(does(spec));
    schema["properties"][OP_KEY] = op;
    let required = schema["required"].as_array_mut().expect("a list");
    required.insert(0, json!(OP_KEY));
    schema
}

/// The JSON Schema of the `where` query selection taken by `set_attributes`.
/// Mirrors `query find` / `query attributes`'s [`Selection`].
fn insert_selection_schema() -> Value {
    let text = |what: &str| json!({ "description": what, "type": "string" });
    let box_schema = |what: &str| {
        json!({
            "description": format!("{what} Two opposite corners, [[x1, y1], [x2, y2]], in the declared unit."),
            "type": "array",
            "minItems": 2,
            "maxItems": 2,
            "items": {
                "type": "array",
                "minItems": 2,
                "maxItems": 3,
                "items": { "type": "number" },
            },
        })
    };
    json!({
        "description": "Query selection choosing which inserts to edit (same keys as `query attributes`). An empty object `{}` selects every insert on the targeted sheets.",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": text("The insert's ID, such as `DEMO-001/title-block`."),
            "name": text("The insert's name, such as `title-block`."),
            "block": text("The block the insert places."),
            "tag": text("An attribute tag the insert carries, or with `value` the tag whose value is matched."),
            "value": text("An attribute value on the insert, in any attribute or in the one `tag` names."),
            "text": text("Text drawn inside the insert's block."),
            "match": {
                "description": "How the selection matches: `exact` (default) or `contains` (case-insensitive substring).",
                "enum": ["exact", "contains"],
            },
            "window": box_schema("Inserts wholly within this box."),
            "crossing": box_schema("Inserts within or crossing this box."),
            "space": {
                "description": "Inserts in this space: `model`, a layout name, or `paper`.",
                "type": "string",
                "minLength": 1,
            },
            "touching": {
                "description": "Inserts whose geometry meets this object.",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": text("The object's ID."),
                    "name": text("The object's name."),
                },
            },
        },
    })
}

/// Parses either polyline mode (`target`) or two-object mode (`first`, `second`,
/// optional `trim`, `first_end`, `second_end`, `new_id`, `layer`) for `fillet`
/// and `chamfer`.
fn read_corner_mode(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
) -> Result<CornerMode, (ErrorCode, String, String)> {
    let parse_target = |key: &str| -> Result<Target, (ErrorCode, String, String)> {
        let Some(val) = map.get(key) else {
            return Err((
                ErrorCode::MissingKey,
                format!("missing key `{key}`"),
                key.to_owned(),
            ));
        };
        let mut t = read_target(val).map_err(|(code, msg, sub)| {
            let field = if sub.is_empty() {
                key.to_owned()
            } else {
                format!("{key}.{sub}")
            };
            (code, msg, field)
        })?;
        if let Some(sheet) = default_sheet {
            if let Target::Name {
                sheet: slot @ None, ..
            } = &mut t
            {
                *slot = Some(sheet.to_owned());
            }
        }
        Ok(t)
    };
    let has_target = map.contains_key("target");
    let has_first = map.contains_key("first");
    let has_second = map.contains_key("second");

    if has_target {
        for forbidden in [
            "first",
            "second",
            "trim",
            "first_end",
            "second_end",
            "new_id",
            "layer",
        ] {
            if map.contains_key(forbidden) {
                return Err((
                    ErrorCode::Usage,
                    format!(
                        "`{forbidden}` applies only to two-object mode (`first` and `second`), \
                         not polyline mode (`target`)"
                    ),
                    forbidden.to_owned(),
                ));
            }
        }
        return Ok(CornerMode::Polyline {
            target: parse_target("target")?,
        });
    }

    if !has_first && !has_second {
        return Err((
            ErrorCode::MissingKey,
            "give `first` and `second` (two objects), or `target` (a polyline)".to_owned(),
            "first".to_owned(),
        ));
    }
    let first = parse_target("first")?;
    let second = parse_target("second")?;
    let trim = match map.get("trim") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected `trim`, true or false".to_owned(),
                "trim".to_owned(),
            ))
        }
    };
    let parse_end = |key: &str| -> Result<Option<CornerEnd>, (ErrorCode, String, String)> {
        match map.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => CornerEnd::parse(s).map(Some).ok_or_else(|| {
                (
                    ErrorCode::BadValue,
                    format!(
                        "unknown `{key}` `{s}`; expected one of {}",
                        CornerEnd::ALL.join(", ")
                    ),
                    key.to_owned(),
                )
            }),
            Some(_) => Err((
                ErrorCode::WrongType,
                format!("expected `{key}`, one of {}", CornerEnd::ALL.join(", ")),
                key.to_owned(),
            )),
        }
    };
    let first_end = parse_end("first_end")?;
    let second_end = parse_end("second_end")?;
    let parse_opt_str = |key: &str| -> Result<Option<String>, (ErrorCode, String, String)> {
        match map.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.clone())),
            Some(Value::String(_)) => Err((
                ErrorCode::BadValue,
                format!("`{key}` cannot be empty"),
                key.to_owned(),
            )),
            Some(_) => Err((
                ErrorCode::WrongType,
                format!("expected `{key}`, a string"),
                key.to_owned(),
            )),
        }
    };
    let new_id = parse_opt_str("new_id")?;
    let layer = parse_opt_str("layer")?;
    Ok(CornerMode::TwoObjects {
        first,
        second,
        trim,
        first_end,
        second_end,
        new_id,
        layer,
    })
}

/// Parses `distance` (equal cutbacks), `distance1` + `distance2`, or `length` +
/// `angle` for `chamfer`.
fn read_chamfer_spec(
    map: &Map<String, Value>,
    units: Unit,
) -> Result<ChamferSpec, (ErrorCode, String, String)> {
    let has_d = map.contains_key("distance");
    let has_d1 = map.contains_key("distance1");
    let has_d2 = map.contains_key("distance2");
    let has_len = map.contains_key("length");
    let has_ang = map.contains_key("angle");
    let read_non_neg = |key: &str| -> Result<f64, (ErrorCode, String, String)> {
        let v = map.get(key).unwrap();
        let d = length(v, units).map_err(|msg| (ErrorCode::BadValue, msg, key.to_owned()))?;
        if !d.is_finite() || d < 0.0 {
            return Err((
                ErrorCode::BadValue,
                format!("`{key}` must be a non-negative length"),
                key.to_owned(),
            ));
        }
        Ok(d)
    };
    match (has_d, has_d1 || has_d2, has_len || has_ang) {
        (true, false, false) => {
            let d = read_non_neg("distance")?;
            Ok(ChamferSpec::Distances(d, d))
        }
        (false, true, false) => {
            if !has_d1 {
                return Err((
                    ErrorCode::MissingKey,
                    "missing key `distance1` to go with `distance2`".to_owned(),
                    "distance1".to_owned(),
                ));
            }
            if !has_d2 {
                return Err((
                    ErrorCode::MissingKey,
                    "missing key `distance2` to go with `distance1`".to_owned(),
                    "distance2".to_owned(),
                ));
            }
            Ok(ChamferSpec::Distances(
                read_non_neg("distance1")?,
                read_non_neg("distance2")?,
            ))
        }
        (false, false, true) => {
            if !has_len {
                return Err((
                    ErrorCode::MissingKey,
                    "missing key `length` to go with `angle`".to_owned(),
                    "length".to_owned(),
                ));
            }
            if !has_ang {
                return Err((
                    ErrorCode::MissingKey,
                    "missing key `angle` to go with `length`".to_owned(),
                    "angle".to_owned(),
                ));
            }
            let len = read_non_neg("length")?;
            let ang = match map.get("angle") {
                Some(Value::Number(n)) => n.as_f64(),
                _ => None,
            };
            let Some(angle_deg) = ang else {
                return Err((
                    ErrorCode::WrongType,
                    "expected `angle`, a number of degrees strictly between 0 and 180".to_owned(),
                    "angle".to_owned(),
                ));
            };
            if !angle_deg.is_finite() || angle_deg <= 0.0 || angle_deg >= 180.0 {
                return Err((
                    ErrorCode::BadValue,
                    "`angle` must be strictly between 0 and 180 degrees".to_owned(),
                    "angle".to_owned(),
                ));
            }
            Ok(ChamferSpec::LengthAngle {
                length: len,
                angle_deg,
            })
        }
        (false, false, false) => Err((
            ErrorCode::MissingKey,
            "give `distance`, `distance1` and `distance2`, or `length` and `angle`".to_owned(),
            "distance".to_owned(),
        )),
        _ => Err((
            ErrorCode::Usage,
            "give `distance`, `distance1` and `distance2`, or `length` and `angle` — not a mix"
                .to_owned(),
            "distance".to_owned(),
        )),
    }
}

/// Parses `sheet` or `template` for a template-scoped operation, defaulting to
/// `default_sheet` on an original DWG/DXF when omitted.
fn parse_template_or_sheet_target(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<AddTo, Error> {
    match (map.get("sheet"), map.get("template")) {
        (Some(Value::String(sheet)), None) => Ok(AddTo::Sheet(sheet.clone())),
        (None, Some(Value::String(template))) => Ok(AddTo::Template(template.clone())),
        (Some(_), Some(_)) => Err(fail(
            ErrorCode::Usage,
            "give sheet or template, not both: either names the template to edit".to_owned(),
            "",
        )),
        (None, None) => match default_sheet {
            Some(sheet) => Ok(AddTo::Sheet(sheet.to_owned())),
            None => Err(fail(
                ErrorCode::MissingKey,
                "missing key `template`: name the template to edit, or a sheet built from it with \
                 sheet"
                    .to_owned(),
                "template",
            )),
        },
        (Some(_), None) => Err(fail(
            ErrorCode::WrongType,
            "expected a sheet's name".to_owned(),
            "sheet",
        )),
        (None, Some(_)) => Err(fail(
            ErrorCode::WrongType,
            "expected a template's path".to_owned(),
            "template",
        )),
    }
}

#[inline(never)]
fn read_attach_xref_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the external reference's block name".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the external reference's block name".to_owned(),
                "name",
            ))
        }
    };
    let path = match map.get("path") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`path` cannot be empty".to_owned(),
                "path",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `path`: the path to the referenced DWG or DXF file".to_owned(),
                "path",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `path`, the path to the referenced DWG or DXF file".to_owned(),
                "path",
            ))
        }
    };
    let kind = match map.get("kind") {
        None | Some(Value::Null) => crate::model::XrefKind::Attach,
        Some(Value::String(s)) if s == "attach" => crate::model::XrefKind::Attach,
        Some(Value::String(s)) if s == "overlay" => crate::model::XrefKind::Overlay,
        Some(Value::String(other)) => {
            return Err(fail(
                ErrorCode::BadValue,
                format!("unknown xref kind `{other}`; expected `attach` or `overlay`"),
                "kind",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `kind`: `attach` or `overlay`".to_owned(),
                "kind",
            ))
        }
    };
    let unloaded = match map.get("unloaded") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `unloaded`, a boolean".to_owned(),
                "unloaded",
            ))
        }
    };
    let place = match map.get("place") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `place`, a boolean".to_owned(),
                "place",
            ))
        }
    };
    let position = match map.get("position") {
        None | Some(Value::Null) => json!([0, 0, 0]),
        Some(v @ Value::Array(items)) if items.len() == 3 => v.clone(),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `position`, a 3D point `[x, y, z]`".to_owned(),
                "position",
            ))
        }
    };
    let scale = match map.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(Value::Number(n)) => {
            let s = n.as_f64().unwrap_or(0.0);
            if !s.is_finite() || s <= 0.0 {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`scale` must be a finite number greater than 0".to_owned(),
                    "scale",
                ));
            }
            s
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `scale`, a positive number".to_owned(),
                "scale",
            ))
        }
    };
    let rotation = match map.get("rotation") {
        None | Some(Value::Null) => 0.0,
        Some(Value::Number(n)) => {
            let r = n.as_f64().unwrap_or(f64::NAN);
            if !r.is_finite() {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`rotation` must be a finite angle in degrees".to_owned(),
                    "rotation",
                ));
            }
            r
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `rotation`, a number of degrees".to_owned(),
                "rotation",
            ))
        }
    };
    let layer = match map.get("layer") {
        None | Some(Value::Null) => crate::model::LAYER_ZERO.to_owned(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `layer`, a non-empty layer name".to_owned(),
                "layer",
            ))
        }
    };
    let space = match map.get("space") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `space`, a non-empty space name".to_owned(),
                "space",
            ))
        }
    };
    let id = match map.get("id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `id`, a non-empty object ID".to_owned(),
                "id",
            ))
        }
    };
    let insert_name = match map.get("insert_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `insert_name`, a non-empty object name".to_owned(),
                "insert_name",
            ))
        }
    };
    Ok(Body::AttachXref {
        to,
        xref: crate::model::Xref {
            name,
            path,
            kind,
            unloaded,
        },
        place,
        position,
        scale,
        rotation,
        layer,
        space,
        id,
        insert_name,
    })
}

#[inline(never)]
fn read_detach_xref_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the external reference to detach".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the external reference to detach".to_owned(),
                "name",
            ))
        }
    };
    Ok(Body::DetachXref { to, name })
}

#[inline(never)]
fn read_bind_xref_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the external reference to bind".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the external reference to bind".to_owned(),
                "name",
            ))
        }
    };
    let mode = match map.get("mode") {
        None | Some(Value::Null) => crate::model::XrefBindMode::Bind,
        Some(Value::String(s)) if s == "bind" => crate::model::XrefBindMode::Bind,
        Some(Value::String(s)) if s == "insert" => crate::model::XrefBindMode::Insert,
        Some(Value::String(other)) => {
            return Err(fail(
                ErrorCode::BadValue,
                format!("unknown bind mode `{other}`; expected `bind` or `insert`"),
                "mode",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `mode`: `bind` or `insert`".to_owned(),
                "mode",
            ))
        }
    };
    Ok(Body::BindXref { to, name, mode })
}

#[inline(never)]
fn read_target_array(
    value: &Value,
    field: &str,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Vec<(Target, String)>, Error> {
    let Value::Array(items) = value else {
        return Err(fail(
            ErrorCode::WrongType,
            format!("expected `{field}`, a list of targets"),
            field,
        ));
    };
    if items.is_empty() {
        return Err(fail(
            ErrorCode::BadValue,
            format!("`{field}` must list at least one object"),
            field,
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let key = format!("{field}[{i}]");
        let mut t = read_target(item).map_err(|(code, msg, sub)| {
            let sub = sub.strip_prefix("target").unwrap_or(&sub);
            fail(code, msg, &format!("{key}{sub}"))
        })?;
        if let Some(ds) = default_sheet {
            match &mut t {
                Target::Name { sheet, .. } | Target::Group { sheet, .. } if sheet.is_none() => {
                    *sheet = Some(ds.to_owned());
                }
                _ => {}
            }
        }
        out.push((t, key));
    }
    Ok(out)
}

#[inline(never)]
fn read_add_group_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the group's name".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the group's name".to_owned(),
                "name",
            ))
        }
    };
    let targets = match map.get("targets") {
        Some(v) => read_target_array(v, "targets", default_sheet, fail)?,
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `targets`: the member objects to put in the group".to_owned(),
                "targets",
            ))
        }
    };
    let description = match map.get("description") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected text under `description`".to_owned(),
                "description",
            ))
        }
    };
    let selectable = match map.get("selectable") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected true or false under `selectable`".to_owned(),
                "selectable",
            ))
        }
    };
    Ok(Body::AddGroup {
        to,
        name,
        targets,
        description,
        selectable,
    })
}

#[inline(never)]
fn read_ungroup_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the group to remove".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the group to remove".to_owned(),
                "name",
            ))
        }
    };
    Ok(Body::Ungroup { to, name })
}

#[inline(never)]
fn read_edit_group_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`name` cannot be empty".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`: the group to edit".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected `name`, the group to edit".to_owned(),
                "name",
            ))
        }
    };
    let add = match map.get("add") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => read_target_array(v, "add", default_sheet, fail)?,
    };
    let remove = match map.get("remove") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => read_target_array(v, "remove", default_sheet, fail)?,
    };
    let rename = match map.get("rename") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`rename` cannot be empty".to_owned(),
                "rename",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected text under `rename`".to_owned(),
                "rename",
            ))
        }
    };
    let description = match map.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected text under `description`".to_owned(),
                "description",
            ))
        }
    };
    let selectable = match map.get("selectable") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected true or false under `selectable`".to_owned(),
                "selectable",
            ))
        }
    };
    if add.is_empty()
        && remove.is_empty()
        && rename.is_none()
        && description.is_none()
        && selectable.is_none()
    {
        return Err(fail(
            ErrorCode::MissingKey,
            "edit_group needs at least one of `add`, `remove`, `rename`, `description`, or \
             `selectable`"
                .to_owned(),
            "",
        ));
    }
    Ok(Body::EditGroup {
        to,
        name,
        add,
        remove,
        rename,
        description,
        selectable,
    })
}

#[inline(never)]
fn read_scale_name_array(
    value: &Value,
    field: &str,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Vec<String>, Error> {
    let Value::Array(items) = value else {
        return Err(fail(
            ErrorCode::WrongType,
            format!("expected `{field}`, a list of annotation scale names"),
            field,
        ));
    };
    if items.is_empty() {
        return Err(fail(
            ErrorCode::BadValue,
            format!("`{field}` must list at least one annotation scale name"),
            field,
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let key = format!("{field}[{i}]");
        match item {
            Value::String(s) if !s.trim().is_empty() => out.push(s.trim().to_owned()),
            Value::String(_) => {
                return Err(fail(
                    ErrorCode::BadValue,
                    "an annotation scale name cannot be empty".to_owned(),
                    &key,
                ));
            }
            _ => {
                return Err(fail(
                    ErrorCode::WrongType,
                    "expected an annotation scale name such as `1:50`".to_owned(),
                    &key,
                ));
            }
        }
    }
    Ok(out)
}

#[inline(never)]
fn read_set_annotation_scale_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let scale = match map.get("scale") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`scale` cannot be empty".to_owned(),
                "scale",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected an annotation scale name under `scale`".to_owned(),
                "scale",
            ))
        }
    };
    let anno_all_visible = match map.get("anno_all_visible") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected true or false under `anno_all_visible`".to_owned(),
                "anno_all_visible",
            ))
        }
    };
    let mut add_scales = std::collections::BTreeMap::new();
    match map.get("add_scales") {
        None | Some(Value::Null) => {}
        Some(Value::Object(entries)) if !entries.is_empty() => {
            for (name, val) in entries {
                let trimmed = name.trim();
                let key = format!("add_scales.{name}");
                if trimmed.is_empty() || trimmed != name || trimmed.chars().any(char::is_control) {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "an annotation scale name cannot be empty, start or end with a space, or \
                         hold control characters"
                            .to_owned(),
                        &key,
                    ));
                }
                let Value::Array(pair) = val else {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected [paper_units, drawing_units], two positive numbers".to_owned(),
                        &key,
                    ));
                };
                if pair.len() != 2 {
                    return Err(fail(
                        ErrorCode::BadValue,
                        format!(
                            "an annotation scale takes [paper_units, drawing_units] (2 numbers); \
                             found {}",
                            pair.len()
                        ),
                        &key,
                    ));
                }
                let p = pair[0].as_f64().unwrap_or(f64::NAN);
                let d = pair[1].as_f64().unwrap_or(f64::NAN);
                if !p.is_finite() || p <= 0.0 || !d.is_finite() || d <= 0.0 {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "paper_units and drawing_units must be finite numbers greater than 0"
                            .to_owned(),
                        &key,
                    ));
                }
                add_scales.insert(name.clone(), [p, d]);
            }
        }
        Some(Value::Object(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "`add_scales` must name at least one scale".to_owned(),
                "add_scales",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a mapping from scale name to [paper_units, drawing_units] under \
                 `add_scales`"
                    .to_owned(),
                "add_scales",
            ))
        }
    }
    let remove_scales = match map.get("remove_scales") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => read_scale_name_array(v, "remove_scales", fail)?,
    };
    if scale.is_none()
        && anno_all_visible.is_none()
        && add_scales.is_empty()
        && remove_scales.is_empty()
    {
        return Err(fail(
            ErrorCode::MissingKey,
            "set_annotation_scale needs at least one of `scale`, `anno_all_visible`, `add_scales`, \
             or `remove_scales`"
                .to_owned(),
            "",
        ));
    }
    Ok(Body::SetAnnotationScale {
        to,
        scale,
        anno_all_visible,
        add_scales,
        remove_scales,
    })
}

#[inline(never)]
fn read_set_point_style_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let pdmode = match map.get("pdmode") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => {
            let Some(v) = n.as_i64().and_then(|v| i16::try_from(v).ok()) else {
                return Err(fail(
                    ErrorCode::BadValue,
                    "expected an integer AutoCAD PDMODE code (0..4, 32..36, 64..68, or 96..100)"
                        .to_owned(),
                    "pdmode",
                ));
            };
            if !crate::model::is_valid_pdmode(v) {
                return Err(fail(
                    ErrorCode::BadValue,
                    format!(
                        "`pdmode` {v} is not a valid AutoCAD PDMODE (expected base 0..4 plus \
                         optional 32 circle and/or 64 square)"
                    ),
                    "pdmode",
                ));
            }
            Some(v)
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected an integer under `pdmode`".to_owned(),
                "pdmode",
            ))
        }
    };
    let pdsize = match map.get("pdsize") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => {
            let v = n.as_f64().unwrap_or(f64::NAN);
            if !v.is_finite() {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`pdsize` must be a finite number".to_owned(),
                    "pdsize",
                ));
            }
            Some(v)
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a number under `pdsize`".to_owned(),
                "pdsize",
            ))
        }
    };
    if pdmode.is_none() && pdsize.is_none() {
        return Err(fail(
            ErrorCode::MissingKey,
            "set_point_style needs at least one of `pdmode` or `pdsize`".to_owned(),
            "",
        ));
    }
    Ok(Body::SetPointStyle { to, pdmode, pdsize })
}

#[inline(never)]
fn read_add_mline_style_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let to = parse_template_or_sheet_target(map, default_sheet, fail)?;
    let name = match map.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        Some(Value::String(_)) => {
            return Err(fail(
                ErrorCode::BadValue,
                "a multiline style name cannot be empty".to_owned(),
                "name",
            ))
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a multiline style name under `name`".to_owned(),
                "name",
            ))
        }
        None => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing key `name`, the multiline style's name".to_owned(),
                "name",
            ))
        }
    };
    let mut style = Map::new();
    for (k, v) in map {
        if matches!(k.as_str(), "op" | "sheet" | "template" | "name") {
            continue;
        }
        style.insert(k.clone(), v.clone());
    }
    Ok(Body::AddMLineStyle { to, name, style })
}

#[inline(never)]
fn read_add_solid_shortcut_to(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<AddTo, Error> {
    match (map.get("sheet"), map.get("template")) {
        (Some(Value::String(sheet)), None) => Ok(AddTo::Sheet(sheet.clone())),
        (None, Some(Value::String(template))) => Ok(AddTo::Template(template.clone())),
        (Some(_), Some(_)) => Err(fail(
            ErrorCode::Usage,
            "give sheet or template, not both: an object added to a sheet goes in \
             its row, and one added to a template is on every sheet built from it"
                .to_owned(),
            "",
        )),
        (None, None) => match default_sheet {
            Some(sheet) => Ok(AddTo::Sheet(sheet.to_owned())),
            None => Err(fail(
                ErrorCode::MissingKey,
                "missing key `sheet`: name the sheet to add it to, in its row, or \
                 give template to add it to a template"
                    .to_owned(),
                "sheet",
            )),
        },
        (Some(_), None) => Err(fail(
            ErrorCode::WrongType,
            "expected a sheet's name".to_owned(),
            "sheet",
        )),
        (None, Some(_)) => Err(fail(
            ErrorCode::WrongType,
            "expected a template's path".to_owned(),
            "template",
        )),
    }
}

#[inline(never)]
fn read_add_box_body(
    map: &Map<String, Value>,
    defs: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let draw_order = match map.get("draw_order") {
        None => None,
        Some(v) => Some(
            read_draw_order_place(v, "draw_order")
                .map_err(|(code, message, field)| fail(code, message, &field))?,
        ),
    };
    if !map.contains_key("origin") {
        return Err(fail(
            ErrorCode::MissingKey,
            "missing key `origin`; a 3D box needs `origin` and `size` (or `length`, `width` and `height`)"
                .to_owned(),
            "origin",
        ));
    }
    let has_size = map.contains_key("size");
    let has_lwh =
        map.contains_key("length") || map.contains_key("width") || map.contains_key("height");
    if has_size && has_lwh {
        return Err(fail(
            ErrorCode::Usage,
            "give `size` or `length`/`width`/`height`, not both".to_owned(),
            "size",
        ));
    }
    let size_value = if let Some(sz) = map.get("size") {
        sz.clone()
    } else {
        let l = map.get("length").ok_or_else(|| {
            fail(
                ErrorCode::MissingKey,
                "missing key `size` (or `length`, `width` and `height`)".to_owned(),
                "length",
            )
        })?;
        let w = map.get("width").ok_or_else(|| {
            fail(
                ErrorCode::MissingKey,
                "missing key `width` (give `length`, `width` and `height`, or `size`)".to_owned(),
                "width",
            )
        })?;
        let h = map.get("height").ok_or_else(|| {
            fail(
                ErrorCode::MissingKey,
                "missing key `height` (give `length`, `width` and `height`, or `size`)".to_owned(),
                "height",
            )
        })?;
        Value::Array(vec![l.clone(), w.clone(), h.clone()])
    };
    let to = read_add_solid_shortcut_to(map, default_sheet, fail)?;
    let mut keys = Map::new();
    keys.insert("kind".into(), Value::String("box".to_owned()));
    keys.insert("origin".into(), map["origin"].clone());
    keys.insert("size".into(), size_value);
    for (k, v) in map {
        if matches!(
            k.as_str(),
            OP_KEY
                | "sheet"
                | "template"
                | "draw_order"
                | "origin"
                | "size"
                | "length"
                | "width"
                | "height"
        ) {
            continue;
        }
        keys.insert(k.clone(), v.clone());
    }
    let schema = defs.get("op_solid3d").unwrap_or(&defs["solid3d"]);
    let mut snaps = Vec::new();
    for (key, value) in &keys {
        let slots: Vec<&Value> = schema["properties"].get(key).into_iter().collect();
        snaps_in(value, &slots, defs, &format!("/{key}"), key, &mut snaps)
            .map_err(|(code, message, field)| fail(code, message, &field))?;
    }
    Ok(Body::Add {
        to,
        keys,
        snaps,
        draw_order,
    })
}

#[inline(never)]
fn read_add_cylinder_body(
    map: &Map<String, Value>,
    defs: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let draw_order = match map.get("draw_order") {
        None => None,
        Some(v) => Some(
            read_draw_order_place(v, "draw_order")
                .map_err(|(code, message, field)| fail(code, message, &field))?,
        ),
    };
    for req in ["base", "radius", "height"] {
        if !map.contains_key(req) {
            return Err(fail(
                ErrorCode::MissingKey,
                format!("missing key `{req}`; a 3D cylinder needs `base`, `radius` and `height`"),
                req,
            ));
        }
    }
    let to = read_add_solid_shortcut_to(map, default_sheet, fail)?;
    let mut keys = Map::new();
    keys.insert("kind".into(), Value::String("cylinder".to_owned()));
    for (k, v) in map {
        if matches!(k.as_str(), OP_KEY | "sheet" | "template" | "draw_order") {
            continue;
        }
        keys.insert(k.clone(), v.clone());
    }
    let schema = defs.get("op_solid3d").unwrap_or(&defs["solid3d"]);
    let mut snaps = Vec::new();
    for (key, value) in &keys {
        let slots: Vec<&Value> = schema["properties"].get(key).into_iter().collect();
        snaps_in(value, &slots, defs, &format!("/{key}"), key, &mut snaps)
            .map_err(|(code, message, field)| fail(code, message, &field))?;
    }
    Ok(Body::Add {
        to,
        keys,
        snaps,
        draw_order,
    })
}

#[inline(never)]
fn read_optional_nonempty_str(
    map: &Map<String, Value>,
    key: &str,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Option<String>, Error> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.clone())),
        Some(Value::String(_)) => Err(fail(
            ErrorCode::BadValue,
            format!("`{key}` cannot be empty"),
            key,
        )),
        Some(_) => Err(fail(
            ErrorCode::WrongType,
            format!("expected a non-empty string under `{key}`"),
            key,
        )),
    }
}

#[inline(never)]
fn read_extrude_body(
    map: &Map<String, Value>,
    units: Unit,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let target_val = map.get("target").ok_or_else(|| {
        fail(
            ErrorCode::MissingKey,
            "missing key `target`, the closed polyline, circle, or ellipse to extrude".to_owned(),
            "target",
        )
    })?;
    let target = read_target(target_val).map_err(|(c, m, f)| fail(c, m, &f))?;
    let height_val = map.get("height").ok_or_else(|| {
        fail(
            ErrorCode::MissingKey,
            "missing key `height`, the non-zero extrusion height".to_owned(),
            "height",
        )
    })?;
    let height = length(height_val, units).map_err(|m| fail(ErrorCode::BadNumber, m, "height"))?;
    if height.abs() <= 1e-9 {
        return Err(fail(
            ErrorCode::BadValue,
            "`height` must be non-zero".to_owned(),
            "height",
        ));
    }
    let direction = match map.get("direction") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let d = distance(v, units, "direction").map_err(|(c, m, f)| fail(c, m, &f))?;
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if !len.is_finite() || len <= 1e-12 {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`direction` cannot be [0, 0, 0]".to_owned(),
                    "direction",
                ));
            }
            Some(Point3::new(d[0], d[1], d[2]))
        }
    };
    let taper_angle = match map.get("taper_angle") {
        None | Some(Value::Null) => 0.0,
        Some(Value::Number(n)) => {
            let a = n.as_f64().ok_or_else(|| {
                fail(
                    ErrorCode::BadNumber,
                    "expected a finite number under `taper_angle`".to_owned(),
                    "taper_angle",
                )
            })?;
            if !a.is_finite() || !(-85.0..=85.0).contains(&a) {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`taper_angle` must be between -85 and 85 degrees".to_owned(),
                    "taper_angle",
                ));
            }
            a
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a number of degrees under `taper_angle`".to_owned(),
                "taper_angle",
            ))
        }
    };
    let erase_source = match map.get("erase_source") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a boolean under `erase_source`".to_owned(),
                "erase_source",
            ))
        }
    };
    let layer = read_optional_nonempty_str(map, "layer", fail)?;
    let id = read_optional_nonempty_str(map, "id", fail)?;
    let name = read_optional_nonempty_str(map, "name", fail)?;
    Ok(Body::Extrude {
        target,
        height,
        direction,
        taper_angle,
        erase_source,
        layer,
        id,
        name,
    })
}

#[inline(never)]
fn read_revolve_body(
    map: &Map<String, Value>,
    units: Unit,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let target_val = map.get("target").ok_or_else(|| {
        fail(
            ErrorCode::MissingKey,
            "missing key `target`, the closed polyline, circle, or ellipse to revolve".to_owned(),
            "target",
        )
    })?;
    let target = read_target(target_val).map_err(|(c, m, f)| fail(c, m, &f))?;
    let origin_val = map.get("axis_origin").ok_or_else(|| {
        fail(
            ErrorCode::MissingKey,
            "missing key `axis_origin`, a point on the axis of revolution".to_owned(),
            "axis_origin",
        )
    })?;
    let axis_origin =
        op_point(origin_val, units, "axis_origin").map_err(|(c, m, f)| fail(c, m, &f))?;
    let dir_val = map.get("axis_direction").ok_or_else(|| {
        fail(
            ErrorCode::MissingKey,
            "missing key `axis_direction`, the direction vector of the axis of revolution"
                .to_owned(),
            "axis_direction",
        )
    })?;
    let d = distance(dir_val, units, "axis_direction").map_err(|(c, m, f)| fail(c, m, &f))?;
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if !len.is_finite() || len <= 1e-12 {
        return Err(fail(
            ErrorCode::BadValue,
            "`axis_direction` cannot be [0, 0, 0]".to_owned(),
            "axis_direction",
        ));
    }
    let axis_direction = Point3::new(d[0], d[1], d[2]);
    let angle = match map.get("angle") {
        None | Some(Value::Null) => 360.0,
        Some(Value::Number(n)) => {
            let a = n.as_f64().ok_or_else(|| {
                fail(
                    ErrorCode::BadNumber,
                    "expected a finite number under `angle`".to_owned(),
                    "angle",
                )
            })?;
            if !a.is_finite() || a <= 0.0 || a > 360.0 {
                return Err(fail(
                    ErrorCode::BadValue,
                    "`angle` must be more than 0 and at most 360 degrees".to_owned(),
                    "angle",
                ));
            }
            a
        }
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a number of degrees under `angle`".to_owned(),
                "angle",
            ))
        }
    };
    let erase_source = match map.get("erase_source") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected a boolean under `erase_source`".to_owned(),
                "erase_source",
            ))
        }
    };
    let layer = read_optional_nonempty_str(map, "layer", fail)?;
    let id = read_optional_nonempty_str(map, "id", fail)?;
    let name = read_optional_nonempty_str(map, "name", fail)?;
    Ok(Body::Revolve {
        target,
        axis_origin,
        axis_direction,
        angle,
        erase_source,
        layer,
        id,
        name,
    })
}

#[inline(never)]
fn read_solid_boolean_body(
    op: BooleanOp,
    map: &Map<String, Value>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let targets = match op {
        BooleanOp::Union | BooleanOp::Intersect => match map.get("targets") {
            Some(Value::Array(items)) if items.len() >= 2 => items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let at = format!("targets[{i}]");
                    read_target(item).map(|t| (t, at.clone())).map_err(|e| {
                        let (c, m, f) = target_at(&at, e);
                        fail(c, m, &f)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Array(_)) => {
                return Err(fail(
                    ErrorCode::BadValue,
                    format!("`{}` requires at least 2 targets", op.name()),
                    "targets",
                ))
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::WrongType,
                    "expected a list of at least 2 target 3D solids under `targets`".to_owned(),
                    "targets",
                ))
            }
            None => {
                return Err(fail(
                    ErrorCode::MissingKey,
                    format!(
                        "missing key `targets`, at least 2 3D solids to {}",
                        op.name()
                    ),
                    "targets",
                ))
            }
        },
        BooleanOp::Subtract => {
            let base_val = map.get("target").ok_or_else(|| {
                fail(
                    ErrorCode::MissingKey,
                    "missing key `target`, the base 3D solid to subtract from".to_owned(),
                    "target",
                )
            })?;
            let base = read_target(base_val).map_err(|(c, m, f)| fail(c, m, &f))?;
            let sub_val = map.get("subtract").ok_or_else(|| {
                fail(
                    ErrorCode::MissingKey,
                    "missing key `subtract`, one or more 3D solids to subtract from `target`"
                        .to_owned(),
                    "subtract",
                )
            })?;
            let mut list = vec![(base, "target".to_owned())];
            match sub_val {
                Value::Array(items) if !items.is_empty() => {
                    for (i, item) in items.iter().enumerate() {
                        let at = format!("subtract[{i}]");
                        let t = read_target(item).map_err(|e| {
                            let (c, m, f) = target_at(&at, e);
                            fail(c, m, &f)
                        })?;
                        list.push((t, at));
                    }
                }
                Value::Array(_) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`subtract` must name at least one 3D solid".to_owned(),
                        "subtract",
                    ));
                }
                other => {
                    let t = read_target(other).map_err(|e| {
                        let (c, m, f) = target_at("subtract", e);
                        fail(c, m, &f)
                    })?;
                    list.push((t, "subtract".to_owned()));
                }
            }
            list
        }
    };
    let layer = read_optional_nonempty_str(map, "layer", fail)?;
    let id = read_optional_nonempty_str(map, "id", fail)?;
    let name = read_optional_nonempty_str(map, "name", fail)?;
    Ok(Body::SolidBoolean {
        op,
        targets,
        layer,
        id,
        name,
    })
}

#[inline(never)]
fn read_set_object_scales_body(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
    fail: &dyn Fn(ErrorCode, String, &str) -> Error,
) -> Result<Body, Error> {
    let targets = match (map.get("target"), map.get("targets")) {
        (Some(_), Some(_)) => {
            return Err(fail(
                ErrorCode::Usage,
                "give `target` or `targets`, not both".to_owned(),
                "",
            ))
        }
        (None, None) => {
            return Err(fail(
                ErrorCode::MissingKey,
                "missing `target` (or `targets`): the annotative object(s) to update".to_owned(),
                "target",
            ))
        }
        (Some(one), None) => {
            let mut t = read_target(one).map_err(|(code, msg, sub)| {
                let field = if sub.is_empty() {
                    "target".to_owned()
                } else if sub.starts_with("target") {
                    sub
                } else {
                    format!("target.{sub}")
                };
                fail(code, msg, &field)
            })?;
            if let Some(ds) = default_sheet {
                match &mut t {
                    Target::Name { sheet, .. } | Target::Group { sheet, .. } if sheet.is_none() => {
                        *sheet = Some(ds.to_owned());
                    }
                    _ => {}
                }
            }
            vec![(t, "target".to_owned())]
        }
        (None, Some(many)) => read_target_array(many, "targets", default_sheet, fail)?,
    };
    let annotative = match map.get("annotative") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(fail(
                ErrorCode::WrongType,
                "expected true or false under `annotative`".to_owned(),
                "annotative",
            ))
        }
    };
    let scales = match map.get("scales") {
        None | Some(Value::Null) => None,
        Some(v) => Some(read_scale_name_array(v, "scales", fail)?),
    };
    let add = match map.get("add") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => read_scale_name_array(v, "add", fail)?,
    };
    let remove = match map.get("remove") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => read_scale_name_array(v, "remove", fail)?,
    };
    if scales.is_some() && (!add.is_empty() || !remove.is_empty()) {
        return Err(fail(
            ErrorCode::Usage,
            "give `scales` (to replace the list) or `add`/`remove` (to edit it), not both"
                .to_owned(),
            "scales",
        ));
    }
    if annotative == Some(false) && (scales.is_some() || !add.is_empty()) {
        return Err(fail(
            ErrorCode::BadValue,
            "cannot add or assign `scales` while turning `annotative: false`".to_owned(),
            "annotative",
        ));
    }
    if annotative.is_none() && scales.is_none() && add.is_empty() && remove.is_empty() {
        return Err(fail(
            ErrorCode::MissingKey,
            "set_object_scales needs at least one of `annotative`, `scales`, `add`, or `remove`"
                .to_owned(),
            "",
        ));
    }
    Ok(Body::SetObjectScales {
        targets,
        annotative,
        scales,
        add,
        remove,
    })
}

/// Parses `boundary` (one target) or `boundaries` (a non-empty list of targets)
/// for `trim` and `extend`.
fn read_boundary_targets(
    map: &Map<String, Value>,
    default_sheet: Option<&str>,
) -> Result<Vec<(Target, String)>, (ErrorCode, String, String)> {
    let parse_one = |val: &Value, prefix: &str| -> Result<Target, (ErrorCode, String, String)> {
        let mut t = read_target(val).map_err(|(code, msg, sub)| {
            let field = if sub.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}.{sub}")
            };
            (code, msg, field)
        })?;
        if let Some(sheet) = default_sheet {
            if let Target::Name {
                sheet: slot @ None, ..
            } = &mut t
            {
                *slot = Some(sheet.to_owned());
            }
        }
        Ok(t)
    };
    match (map.get("boundary"), map.get("boundaries")) {
        (Some(_), Some(_)) => Err((
            ErrorCode::Usage,
            "give `boundary` or `boundaries`, not both".to_owned(),
            "boundaries".to_owned(),
        )),
        (None, None) => Err((
            ErrorCode::MissingKey,
            "missing `boundary` (or `boundaries`), the cutting or boundary object".to_owned(),
            "boundary".to_owned(),
        )),
        (Some(one), None) => Ok(vec![(parse_one(one, "boundary")?, "boundary".to_owned())]),
        (None, Some(Value::Array(items))) if !items.is_empty() => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let field = format!("boundaries[{i}]");
                out.push((parse_one(item, &field)?, field));
            }
            Ok(out)
        }
        (None, Some(Value::Array(_))) => Err((
            ErrorCode::BadValue,
            "`boundaries` must name at least one boundary object".to_owned(),
            "boundaries".to_owned(),
        )),
        (None, Some(_)) => Err((
            ErrorCode::WrongType,
            "expected `boundaries`, a list of boundary targets".to_owned(),
            "boundaries".to_owned(),
        )),
    }
}

/// Parses an optional non-empty `sheets` list of sheet names on a batch
/// operation (`replace_text`, `set_attributes`).
fn read_optional_sheets(
    map: &Map<String, Value>,
) -> Result<Option<Vec<String>>, (ErrorCode, String, String)> {
    match map.get("sheets") {
        None => Ok(None),
        Some(Value::Array(items)) if !items.is_empty() => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::String(name) if !name.trim().is_empty() => out.push(name.clone()),
                    Value::String(_) => {
                        return Err((
                            ErrorCode::BadValue,
                            "a sheet name cannot be empty".to_owned(),
                            format!("sheets[{i}]"),
                        ))
                    }
                    _ => {
                        return Err((
                            ErrorCode::WrongType,
                            "expected a sheet's name".to_owned(),
                            format!("sheets[{i}]"),
                        ))
                    }
                }
            }
            Ok(Some(out))
        }
        Some(Value::Array(_)) => Err((
            ErrorCode::BadValue,
            "`sheets` must name at least one sheet, or be left out for the whole set".to_owned(),
            "sheets".to_owned(),
        )),
        Some(_) => Err((
            ErrorCode::WrongType,
            "expected `sheets`, a list of sheet names".to_owned(),
            "sheets".to_owned(),
        )),
    }
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
fn table_schema(
    op: TableOp,
    table: Table,
    defs: &Map<String, Value>,
    allow_omitted_sheet: bool,
) -> Value {
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
    if allow_omitted_sheet && matches!((op, table), (TableOp::Add | TableOp::Change, Table::Layer))
    {
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

/// The side an `offset` operation offsets to, before snapped points are resolved.
#[derive(Clone, Debug)]
pub(crate) enum OpOffsetSide {
    Left,
    Right,
    Inside,
    Outside,
    Point(OpPoint),
}

/// How an `offset` operation gives its distance and side.
#[derive(Clone, Debug)]
pub(crate) enum OpOffsetBy {
    Distance { distance: f64, side: OpOffsetSide },
    Through(OpPoint),
}

#[derive(Clone, Debug)]
pub(crate) enum Body {
    Add {
        to: AddTo,
        keys: Map<String, Value>,
        /// The snapped points its keys hold, to be written as coordinates.
        snaps: Vec<PointSnap>,
        /// Where in draw order to place the added object; `None` appends at
        /// the front.
        draw_order: Option<DrawOrderPlace>,
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
    /// Moves `targets` in draw order to `order`: AutoCAD's DRAWORDER.
    DrawOrder {
        targets: Vec<(Target, String)>,
        order: DrawOrderPlace,
    },
    /// Brings text, dimensions, leaders, or all of them to the front of draw
    /// order: AutoCAD's TEXTTOFRONT.
    TextToFront {
        to: AddTo,
        which: TextToFrontWhich,
    },
    SetAttribute {
        target: Target,
        tag: String,
        /// Which of the attributes with the tag, counted from 1, when the
        /// block defines it more than once.
        occurrence: Option<usize>,
        value: String,
    },
    /// Sets one or more dynamic block parameter values on an insert: AutoCAD's
    /// PROPERTIES.
    SetBlockState {
        target: Target,
        state: Map<String, Value>,
    },
    /// Attaches, replaces, or clears an AutoCAD field on a text, mtext, or
    /// insert attribute: AutoCAD's FIELD.
    SetField {
        target: Target,
        tag: Option<String>,
        occurrence: Option<usize>,
        field: Option<crate::model::TextField>,
        value: Option<String>,
    },
    /// Sets a sheet's or layout's page setup: AutoCAD's PAGESETUP.
    SetPageSetup {
        to: AddTo,
        layout: Option<String>,
        set: Map<String, Value>,
    },
    /// Attaches or overlays an external reference (`XATTACH` / `XREF`) and
    /// optionally places an insert of it.
    AttachXref {
        to: AddTo,
        xref: crate::model::Xref,
        place: bool,
        position: Value,
        scale: f64,
        rotation: f64,
        layer: String,
        space: Option<String>,
        id: Option<String>,
        insert_name: Option<String>,
    },
    /// Detaches an external reference and removes every insert placing it.
    DetachXref {
        to: AddTo,
        name: String,
    },
    /// Binds an external reference into a local block definition (`XBIND`).
    BindXref {
        to: AddTo,
        name: String,
        mode: crate::model::XrefBindMode,
    },
    /// Sets an attribute on every insert matching a query selection: AutoCAD's
    /// GATTE.
    SetAttributes {
        selection: Selection,
        sheets: Option<Vec<String>>,
        tag: String,
        occurrence: Option<usize>,
        value: String,
    },
    /// Replaces text across texts, mtexts and attribute values: AutoCAD's FIND.
    ReplaceText {
        find: String,
        replace: String,
        matching: TextMatchKind,
        case_sensitive: bool,
        sheets: Option<Vec<String>>,
        scopes: Vec<TextScope>,
        tag: Option<String>,
        block: Option<String>,
    },
    /// Draws a parallel offset of a curve object: AutoCAD's OFFSET.
    Offset {
        target: Target,
        by: OpOffsetBy,
        id: Option<String>,
        name: Option<String>,
        layer: Option<String>,
        erase_source: bool,
    },
    /// Cuts `target` back to one or more `boundaries`: AutoCAD's TRIM.
    Trim {
        target: Target,
        boundaries: Vec<(Target, String)>,
        keep: TrimKeep,
        new_id: Option<String>,
    },
    /// Carries an open `target` on to one or more `boundaries`: AutoCAD's EXTEND.
    Extend {
        target: Target,
        boundaries: Vec<(Target, String)>,
        end: ExtendEnd,
    },
    /// Rounds the corner between two objects or every corner of a polyline:
    /// AutoCAD's FILLET.
    Fillet {
        mode: CornerMode,
        radius: f64,
    },
    /// Bevels the corner between two lines or every corner of a polyline:
    /// AutoCAD's CHAMFER.
    Chamfer {
        mode: CornerMode,
        spec: ChamferSpec,
    },
    /// Cuts a gap in `target` or splits it at a point: AutoCAD's BREAK /
    /// BREAKATPOINT.
    Break {
        target: Target,
        by: OpBreakBy,
        new_id: Option<String>,
    },
    /// Joins lines, arcs, and open polylines into one object: AutoCAD's JOIN.
    Join {
        targets: Vec<(Target, String)>,
        tolerance: f64,
    },
    /// Moves vertices and objects inside `box` by `motion`: AutoCAD's STRETCH.
    Stretch {
        to: AddTo,
        space: Option<String>,
        targets: Option<Vec<(Target, String)>>,
        corners: [Box<OpPoint>; 2],
        motion: Motion,
    },
    /// Explodes compound objects into the simpler objects that draw them:
    /// AutoCAD's EXPLODE.
    Explode {
        targets: Vec<(Target, String)>,
        id_prefix: Option<String>,
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
    /// A revision cloud added round one or more existing objects' extents.
    AddRevisionCloud {
        to: Option<AddTo>,
        around: Vec<(Target, String)>,
        margin: Option<Value>,
        keys: Map<String, Value>,
        draw_order: Option<DrawOrderPlace>,
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
    /// Creates a named group (`GROUP` / `G`).
    AddGroup {
        to: AddTo,
        name: String,
        targets: Vec<(Target, String)>,
        description: String,
        selectable: bool,
    },
    /// Removes a named group without deleting its members (`UNGROUP`).
    Ungroup {
        to: AddTo,
        name: String,
    },
    /// Edits a named group (`GROUPEDIT`).
    EditGroup {
        to: AddTo,
        name: String,
        add: Vec<(Target, String)>,
        remove: Vec<(Target, String)>,
        rename: Option<String>,
        description: Option<String>,
        selectable: Option<bool>,
    },
    /// Sets the active annotation scale (`CANNOSCALE`), `ANNOALLVISIBLE`, or
    /// edits `annotation_scales` (`SCALELISTEDIT`).
    SetAnnotationScale {
        to: AddTo,
        scale: Option<String>,
        anno_all_visible: Option<bool>,
        add_scales: std::collections::BTreeMap<String, [f64; 2]>,
        remove_scales: Vec<String>,
    },
    /// Adds, removes, or replaces supported annotation scales on annotative
    /// objects, or toggles their `annotative` flag (`OBJECTSCALE` / `ANNOUPDATE`).
    SetObjectScales {
        targets: Vec<(Target, String)>,
        annotative: Option<bool>,
        scales: Option<Vec<String>>,
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// Sets drawing-wide point display style (`PDMODE` / `PDSIZE`, AutoCAD `DDPTYPE`).
    SetPointStyle {
        to: AddTo,
        pdmode: Option<i16>,
        pdsize: Option<f64>,
    },
    /// Adds or updates a named multiline style (`mline_styles`, AutoCAD `MLSTYLE`).
    AddMLineStyle {
        to: AddTo,
        name: String,
        style: Map<String, Value>,
    },
    /// Extrudes a closed 2D profile into a 3D solid (AutoCAD `EXTRUDE` / `EXT`).
    Extrude {
        target: Target,
        height: f64,
        direction: Option<Point3>,
        taper_angle: f64,
        erase_source: bool,
        layer: Option<String>,
        id: Option<String>,
        name: Option<String>,
    },
    /// Revolves a closed 2D profile around a 3D axis into a 3D solid (AutoCAD `REVOLVE` / `REV`).
    Revolve {
        target: Target,
        axis_origin: OpPoint,
        axis_direction: Point3,
        angle: f64,
        erase_source: bool,
        layer: Option<String>,
        id: Option<String>,
        name: Option<String>,
    },
    /// Combines 3D solids by CSG boolean (`UNION`, `SUBTRACT`, `INTERSECT`).
    SolidBoolean {
        op: BooleanOp,
        targets: Vec<(Target, String)>,
        layer: Option<String>,
        id: Option<String>,
        name: Option<String>,
    },
}

/// How a `break` operation specifies its cut points, as parsed (may hold
/// snapped points).
#[derive(Clone, Debug)]
pub(crate) enum OpBreakBy {
    AtPoint {
        at: Box<OpPoint>,
        gap: Option<f64>,
    },
    Between {
        from: Box<OpPoint>,
        to: Box<OpPoint>,
    },
}

/// Target specification shared by `fillet` and `chamfer`: either a single flat
/// polyline (`target`) or two objects (`first` and `second`).
#[derive(Clone, Debug)]
pub(crate) enum CornerMode {
    Polyline {
        target: Target,
    },
    TwoObjects {
        first: Target,
        second: Target,
        trim: bool,
        first_end: Option<CornerEnd>,
        second_end: Option<CornerEnd>,
        new_id: Option<String>,
        layer: Option<String>,
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

/// Where an object is placed in draw order: at the front (drawn last, on top),
/// at the back (drawn first, behind), immediately above another object, or
/// immediately under another object.
#[derive(Clone, Debug)]
pub(crate) enum DrawOrderPlace {
    Front,
    Back,
    Above(Target),
    Under(Target),
}

impl DrawOrderPlace {
    pub(crate) fn words(&self) -> String {
        match self {
            DrawOrderPlace::Front => "to the front".to_owned(),
            DrawOrderPlace::Back => "to the back".to_owned(),
            DrawOrderPlace::Above(t) => format!("above {}", t.words()),
            DrawOrderPlace::Under(t) => format!("under {}", t.words()),
        }
    }
}

/// The kinds `text_to_front` accepts under `which`.
pub(crate) const TEXT_TO_FRONT_KINDS: [&str; 4] = ["text", "dimensions", "leaders", "all"];

/// Which objects `text_to_front` brings to the front: AutoCAD's TEXTTOFRONT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextToFrontWhich {
    Text,
    Dimensions,
    Leaders,
    All,
}

impl TextToFrontWhich {
    pub(crate) fn name(self) -> &'static str {
        match self {
            TextToFrontWhich::Text => "text",
            TextToFrontWhich::Dimensions => "dimensions",
            TextToFrontWhich::Leaders => "leaders",
            TextToFrontWhich::All => "all",
        }
    }

    pub(crate) fn matches_kind(self, kind: &str) -> bool {
        match self {
            TextToFrontWhich::Text => matches!(kind, "text" | "mtext"),
            TextToFrontWhich::Dimensions => kind == "dimension",
            TextToFrontWhich::Leaders => matches!(kind, "mleader" | "leader"),
            TextToFrontWhich::All => {
                matches!(kind, "text" | "mtext" | "dimension" | "mleader" | "leader")
            }
        }
    }

    pub(crate) fn matches_object(self, object: &crate::model::Object) -> bool {
        self.matches_kind(object.type_name())
    }
}

/// How an operation names the object (or group of objects) it edits.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// By its ID: the sheet's name and the ID in its file.
    Id { sheet: String, id: String },
    /// By its name, on the sheet given or on the one sheet it is on.
    Name { name: String, sheet: Option<String> },
    /// By a group's name (`GROUP`), targeting all of its member objects.
    Group {
        group: String,
        sheet: Option<String>,
    },
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
            Target::Group {
                group,
                sheet: Some(sheet),
            } => format!("group `{group}` on sheet `{sheet}`"),
            Target::Group { group, sheet: None } => format!("group `{group}`"),
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
            let mut allowed = vec!["sheet", "template", "draw_order"];
            allowed.extend(&own);
            if object == "revision_cloud" {
                allowed.push("around");
                allowed.push("margin");
            }
            keys(&allowed)?;
            let draw_order = match map.get("draw_order") {
                None => None,
                Some(v) => Some(
                    read_draw_order_place(v, "draw_order")
                        .map_err(|(code, message, field)| fail(code, message, &field))?,
                ),
            };
            if object == "revision_cloud" && map.contains_key("around") {
                if map.contains_key("rectangle") {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "give `rectangle`, `polygon` or `around`, not both `around` and \
                         `rectangle`"
                            .to_owned(),
                        "rectangle",
                    ));
                }
                if map.contains_key("polygon") {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "give `rectangle`, `polygon` or `around`, not both `around` and `polygon`"
                            .to_owned(),
                        "polygon",
                    ));
                }
                let around_val = map.get("around").expect("checked above");
                let around = match around_val {
                    Value::Array(items) if !items.is_empty() => items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let at = format!("around[{i}]");
                            read_target(item).map(|t| (t, at.clone())).map_err(
                                |(code, msg, field)| {
                                    let path = if field.is_empty() {
                                        at
                                    } else {
                                        format!("{at}.{field}")
                                    };
                                    fail(code, msg, &path)
                                },
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    Value::Array(_) => {
                        return Err(fail(
                            ErrorCode::BadValue,
                            "`around` must name at least one object".to_owned(),
                            "around",
                        ));
                    }
                    other => {
                        let t = read_target(other).map_err(|(code, msg, field)| {
                            let path = if field.is_empty() {
                                "around".to_owned()
                            } else {
                                format!("around.{field}")
                            };
                            fail(code, msg, &path)
                        })?;
                        vec![(t, "around".to_owned())]
                    }
                };
                let to = match (map.get("sheet"), map.get("template")) {
                    (Some(Value::String(sheet)), None) => Some(AddTo::Sheet(sheet.clone())),
                    (None, Some(Value::String(template))) => {
                        Some(AddTo::Template(template.clone()))
                    }
                    (Some(_), Some(_)) => {
                        return Err(fail(
                            ErrorCode::Usage,
                            "give sheet or template, not both: an object added to a sheet goes in \
                             its row, and one added to a template is on every sheet built from it"
                                .to_owned(),
                            "",
                        ));
                    }
                    (None, None) => default_sheet.map(|s| AddTo::Sheet(s.to_owned())),
                    (Some(_), None) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a sheet's name".to_owned(),
                            "sheet",
                        ));
                    }
                    (None, Some(_)) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a template's path".to_owned(),
                            "template",
                        ));
                    }
                };
                let margin = map.get("margin").cloned().filter(|v| !v.is_null());
                let keys: Map<String, Value> = map
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            OP_KEY | "sheet" | "template" | "around" | "margin" | "draw_order"
                        )
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Body::AddRevisionCloud {
                    to,
                    around,
                    margin,
                    keys,
                    draw_order,
                }
            } else {
                if object == "revision_cloud" && map.contains_key("margin") {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`margin` is only used with `around`".to_owned(),
                        "margin",
                    ));
                }
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
                let to =
                    match (map.get("sheet"), map.get("template")) {
                        (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                        (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                        (Some(_), Some(_)) => return Err(fail(
                            ErrorCode::Usage,
                            "give sheet or template, not both: an object added to a sheet goes in \
                         its row, and one added to a template is on every sheet built from it"
                                .to_owned(),
                            "",
                        )),
                        (None, None) => match default_sheet {
                            Some(sheet) => AddTo::Sheet(sheet.to_owned()),
                            None => return Err(fail(
                                ErrorCode::MissingKey,
                                "missing key `sheet`: name the sheet to add it to, in its row, or \
                                 give template to add it to a template"
                                    .to_owned(),
                                "sheet",
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
                        .filter(|(k, _)| {
                            !matches!(k.as_str(), OP_KEY | "sheet" | "template" | "draw_order")
                        })
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
                let def_name = crate::description::schema::object_def_name(object);
                let schema = defs.get(&format!("op_{object}")).unwrap_or(&defs[def_name]);
                let mut snaps = Vec::new();
                for (key, value) in &keys {
                    let slots: Vec<&Value> = schema["properties"].get(key).into_iter().collect();
                    snaps_in(value, &slots, defs, &format!("/{key}"), key, &mut snaps)
                        .map_err(|(code, message, field)| fail(code, message, &field))?;
                }
                Body::Add {
                    to,
                    keys,
                    snaps,
                    draw_order,
                }
            }
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
        Kind::DrawOrder => {
            keys(&["target", "targets", "order"])?;
            let targets = read_targets_to(map, "reorder")
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            let Some(order_val) = map.get("order") else {
                return Err(fail(
                    ErrorCode::MissingKey,
                    "missing key `order`: \"front\", \"back\", {\"above\": target}, or \
                     {\"under\": target}"
                        .to_owned(),
                    "order",
                ));
            };
            let order = read_draw_order_place(order_val, "order")
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            Body::DrawOrder { targets, order }
        }
        Kind::TextToFront => {
            keys(&["sheet", "template", "which"])?;
            let to = match (map.get("sheet"), map.get("template")) {
                (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                (Some(_), Some(_)) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give sheet or template, not both".to_owned(),
                        "",
                    ));
                }
                (None, None) => match default_sheet {
                    Some(sheet) => AddTo::Sheet(sheet.to_owned()),
                    None => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `sheet`: name the sheet whose row's annotations to bring \
                             to the front, or give template for a template"
                                .to_owned(),
                            "sheet",
                        ));
                    }
                },
                (Some(_), None) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a sheet's name".to_owned(),
                        "sheet",
                    ));
                }
                (None, Some(_)) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a template's path".to_owned(),
                        "template",
                    ));
                }
            };
            let which = match map.get("which") {
                None => TextToFrontWhich::All,
                Some(Value::String(s)) => match s.as_str() {
                    "text" => TextToFrontWhich::Text,
                    "dimensions" => TextToFrontWhich::Dimensions,
                    "leaders" => TextToFrontWhich::Leaders,
                    "all" => TextToFrontWhich::All,
                    other => {
                        return Err(fail(
                            ErrorCode::BadValue,
                            format!(
                                "unknown `which` value `{other}`; expected one of {}",
                                TEXT_TO_FRONT_KINDS.join(", ")
                            ),
                            "which",
                        ));
                    }
                },
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        format!("expected one of {}", TEXT_TO_FRONT_KINDS.join(", ")),
                        "which",
                    ));
                }
            };
            Body::TextToFront { to, which }
        }
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
        Kind::SetBlockState => {
            keys(&["target", "state"])?;
            let target = target()?;
            let state = match map.get("state") {
                Some(Value::Object(entries)) if !entries.is_empty() => {
                    for (k, v) in entries {
                        if k.trim().is_empty() {
                            return Err(fail(
                                ErrorCode::BadValue,
                                "a dynamic parameter name cannot be empty".to_owned(),
                                "state",
                            ));
                        }
                        if !matches!(
                            v,
                            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                        ) {
                            return Err(fail(
                                ErrorCode::WrongType,
                                format!(
                                    "dynamic parameter `{k}` takes a string, number, boolean, or \
                                     null"
                                ),
                                &format!("state.{k}"),
                            ));
                        }
                    }
                    entries.clone()
                }
                Some(Value::Object(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`state` must name at least one dynamic parameter to set".to_owned(),
                        "state",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `state`, a mapping from dynamic parameter names to values"
                            .to_owned(),
                        "state",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `state`, the dynamic block parameters to set".to_owned(),
                        "state",
                    ))
                }
            };
            Body::SetBlockState { target, state }
        }
        Kind::SetField => {
            keys(&["target", "tag", "occurrence", "field", "value"])?;
            let target = target()?;
            let tag = match map.get("tag") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`tag` cannot be empty".to_owned(),
                        "tag",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `tag`, an attribute tag name".to_owned(),
                        "tag",
                    ))
                }
            };
            let occurrence = match map.get("occurrence") {
                None | Some(Value::Null) => None,
                Some(Value::Number(n)) if n.as_u64().is_some_and(|v| v >= 1) => {
                    if tag.is_none() {
                        return Err(fail(
                            ErrorCode::BadValue,
                            "`occurrence` requires `tag`".to_owned(),
                            "occurrence",
                        ));
                    }
                    Some(n.as_u64().unwrap() as usize)
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`occurrence` is a whole number from 1".to_owned(),
                        "occurrence",
                    ))
                }
            };
            let field =
                match map.get("field") {
                    Some(Value::Null) => None,
                    Some(val) => {
                        let parsed = crate::description::field::parse_field_json(val, "field")
                            .map_err(|e| {
                                let f = e.field.as_deref().unwrap_or("field");
                                fail(e.code, e.message, f)
                            })?;
                        Some(parsed)
                    }
                    None => return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `field`: give a field definition, or `null` to clear a field"
                            .to_owned(),
                        "field",
                    )),
                };
            let value = match map.get("value") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `value`, a text string".to_owned(),
                        "value",
                    ))
                }
            };
            Body::SetField {
                target,
                tag,
                occurrence,
                field,
                value,
            }
        }
        Kind::SetPageSetup => {
            keys(&["sheet", "template", "layout", "set"])?;
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
            let layout = match map.get("layout") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`layout` cannot be empty".to_owned(),
                        "layout",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `layout`, a layout tab name".to_owned(),
                        "layout",
                    ))
                }
            };
            let set = match map.get("set") {
                Some(Value::Object(keys)) if !keys.is_empty() => {
                    for k in keys.keys() {
                        if !crate::description::page_setup::PAGE_SETUP_KEYS.contains(&k.as_str()) {
                            return Err(fail(
                                ErrorCode::UnknownKey,
                                format!(
                                    "unknown page_setup key `{k}`; expected one of: {}",
                                    crate::description::page_setup::PAGE_SETUP_KEYS.join(", ")
                                ),
                                &format!("set.{k}"),
                            ));
                        }
                    }
                    keys.clone()
                }
                Some(Value::Object(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`set` cannot be empty; give at least one page_setup key to change"
                            .to_owned(),
                        "set",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `set`, a mapping of page_setup keys".to_owned(),
                        "set",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `set`: give the page_setup keys to change".to_owned(),
                        "set",
                    ))
                }
            };
            Body::SetPageSetup { to, layout, set }
        }
        Kind::AttachXref => {
            keys(&[
                "sheet",
                "template",
                "name",
                "path",
                "kind",
                "unloaded",
                "place",
                "position",
                "scale",
                "rotation",
                "layer",
                "space",
                "id",
                "insert_name",
            ])?;
            read_attach_xref_body(map, default_sheet, &fail)?
        }
        Kind::DetachXref => {
            keys(&["sheet", "template", "name"])?;
            read_detach_xref_body(map, default_sheet, &fail)?
        }
        Kind::BindXref => {
            keys(&["sheet", "template", "name", "mode"])?;
            read_bind_xref_body(map, default_sheet, &fail)?
        }
        Kind::AddGroup => {
            keys(&[
                "sheet",
                "template",
                "name",
                "targets",
                "description",
                "selectable",
            ])?;
            read_add_group_body(map, default_sheet, &fail)?
        }
        Kind::Ungroup => {
            keys(&["sheet", "template", "name"])?;
            read_ungroup_body(map, default_sheet, &fail)?
        }
        Kind::EditGroup => {
            keys(&[
                "sheet",
                "template",
                "name",
                "add",
                "remove",
                "rename",
                "description",
                "selectable",
            ])?;
            read_edit_group_body(map, default_sheet, &fail)?
        }
        Kind::SetAnnotationScale => {
            keys(&[
                "sheet",
                "template",
                "scale",
                "anno_all_visible",
                "add_scales",
                "remove_scales",
            ])?;
            read_set_annotation_scale_body(map, default_sheet, &fail)?
        }
        Kind::SetObjectScales => {
            keys(&["target", "targets", "annotative", "scales", "add", "remove"])?;
            read_set_object_scales_body(map, default_sheet, &fail)?
        }
        Kind::SetPointStyle => {
            keys(&["sheet", "template", "pdmode", "pdsize"])?;
            read_set_point_style_body(map, default_sheet, &fail)?
        }
        Kind::AddMLineStyle => {
            keys(&[
                "sheet",
                "template",
                "name",
                "description",
                "elements",
                "fill_color",
                "display_joints",
                "start_angle",
                "end_angle",
                "start_square_cap",
                "start_round_cap",
                "start_inner_arcs_cap",
                "end_square_cap",
                "end_round_cap",
                "end_inner_arcs_cap",
            ])?;
            read_add_mline_style_body(map, default_sheet, &fail)?
        }
        Kind::AddBox => {
            keys(&[
                "sheet",
                "template",
                "draw_order",
                "origin",
                "size",
                "length",
                "width",
                "height",
                "layer",
                "color",
                "linetype",
                "lineweight",
                "plot_style",
                "transparency",
                "space",
                "id",
                "name",
            ])?;
            read_add_box_body(map, defs, default_sheet, &fail)?
        }
        Kind::AddCylinder => {
            keys(&[
                "sheet",
                "template",
                "draw_order",
                "base",
                "radius",
                "height",
                "layer",
                "color",
                "linetype",
                "lineweight",
                "plot_style",
                "transparency",
                "space",
                "id",
                "name",
            ])?;
            read_add_cylinder_body(map, defs, default_sheet, &fail)?
        }
        Kind::Extrude => {
            keys(&[
                "target",
                "height",
                "direction",
                "taper_angle",
                "erase_source",
                "layer",
                "id",
                "name",
            ])?;
            read_extrude_body(map, units, &fail)?
        }
        Kind::Revolve => {
            keys(&[
                "target",
                "axis_origin",
                "axis_direction",
                "angle",
                "erase_source",
                "layer",
                "id",
                "name",
            ])?;
            read_revolve_body(map, units, &fail)?
        }
        Kind::SolidBoolean(BooleanOp::Union | BooleanOp::Intersect) => {
            keys(&["targets", "layer", "id", "name"])?;
            let Kind::SolidBoolean(op) = spec.kind else {
                unreachable!()
            };
            read_solid_boolean_body(op, map, &fail)?
        }
        Kind::SolidBoolean(BooleanOp::Subtract) => {
            keys(&["target", "subtract", "layer", "id", "name"])?;
            read_solid_boolean_body(BooleanOp::Subtract, map, &fail)?
        }
        Kind::SetAttributes => {
            keys(&[
                "where",
                "sheets",
                "tag",
                "occurrence",
                "value",
                ROW_VALUE_KEY,
            ])?;
            let selection: Selection = match map.get("where") {
                Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).map_err(|e| {
                    fail(
                        ErrorCode::BadValue,
                        format!("invalid `where` selection: {e}"),
                        "where",
                    )
                })?,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `where`, a query selection mapping such as {\"block\": \"TITLE_BLOCK\"}"
                            .to_owned(),
                        "where",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `where`, the query selection of inserts to edit".to_owned(),
                        "where",
                    ))
                }
            };
            let sheets = read_optional_sheets(map)
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            let occurrence =
                match map.get("occurrence") {
                    None => None,
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
                Some(Value::String(tag)) if !tag.trim().is_empty() => tag.clone(),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`tag` cannot be empty".to_owned(),
                        "tag",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `tag`, the attribute tag to set, such as REV".to_owned(),
                        "tag",
                    ))
                }
                None => {
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
            Body::SetAttributes {
                selection,
                sheets,
                tag,
                occurrence,
                value,
            }
        }
        Kind::ReplaceText => {
            keys(&[
                "find",
                "replace",
                "match",
                "case_sensitive",
                "sheets",
                "in",
                "tag",
                "block",
            ])?;
            let find = match map.get("find") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`find` cannot be empty".to_owned(),
                        "find",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `find`, the text or pattern to find".to_owned(),
                        "find",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `find`, the text or pattern to find".to_owned(),
                        "find",
                    ))
                }
            };
            let replace = match map.get("replace") {
                Some(Value::String(s)) => s.clone(),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `replace`, the replacement text".to_owned(),
                        "replace",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `replace`, the replacement text".to_owned(),
                        "replace",
                    ))
                }
            };
            let matching = match map.get("match") {
                None => TextMatchKind::Exact,
                Some(Value::String(s)) => TextMatchKind::parse(s).ok_or_else(|| {
                    fail(
                        ErrorCode::BadValue,
                        format!(
                            "unknown `match` mode `{s}`; expected one of {}",
                            TextMatchKind::ALL.join(", ")
                        ),
                        "match",
                    )
                })?,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        format!("expected `match`, one of {}", TextMatchKind::ALL.join(", ")),
                        "match",
                    ))
                }
            };
            let case_sensitive = match map.get("case_sensitive") {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `case_sensitive`, true or false".to_owned(),
                        "case_sensitive",
                    ))
                }
            };
            // Validate regex / matcher at parse time so malformed patterns fail
            // before touching any sheet.
            TextMatcher::compile(&find, matching, case_sensitive)
                .map_err(|e| fail(e.code, e.message, "find"))?;
            let sheets = read_optional_sheets(map)
                .map_err(|(code, message, field)| fail(code, message, &field))?;
            let scopes = match map.get("in") {
                None => vec![TextScope::Text, TextScope::MText, TextScope::Attribute],
                Some(Value::Array(items)) if !items.is_empty() => {
                    let mut parsed = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let Some(s) = item.as_str() else {
                            return Err(fail(
                                ErrorCode::WrongType,
                                format!("expected one of {}", TextScope::ALL.join(", ")),
                                &format!("in[{i}]"),
                            ));
                        };
                        let Some(scope) = TextScope::parse(s) else {
                            return Err(fail(
                                ErrorCode::BadValue,
                                format!(
                                    "unknown `in` target `{s}`; expected one of {}",
                                    TextScope::ALL.join(", ")
                                ),
                                &format!("in[{i}]"),
                            ));
                        };
                        if parsed.contains(&scope) {
                            return Err(fail(
                                ErrorCode::BadValue,
                                format!("`in` lists `{s}` more than once"),
                                &format!("in[{i}]"),
                            ));
                        }
                        parsed.push(scope);
                    }
                    parsed
                }
                Some(Value::Array(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`in` must name at least one of text, mtext, attribute".to_owned(),
                        "in",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        format!("expected `in`, a list of {}", TextScope::ALL.join(", ")),
                        "in",
                    ))
                }
            };
            let tag = match map.get("tag") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`tag` cannot be empty".to_owned(),
                        "tag",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `tag`, an attribute tag".to_owned(),
                        "tag",
                    ))
                }
            };
            let block = match map.get("block") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`block` cannot be empty".to_owned(),
                        "block",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `block`, a block name".to_owned(),
                        "block",
                    ))
                }
            };
            Body::ReplaceText {
                find,
                replace,
                matching,
                case_sensitive,
                sheets,
                scopes,
                tag,
                block,
            }
        }
        Kind::Offset => {
            keys(&[
                "target",
                "distance",
                "side",
                "through",
                "id",
                "name",
                "layer",
                "erase_source",
            ])?;
            let target = target()?;
            let has_dist = map.contains_key("distance");
            let has_side = map.contains_key("side");
            let has_through = map.contains_key("through");
            let by =
                match (has_dist, has_side, has_through) {
                    (true, true, false) => {
                        let dist_val = map.get("distance").unwrap();
                        let d = length(dist_val, units)
                            .map_err(|msg| fail(ErrorCode::BadValue, msg, "distance"))?;
                        if !d.is_finite() || d <= 0.0 {
                            return Err(fail(
                                ErrorCode::BadValue,
                                "`distance` must be a positive length".to_owned(),
                                "distance",
                            ));
                        }
                        let side_val = map.get("side").unwrap();
                        let side = match side_val {
                            Value::String(s) => match s.as_str() {
                                "left" => OpOffsetSide::Left,
                                "right" => OpOffsetSide::Right,
                                "inside" => OpOffsetSide::Inside,
                                "outside" => OpOffsetSide::Outside,
                                other => {
                                    return Err(fail(
                                        ErrorCode::BadValue,
                                        format!(
                                            "unknown `side` `{other}`; expected `left`, `right`, \
                                         `inside`, `outside`, or a point [x, y, z]"
                                        ),
                                        "side",
                                    ))
                                }
                            },
                            other => {
                                let pt = op_point(other, units, "side")
                                    .map_err(|(code, msg, field)| fail(code, msg, &field))?;
                                OpOffsetSide::Point(pt)
                            }
                        };
                        OpOffsetBy::Distance { distance: d, side }
                    }
                    (false, false, true) => {
                        let pt = op_point(map.get("through").unwrap(), units, "through")
                            .map_err(|(code, msg, field)| fail(code, msg, &field))?;
                        OpOffsetBy::Through(pt)
                    }
                    (_, _, true) => {
                        return Err(fail(
                            ErrorCode::Usage,
                            "give `through` alone, or `distance` and `side`, not both".to_owned(),
                            "through",
                        ))
                    }
                    (true, false, false) => return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `side` (`left`, `right`, `inside`, `outside`, or a point) \
                         to go with `distance`"
                            .to_owned(),
                        "side",
                    )),
                    (false, true, false) => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `distance` to go with `side`".to_owned(),
                            "distance",
                        ))
                    }
                    (false, false, false) => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "give `distance` and `side`, or `through`".to_owned(),
                            "distance",
                        ))
                    }
                };
            let id = match map.get("id") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`id` cannot be empty".to_owned(),
                        "id",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `id`, a string".to_owned(),
                        "id",
                    ))
                }
            };
            let name = match map.get("name") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`name` cannot be empty".to_owned(),
                        "name",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `name`, a string".to_owned(),
                        "name",
                    ))
                }
            };
            let layer = match map.get("layer") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`layer` cannot be empty".to_owned(),
                        "layer",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `layer`, a layer name".to_owned(),
                        "layer",
                    ))
                }
            };
            let erase_source = match map.get("erase_source") {
                None => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `erase_source`, true or false".to_owned(),
                        "erase_source",
                    ))
                }
            };
            Body::Offset {
                target,
                by,
                id,
                name,
                layer,
                erase_source,
            }
        }
        Kind::Trim => {
            keys(&["target", "boundary", "boundaries", "keep", "new_id"])?;
            let target = target()?;
            let boundaries = read_boundary_targets(map, default_sheet)
                .map_err(|(code, msg, field)| fail(code, msg, &field))?;
            let keep = match map.get("keep") {
                Some(Value::String(s)) => TrimKeep::parse(s).ok_or_else(|| {
                    fail(
                        ErrorCode::BadValue,
                        format!(
                            "unknown `keep` `{s}`; expected one of {}",
                            TrimKeep::ALL.join(", ")
                        ),
                        "keep",
                    )
                })?,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        format!("expected `keep`, one of {}", TrimKeep::ALL.join(", ")),
                        "keep",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        format!("missing key `keep`, one of {}", TrimKeep::ALL.join(", ")),
                        "keep",
                    ))
                }
            };
            let new_id = match map.get("new_id") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`new_id` cannot be empty".to_owned(),
                        "new_id",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `new_id`, a string".to_owned(),
                        "new_id",
                    ))
                }
            };
            Body::Trim {
                target,
                boundaries,
                keep,
                new_id,
            }
        }
        Kind::Extend => {
            keys(&["target", "boundary", "boundaries", "end"])?;
            let target = target()?;
            let boundaries = read_boundary_targets(map, default_sheet)
                .map_err(|(code, msg, field)| fail(code, msg, &field))?;
            let end = match map.get("end") {
                Some(Value::String(s)) => ExtendEnd::parse(s).ok_or_else(|| {
                    fail(
                        ErrorCode::BadValue,
                        format!(
                            "unknown `end` `{s}`; expected one of {}",
                            ExtendEnd::ALL.join(", ")
                        ),
                        "end",
                    )
                })?,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        format!("expected `end`, one of {}", ExtendEnd::ALL.join(", ")),
                        "end",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        format!("missing key `end`, one of {}", ExtendEnd::ALL.join(", ")),
                        "end",
                    ))
                }
            };
            Body::Extend {
                target,
                boundaries,
                end,
            }
        }
        Kind::Fillet => {
            keys(&[
                "first",
                "second",
                "target",
                "radius",
                "trim",
                "first_end",
                "second_end",
                "new_id",
                "layer",
            ])?;
            let mode = read_corner_mode(map, default_sheet)
                .map_err(|(code, msg, field)| fail(code, msg, &field))?;
            let radius = match map.get("radius") {
                Some(v) => {
                    let r =
                        length(v, units).map_err(|msg| fail(ErrorCode::BadValue, msg, "radius"))?;
                    if !r.is_finite() || r < 0.0 {
                        return Err(fail(
                            ErrorCode::BadValue,
                            "`radius` must be a non-negative length".to_owned(),
                            "radius",
                        ));
                    }
                    r
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `radius`, the fillet radius (0 for a sharp corner)".to_owned(),
                        "radius",
                    ))
                }
            };
            Body::Fillet { mode, radius }
        }
        Kind::Chamfer => {
            keys(&[
                "first",
                "second",
                "target",
                "distance",
                "distance1",
                "distance2",
                "length",
                "angle",
                "trim",
                "first_end",
                "second_end",
                "new_id",
                "layer",
            ])?;
            let mode = read_corner_mode(map, default_sheet)
                .map_err(|(code, msg, field)| fail(code, msg, &field))?;
            let spec = read_chamfer_spec(map, units)
                .map_err(|(code, msg, field)| fail(code, msg, &field))?;
            Body::Chamfer { mode, spec }
        }
        Kind::Break => {
            keys(&["target", "at", "gap", "from", "to", "new_id"])?;
            let target = target()?;
            let has_at = map.contains_key("at");
            let has_gap = map.contains_key("gap");
            let has_from = map.contains_key("from");
            let has_to = map.contains_key("to");
            let by = match (has_at, has_from || has_to) {
                (true, false) => {
                    let at = op_point(map.get("at").unwrap(), units, "at")
                        .map_err(|(code, msg, field)| fail(code, msg, &field))?;
                    let gap = match map.get("gap") {
                        None => None,
                        Some(v) => {
                            let g = length(v, units)
                                .map_err(|msg| fail(ErrorCode::BadValue, msg, "gap"))?;
                            if !g.is_finite() || g <= 0.0 {
                                return Err(fail(
                                    ErrorCode::BadValue,
                                    "`gap` must be a positive length".to_owned(),
                                    "gap",
                                ));
                            }
                            Some(g)
                        }
                    };
                    OpBreakBy::AtPoint {
                        at: Box::new(at),
                        gap,
                    }
                }
                (false, true) => {
                    if has_gap {
                        return Err(fail(
                            ErrorCode::Usage,
                            "`gap` is used with `at`, not with `from` and `to`".to_owned(),
                            "gap",
                        ));
                    }
                    if !has_from {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `from` to go with `to`".to_owned(),
                            "from",
                        ));
                    }
                    if !has_to {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `to` to go with `from`".to_owned(),
                            "to",
                        ));
                    }
                    let from = op_point(map.get("from").unwrap(), units, "from")
                        .map_err(|(code, msg, field)| fail(code, msg, &field))?;
                    let to = op_point(map.get("to").unwrap(), units, "to")
                        .map_err(|(code, msg, field)| fail(code, msg, &field))?;
                    OpBreakBy::Between {
                        from: Box::new(from),
                        to: Box::new(to),
                    }
                }
                (true, true) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give `at` (with optional `gap`) or `from` and `to`, not both".to_owned(),
                        "from",
                    ));
                }
                (false, false) => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing `at` (or `from` and `to`), where to break `target`".to_owned(),
                        "at",
                    ));
                }
            };
            let new_id = match map.get("new_id") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`new_id` cannot be empty".to_owned(),
                        "new_id",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `new_id`, a string".to_owned(),
                        "new_id",
                    ))
                }
            };
            Body::Break { target, by, new_id }
        }
        Kind::Join => {
            keys(&["targets", "tolerance"])?;
            let targets = match map.get("targets") {
                Some(Value::Array(items)) if items.len() >= 2 => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let field = format!("targets[{i}]");
                        let mut t = read_target(item).map_err(|(code, msg, sub)| {
                            let f = if sub.is_empty() {
                                field.clone()
                            } else {
                                format!("{field}.{sub}")
                            };
                            fail(code, msg, &f)
                        })?;
                        if let Some(sheet) = default_sheet {
                            if let Target::Name {
                                sheet: slot @ None, ..
                            } = &mut t
                            {
                                *slot = Some(sheet.to_owned());
                            }
                        }
                        out.push((t, field));
                    }
                    out
                }
                Some(Value::Array(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`targets` must name at least two objects to join".to_owned(),
                        "targets",
                    ))
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `targets`, a list of at least two objects to join".to_owned(),
                        "targets",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `targets`, the objects to join".to_owned(),
                        "targets",
                    ))
                }
            };
            let tolerance = match map.get("tolerance") {
                None => DEFAULT_JOIN_TOLERANCE,
                Some(v) => {
                    let d = length(v, units)
                        .map_err(|msg| fail(ErrorCode::BadValue, msg, "tolerance"))?;
                    if !d.is_finite() || d < 0.0 {
                        return Err(fail(
                            ErrorCode::BadValue,
                            "`tolerance` must be a non-negative length".to_owned(),
                            "tolerance",
                        ));
                    }
                    d
                }
            };
            Body::Join { targets, tolerance }
        }
        Kind::Stretch => {
            keys(&[
                "sheet", "template", "space", "targets", "box", "by", "from", "to",
            ])?;
            let to = match (map.get("sheet"), map.get("template")) {
                (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                (Some(_), Some(_)) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give `sheet` or `template`, not both".to_owned(),
                        "",
                    ));
                }
                (None, None) => match default_sheet {
                    Some(sheet) => AddTo::Sheet(sheet.to_owned()),
                    None => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `sheet` (or `template`), where to stretch".to_owned(),
                            "sheet",
                        ));
                    }
                },
                (Some(_), None) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a sheet's name".to_owned(),
                        "sheet",
                    ));
                }
                (None, Some(_)) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a template's path".to_owned(),
                        "template",
                    ));
                }
            };
            let space = match map.get("space") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`space` cannot be empty".to_owned(),
                        "space",
                    ));
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `space`, \"model\" or a layout name".to_owned(),
                        "space",
                    ));
                }
            };
            let targets = match map.get("targets") {
                None => None,
                Some(Value::Array(items)) if !items.is_empty() => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let field = format!("targets[{i}]");
                        let mut t = read_target(item).map_err(|(code, msg, sub)| {
                            let f = if sub.is_empty() {
                                field.clone()
                            } else {
                                format!("{field}.{sub}")
                            };
                            fail(code, msg, &f)
                        })?;
                        if let Some(sheet) = default_sheet {
                            if let Target::Name {
                                sheet: slot @ None, ..
                            } = &mut t
                            {
                                *slot = Some(sheet.to_owned());
                            }
                        }
                        out.push((t, field));
                    }
                    Some(out)
                }
                Some(Value::Array(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`targets` must name at least one object, or be left out".to_owned(),
                        "targets",
                    ));
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `targets`, a list of object targets".to_owned(),
                        "targets",
                    ));
                }
            };
            let parse_box_corner = |v: &Value, field: &str| -> Result<OpPoint, Error> {
                match v {
                    Value::Array(xy) if xy.len() == 2 => {
                        let x = length(&xy[0], units)
                            .map_err(|m| fail(ErrorCode::BadNumber, m, &format!("{field}[0]")))?;
                        let y = length(&xy[1], units)
                            .map_err(|m| fail(ErrorCode::BadNumber, m, &format!("{field}[1]")))?;
                        Ok(OpPoint::At([x, y, 0.0]))
                    }
                    _ => op_point(v, units, field).map_err(|(code, msg, f)| fail(code, msg, &f)),
                }
            };
            let corners = match map.get("box") {
                Some(Value::Array(items)) if items.len() == 2 => [
                    Box::new(parse_box_corner(&items[0], "box[0]")?),
                    Box::new(parse_box_corner(&items[1], "box[1]")?),
                ],
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `box`, two opposite corners `[corner1, corner2]`".to_owned(),
                        "box",
                    ));
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `box`, two opposite corners of the stretch selection box"
                            .to_owned(),
                        "box",
                    ));
                }
            };
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
                            "expected how far to stretch, three lengths [dx, dy, dz]".to_owned(),
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
                        "missing key `by`, [dx, dy, dz]; or give `from` and `to`".to_owned(),
                        "by",
                    ))
                }
                (false, true, false) | (false, false, true) => {
                    let missing = if given("from") { "to" } else { "from" };
                    return Err(fail(
                        ErrorCode::MissingKey,
                        format!("missing key `{missing}` to go with the other point"),
                        missing,
                    ));
                }
                (true, _, _) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give `by`, or `from` and `to`, not both".to_owned(),
                        "",
                    ))
                }
            };
            Body::Stretch {
                to,
                space,
                targets,
                corners,
                motion,
            }
        }
        Kind::Explode => {
            keys(&["target", "targets", "id_prefix"])?;
            let targets = match (map.get("target"), map.get("targets")) {
                (Some(_), Some(_)) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give `target` or `targets`, not both".to_owned(),
                        "targets",
                    ));
                }
                (None, None) => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `target` (or `targets`), the object(s) to explode".to_owned(),
                        "target",
                    ));
                }
                (Some(one), None) => {
                    let mut t = read_target(one).map_err(|(code, msg, sub)| {
                        let f = if sub.is_empty() {
                            "target".to_owned()
                        } else {
                            format!("target.{sub}")
                        };
                        fail(code, msg, &f)
                    })?;
                    if let Some(sheet) = default_sheet {
                        if let Target::Name {
                            sheet: slot @ None, ..
                        } = &mut t
                        {
                            *slot = Some(sheet.to_owned());
                        }
                    }
                    vec![(t, "target".to_owned())]
                }
                (None, Some(Value::Array(items))) if !items.is_empty() => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let field = format!("targets[{i}]");
                        let mut t = read_target(item).map_err(|(code, msg, sub)| {
                            let f = if sub.is_empty() {
                                field.clone()
                            } else {
                                format!("{field}.{sub}")
                            };
                            fail(code, msg, &f)
                        })?;
                        if let Some(sheet) = default_sheet {
                            if let Target::Name {
                                sheet: slot @ None, ..
                            } = &mut t
                            {
                                *slot = Some(sheet.to_owned());
                            }
                        }
                        out.push((t, field));
                    }
                    out
                }
                (None, Some(Value::Array(_))) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`targets` must name at least one object to explode".to_owned(),
                        "targets",
                    ));
                }
                (None, Some(_)) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `targets`, a list of object targets".to_owned(),
                        "targets",
                    ));
                }
            };
            let id_prefix = match map.get("id_prefix") {
                None => None,
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                Some(Value::String(_)) => {
                    return Err(fail(
                        ErrorCode::BadValue,
                        "`id_prefix` cannot be empty".to_owned(),
                        "id_prefix",
                    ));
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected `id_prefix`, a non-empty string".to_owned(),
                        "id_prefix",
                    ));
                }
            };
            Body::Explode { targets, id_prefix }
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

/// Reads a `draw_order` / `order` placement: `"front"`, `"back"`,
/// `{"above": target}`, or `{"under": target}`.
fn read_draw_order_place(value: &Value, field: &str) -> Result<DrawOrderPlace, KeyError> {
    match value {
        Value::String(s) => match s.as_str() {
            "front" => Ok(DrawOrderPlace::Front),
            "back" => Ok(DrawOrderPlace::Back),
            other => Err((
                ErrorCode::BadValue,
                format!(
                    "unknown draw order `{other}`; expected \"front\", \"back\", \
                     {{\"above\": target}}, or {{\"under\": target}}"
                ),
                field.to_owned(),
            )),
        },
        Value::Object(map) => {
            if let Some(key) = map
                .keys()
                .find(|k| !matches!(k.as_str(), "above" | "under"))
            {
                return Err((
                    ErrorCode::UnknownKey,
                    format!("unknown key `{key}`; expected `above` or `under`"),
                    format!("{field}.{key}"),
                ));
            }
            match (map.get("above"), map.get("under")) {
                (Some(_), Some(_)) => Err((
                    ErrorCode::Usage,
                    "give `above` or `under`, not both".to_owned(),
                    field.to_owned(),
                )),
                (Some(t), None) => {
                    let target =
                        read_target(t).map_err(|e| target_at(&format!("{field}.above"), e))?;
                    Ok(DrawOrderPlace::Above(target))
                }
                (None, Some(t)) => {
                    let target =
                        read_target(t).map_err(|e| target_at(&format!("{field}.under"), e))?;
                    Ok(DrawOrderPlace::Under(target))
                }
                (None, None) => Err((
                    ErrorCode::MissingKey,
                    "expected `above` or `under` naming a reference object".to_owned(),
                    field.to_owned(),
                )),
            }
        }
        _ => Err((
            ErrorCode::WrongType,
            "expected \"front\", \"back\", {\"above\": target}, or {\"under\": target}".to_owned(),
            field.to_owned(),
        )),
    }
}

/// Reads a target: `{"id": "SHEET/ID"}`, `{"name": ..., "sheet": ...}`, or
/// `{"group": ..., "sheet": ...}`.
fn read_target(value: &Value) -> Result<Target, (ErrorCode, String, String)> {
    let Value::Object(map) = value else {
        return Err((
            ErrorCode::WrongType,
            "expected a target, {\"id\": \"DEMO-001/border\"}, {\"name\": \"title-block\", \
             \"sheet\": \"DEMO-001\"}, or {\"group\": \"FT-101\"}"
                .to_owned(),
            "target".to_owned(),
        ));
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "name" | "group" | "sheet"))
    {
        return Err((
            ErrorCode::UnknownKey,
            format!("unknown key `{key}`; a target takes id, name and sheet, or group and sheet"),
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
    match (text("id")?, text("name")?, text("group")?, text("sheet")?) {
        (Some(id), None, None, None) => match id.split_once('/') {
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
        (Some(_), None, None, Some(_)) => Err((
            ErrorCode::Usage,
            "an ID names its sheet already; give id alone, or name/group and sheet".to_owned(),
            "target".to_owned(),
        )),
        (None, Some(name), None, sheet) => Ok(Target::Name { name, sheet }),
        (None, None, Some(group), sheet) => {
            let trimmed = group.trim();
            if trimmed.is_empty() {
                return Err((
                    ErrorCode::BadValue,
                    "`group` cannot be empty".to_owned(),
                    "target.group".to_owned(),
                ));
            }
            Ok(Target::Group {
                group: trimmed.to_owned(),
                sheet,
            })
        }
        (None, None, None, _) => Err((
            ErrorCode::MissingKey,
            "a target names its object by `id`, `name`, or `group`".to_owned(),
            "target".to_owned(),
        )),
        _ => Err((
            ErrorCode::Usage,
            "a target takes one of `id`, `name`, or `group`, not more than one".to_owned(),
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
