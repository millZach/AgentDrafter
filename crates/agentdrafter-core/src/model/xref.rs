//! External references (xrefs): attached or overlaid DWG/DXF drawings placed
//! into a host sheet via an [`Insert`].
//!
//! In AutoCAD, an external reference is a block table record marked with
//! `is_xref` (and `is_xref_overlay` for an overlay) carrying a `xref_path`
//! instead of owned entities, placed on the sheet by one or more `INSERT`
//! objects naming it. Attached xrefs propagate when the host drawing is itself
//! referenced by another drawing; overlaid xrefs do not.

use serde::{Deserialize, Serialize};

/// How an external reference is attached to its host drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XrefKind {
    /// Attached reference (`XATTACH` default): carried along when this drawing
    /// is itself externally referenced by another drawing.
    #[default]
    Attach,
    /// Overlaid reference: drawn in this drawing only, skipped when this
    /// drawing is externally referenced by another drawing.
    Overlay,
}

impl XrefKind {
    /// All xref attachment kinds, in schema order.
    pub const ALL: [XrefKind; 2] = [XrefKind::Attach, XrefKind::Overlay];

    /// Whether this kind is the default (`Attach`).
    pub fn is_attach(&self) -> bool {
        matches!(self, XrefKind::Attach)
    }

    /// Name as written in sheet descriptions, JSON operations and reports.
    pub const fn name(self) -> &'static str {
        match self {
            XrefKind::Attach => "attach",
            XrefKind::Overlay => "overlay",
        }
    }
}

/// How an external reference is bound into its host drawing by `bind_xref`
/// (`XBIND`), matching AutoCAD's `BINDTYPE` system variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XrefBindMode {
    /// AutoCAD `BINDTYPE = 0` (`Bind`): converts xref-dependent table names
    /// (`xref|layer`) to `xref$0$layer` so they never collide with host tables.
    #[default]
    Bind,
    /// AutoCAD `BINDTYPE = 1` (`Insert`): strips the `xref|` prefix and merges
    /// table names into the host drawing's own tables (keeping the host's
    /// definition when a name already exists).
    Insert,
}

impl XrefBindMode {
    /// All bind modes, in schema order.
    pub const ALL: [XrefBindMode; 2] = [XrefBindMode::Bind, XrefBindMode::Insert];

    /// Name as written in JSON operations.
    pub const fn name(self) -> &'static str {
        match self {
            XrefBindMode::Bind => "bind",
            XrefBindMode::Insert => "insert",
        }
    }
}

/// An external reference declared on a sheet or read from a DWG/DXF.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Xref {
    /// The block name the host drawing's inserts refer to (e.g. `GRID` or
    /// `TITLE_BLOCK`).
    pub name: String,
    /// The saved file path of the referenced DWG or DXF, as written in the
    /// sheet description or DWG/DXF block record.
    pub path: String,
    /// Whether this xref is attached or overlaid.
    #[serde(default, skip_serializing_if = "XrefKind::is_attach")]
    pub kind: XrefKind,
    /// Whether the xref is currently unloaded (not drawn, kept as a
    /// reference).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unloaded: bool,
}

/// An external reference that could not be found or loaded when resolving
/// xrefs for reading, drawing, querying or checking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedXref {
    /// The xref's block name in its host drawing.
    pub name: String,
    /// The saved file path that was looked for.
    pub path: String,
    /// Whether this xref was attached or overlaid.
    pub kind: XrefKind,
    /// Why the xref could not be loaded (e.g. file not found, circular
    /// reference, or DWG/DXF read error).
    pub reason: String,
}
