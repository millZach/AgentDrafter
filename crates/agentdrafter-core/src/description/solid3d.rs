//! Reads 3D solid (`solid3d`) objects from sheet descriptions and block
//! libraries, and states their JSON Schema definitions (ADR-0012).

use serde_json::{json, Value};

use super::schema::{object_def, NUMBER, POSITIVE_NUMBER};
use super::strict_yaml::{index_path, Node};
use super::{at, bad_value, length, plain_number, point, scalar, sequence, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{BooleanOp, Point3, Properties, Solid3D, SolidBody};
use crate::units::Unit;

/// All allowed `kind` values for a `solid3d` body.
pub(crate) const SOLID_KINDS: [&str; 13] = [
    "box",
    "cylinder",
    "cone",
    "sphere",
    "torus",
    "wedge",
    "pyramid",
    "extrude",
    "revolve",
    "union",
    "subtract",
    "intersect",
    "mesh",
];

/// Keys of a nested [`SolidBody`] (used inside `operands` of a boolean solid).
pub(crate) const SOLID_BODY_KEYS: [&str; 19] = [
    "kind",
    "origin",
    "size",
    "base",
    "center",
    "radius",
    "major_radius",
    "minor_radius",
    "base_size",
    "height",
    "profile",
    "direction",
    "taper_angle",
    "axis_origin",
    "axis_direction",
    "angle",
    "operands",
    "vertices",
    "faces",
];

/// All keys a `solid3d` object takes on a sheet or in a block.
pub(crate) const SOLID3D_KEYS: [&str; 26] = [
    "kind",
    "origin",
    "size",
    "base",
    "center",
    "radius",
    "major_radius",
    "minor_radius",
    "base_size",
    "height",
    "profile",
    "direction",
    "taper_angle",
    "axis_origin",
    "axis_direction",
    "angle",
    "operands",
    "vertices",
    "faces",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// Reads a `solid3d` object from `fields` on `layer`.
pub(super) fn read_solid3d(fields: &Fields, units: Unit, layer: String) -> Result<Solid3D, Error> {
    let body = read_solid_body_fields(fields, units, 0)?;
    Ok(Solid3D {
        body,
        layer,
        properties: Properties::default(),
    })
}

/// Reads a standalone [`SolidBody`] from a JSON value (for change-set operations
/// such as `add_solid3d`, `add_box`, `add_cylinder`, `extrude`, `revolve`).
pub(crate) fn solid_body_from_json(value: &Value, units: Unit) -> Result<SolidBody, Error> {
    let node = super::edit::node_of(value);
    read_solid_body_node(&node, "solid3d", units, 0)
}

fn read_solid_body_node(
    node: &Node,
    path: &str,
    units: Unit,
    depth: usize,
) -> Result<SolidBody, Error> {
    if depth > 16 {
        return Err(bad_value(
            node,
            path,
            "3D solid boolean nesting exceeds the maximum depth of 16",
        ));
    }
    let fields = Fields::of(node, path, &SOLID_BODY_KEYS, "a 3D solid body")?;
    read_solid_body_fields(&fields, units, depth)
}

fn positive_length(fields: &Fields, key: &str, units: Unit, what: &str) -> Result<f64, Error> {
    let node = fields.require(key)?;
    let path = fields.path_of(key);
    let v = length(node, &path, units)?;
    if v > 0.0 && v.is_finite() {
        Ok(v)
    } else {
        Err(bad_value(
            node,
            &path,
            format!("{what} must be more than 0"),
        ))
    }
}

fn positive_size3(fields: &Fields, key: &str, units: Unit) -> Result<[f64; 3], Error> {
    let node = fields.require(key)?;
    let path = fields.path_of(key);
    let items = sequence(
        node,
        &path,
        "three positive lengths [length_x, width_y, height_z]",
    )?;
    let [lx, wy, hz] = items else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "`size` has three dimensions [length_x, width_y, height_z]; this one has {}",
                    items.len()
                ),
            ),
            &path,
            node.at,
        ));
    };
    let x = length(lx, &index_path(&path, 0), units)?;
    let y = length(wy, &index_path(&path, 1), units)?;
    let z = length(hz, &index_path(&path, 2), units)?;
    if x <= 0.0 {
        return Err(bad_value(
            lx,
            &index_path(&path, 0),
            "length_x must be more than 0",
        ));
    }
    if y <= 0.0 {
        return Err(bad_value(
            wy,
            &index_path(&path, 1),
            "width_y must be more than 0",
        ));
    }
    if z <= 0.0 {
        return Err(bad_value(
            hz,
            &index_path(&path, 2),
            "height_z must be more than 0",
        ));
    }
    Ok([x, y, z])
}

fn non_zero_vector(node: &Node, path: &str, units: Unit, what: &str) -> Result<Point3, Error> {
    let v = point(node, path, units)?;
    if v.x * v.x + v.y * v.y + v.z * v.z <= 1e-18 {
        return Err(bad_value(node, path, format!("{what} cannot be [0, 0, 0]")));
    }
    Ok(v)
}

fn forbid_keys(fields: &Fields, kind: &str, allowed: &[&str]) -> Result<(), Error> {
    for &key in &SOLID_BODY_KEYS {
        if key == "kind" || allowed.contains(&key) {
            continue;
        }
        if let Some(node) = fields.get(key) {
            return Err(bad_value(
                node,
                &fields.path_of(key),
                format!("`{key}` does not belong on a `{kind}` 3D solid"),
            ));
        }
    }
    Ok(())
}

fn read_solid_body_fields(fields: &Fields, units: Unit, depth: usize) -> Result<SolidBody, Error> {
    let kind_node = fields.require("kind")?;
    let kind_path = fields.path_of("kind");
    let kind = scalar(
        kind_node,
        &kind_path,
        "a 3D solid kind (`box`, `cylinder`, `cone`, `sphere`, `torus`, `wedge`, `pyramid`, \
         `extrude`, `revolve`, `union`, `subtract`, `intersect`, or `mesh`)",
    )?;
    match kind {
        "box" => {
            forbid_keys(fields, "box", &["origin", "size"])?;
            let origin = point(fields.require("origin")?, &fields.path_of("origin"), units)?;
            let size = positive_size3(fields, "size", units)?;
            Ok(SolidBody::Box { origin, size })
        }
        "wedge" => {
            forbid_keys(fields, "wedge", &["origin", "size"])?;
            let origin = point(fields.require("origin")?, &fields.path_of("origin"), units)?;
            let size = positive_size3(fields, "size", units)?;
            Ok(SolidBody::Wedge { origin, size })
        }
        "cylinder" => {
            forbid_keys(fields, "cylinder", &["base", "radius", "height"])?;
            let base = point(fields.require("base")?, &fields.path_of("base"), units)?;
            let radius = positive_length(fields, "radius", units, "a cylinder's radius")?;
            let height = positive_length(fields, "height", units, "a cylinder's height")?;
            Ok(SolidBody::Cylinder {
                base,
                radius,
                height,
            })
        }
        "cone" => {
            forbid_keys(fields, "cone", &["base", "radius", "height"])?;
            let base = point(fields.require("base")?, &fields.path_of("base"), units)?;
            let radius = positive_length(fields, "radius", units, "a cone's radius")?;
            let height = positive_length(fields, "height", units, "a cone's height")?;
            Ok(SolidBody::Cone {
                base,
                radius,
                height,
            })
        }
        "sphere" => {
            forbid_keys(fields, "sphere", &["center", "radius"])?;
            let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
            let radius = positive_length(fields, "radius", units, "a sphere's radius")?;
            Ok(SolidBody::Sphere { center, radius })
        }
        "torus" => {
            forbid_keys(fields, "torus", &["center", "major_radius", "minor_radius"])?;
            let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
            let major_radius =
                positive_length(fields, "major_radius", units, "a torus's major_radius")?;
            let minor_node = fields.require("minor_radius")?;
            let minor_path = fields.path_of("minor_radius");
            let minor_radius =
                positive_length(fields, "minor_radius", units, "a torus's minor_radius")?;
            if minor_radius >= major_radius {
                return Err(bad_value(
                    minor_node,
                    &minor_path,
                    "a torus's `minor_radius` must be less than `major_radius`",
                ));
            }
            Ok(SolidBody::Torus {
                center,
                major_radius,
                minor_radius,
            })
        }
        "pyramid" => {
            forbid_keys(fields, "pyramid", &["base", "base_size", "height"])?;
            let base = point(fields.require("base")?, &fields.path_of("base"), units)?;
            let base_size = positive_length(fields, "base_size", units, "a pyramid's base_size")?;
            let height = positive_length(fields, "height", units, "a pyramid's height")?;
            Ok(SolidBody::Pyramid {
                base,
                base_size,
                height,
            })
        }
        "extrude" => {
            forbid_keys(
                fields,
                "extrude",
                &["profile", "height", "direction", "taper_angle"],
            )?;
            let profile_node = fields.require("profile")?;
            let profile_path = fields.path_of("profile");
            let (profile, bulges) =
                super::polyline::profile_3d_points(profile_node, &profile_path, units)?;
            let h_node = fields.require("height")?;
            let h_path = fields.path_of("height");
            let height = length(h_node, &h_path, units)?;
            if height.abs() <= 1e-9 || !height.is_finite() {
                return Err(bad_value(
                    h_node,
                    &h_path,
                    "an extrusion's `height` must be non-zero",
                ));
            }
            let direction = match fields.get("direction") {
                Some(n) => Some(non_zero_vector(
                    n,
                    &fields.path_of("direction"),
                    units,
                    "extrusion `direction`",
                )?),
                None => None,
            };
            let taper_angle = match fields.get("taper_angle") {
                Some(n) => {
                    let tp = fields.path_of("taper_angle");
                    let a = plain_number(n, &tp, "a taper angle in degrees between -85 and 85")?;
                    if !(-85.0..=85.0).contains(&a) {
                        return Err(bad_value(
                            n,
                            &tp,
                            "extrusion `taper_angle` must be between -85 and 85 degrees",
                        ));
                    }
                    a
                }
                None => 0.0,
            };
            let body = SolidBody::Extrude {
                profile,
                bulges,
                height,
                direction,
                taper_angle,
            };
            crate::geometry::solid3d::evaluate_polyhedron(&body).map_err(|err| {
                err.at_field(fields.path.to_owned())
                    .at_position(fields.at.line, fields.at.column)
            })?;
            Ok(body)
        }
        "revolve" => {
            forbid_keys(
                fields,
                "revolve",
                &["profile", "axis_origin", "axis_direction", "angle"],
            )?;
            let profile_node = fields.require("profile")?;
            let profile_path = fields.path_of("profile");
            let (profile, bulges) =
                super::polyline::profile_3d_points(profile_node, &profile_path, units)?;
            let axis_origin = point(
                fields.require("axis_origin")?,
                &fields.path_of("axis_origin"),
                units,
            )?;
            let axis_direction = non_zero_vector(
                fields.require("axis_direction")?,
                &fields.path_of("axis_direction"),
                units,
                "revolution `axis_direction`",
            )?;
            let angle = match fields.get("angle") {
                Some(n) => {
                    let ap = fields.path_of("angle");
                    let a = plain_number(n, &ap, "a sweep angle in degrees in (0, 360]")?;
                    if !(a > 0.0 && a <= 360.0) {
                        return Err(bad_value(
                            n,
                            &ap,
                            "revolve `angle` must be more than 0 and at most 360 degrees",
                        ));
                    }
                    a
                }
                None => 360.0,
            };
            let body = SolidBody::Revolve {
                profile,
                bulges,
                axis_origin,
                axis_direction,
                angle,
            };
            crate::geometry::solid3d::evaluate_polyhedron(&body).map_err(|err| {
                err.at_field(fields.path.to_owned())
                    .at_position(fields.at.line, fields.at.column)
            })?;
            Ok(body)
        }
        "union" | "subtract" | "intersect" => {
            forbid_keys(fields, kind, &["operands"])?;
            let op = match kind {
                "union" => BooleanOp::Union,
                "subtract" => BooleanOp::Subtract,
                _ => BooleanOp::Intersect,
            };
            let ops_node = fields.require("operands")?;
            let ops_path = fields.path_of("operands");
            let items = sequence(
                ops_node,
                &ops_path,
                "a list of at least two 3D solid bodies",
            )?;
            if items.len() < 2 {
                return Err(bad_value(
                    ops_node,
                    &ops_path,
                    format!(
                        "`{kind}` requires at least 2 operands; this one has {}",
                        items.len()
                    ),
                ));
            }
            let mut operands = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                operands.push(read_solid_body_node(
                    item,
                    &index_path(&ops_path, i),
                    units,
                    depth + 1,
                )?);
            }
            let body = SolidBody::Boolean { op, operands };
            crate::geometry::solid3d::evaluate_polyhedron(&body).map_err(|err| {
                err.at_field(fields.path.to_owned())
                    .at_position(fields.at.line, fields.at.column)
            })?;
            Ok(body)
        }
        "mesh" => {
            forbid_keys(fields, "mesh", &["vertices", "faces"])?;
            let verts_node = fields.require("vertices")?;
            let verts_path = fields.path_of("vertices");
            let v_items = sequence(verts_node, &verts_path, "a list of at least 4 3D vertices")?;
            if v_items.len() < 4 {
                return Err(bad_value(
                    verts_node,
                    &verts_path,
                    "a 3D solid `mesh` requires at least 4 vertices",
                ));
            }
            let mut vertices = Vec::with_capacity(v_items.len());
            for (i, v) in v_items.iter().enumerate() {
                vertices.push(point(v, &index_path(&verts_path, i), units)?);
            }
            let faces_node = fields.require("faces")?;
            let faces_path = fields.path_of("faces");
            let f_items = sequence(
                faces_node,
                &faces_path,
                "a list of at least 4 planar polygon faces",
            )?;
            if f_items.len() < 4 {
                return Err(bad_value(
                    faces_node,
                    &faces_path,
                    "a 3D solid `mesh` requires at least 4 faces",
                ));
            }
            let mut faces = Vec::with_capacity(f_items.len());
            for (fi, f) in f_items.iter().enumerate() {
                let fp = index_path(&faces_path, fi);
                let idx_items = sequence(f, &fp, "a list of at least 3 vertex indices")?;
                if idx_items.len() < 3 {
                    return Err(bad_value(
                        f,
                        &fp,
                        "each face must have at least 3 vertex indices",
                    ));
                }
                let mut ring = Vec::with_capacity(idx_items.len());
                for (vi_pos, vn) in idx_items.iter().enumerate() {
                    let vp = index_path(&fp, vi_pos);
                    let raw = scalar(vn, &vp, "a vertex index (non-negative integer)")?;
                    let idx = raw.parse::<usize>().map_err(|_| {
                        bad_value(
                            vn,
                            &vp,
                            format!("expected a non-negative vertex index, found `{raw}`"),
                        )
                    })?;
                    if idx >= vertices.len() {
                        return Err(bad_value(
                            vn,
                            &vp,
                            format!(
                                "vertex index {idx} is out of range (mesh has {} vertices)",
                                vertices.len()
                            ),
                        ));
                    }
                    ring.push(idx);
                }
                faces.push(ring);
            }
            let body = SolidBody::Mesh { vertices, faces };
            crate::geometry::solid3d::evaluate_polyhedron(&body).map_err(|err| {
                err.at_field(fields.path.to_owned())
                    .at_position(fields.at.line, fields.at.column)
            })?;
            Ok(body)
        }
        other => Err(bad_value(
            kind_node,
            &kind_path,
            format!(
                "unknown 3D solid kind `{other}`; expected one of {}",
                SOLID_KINDS.join(", ")
            ),
        )),
    }
}

/// Shared JSON Schema properties for a [`SolidBody`].
fn solid_body_properties() -> Value {
    json!({
        "kind": {
            "description": "How the 3D solid is constructed: a primitive (`box`, `cylinder`, \
                            `cone`, `sphere`, `torus`, `wedge`, `pyramid`), a sweep (`extrude`, \
                            `revolve`), a CSG boolean (`union`, `subtract`, `intersect`), or a \
                            closed 2-manifold planar polyhedron (`mesh`).",
            "enum": SOLID_KINDS,
        },
        "origin": {
            "$ref": "#/$defs/point",
            "description": "Minimum corner `[x, y, z]` of a `box` or `wedge`.",
        },
        "size": {
            "description": "Positive dimensions `[length_x, width_y, height_z]` of a `box` or `wedge`.",
            "type": "array",
            "items": { "$ref": "#/$defs/length_more_than_0" },
            "minItems": 3,
            "maxItems": 3,
        },
        "base": {
            "$ref": "#/$defs/point",
            "description": "Base centre `[x, y, z]` of a `cylinder`, `cone`, or `pyramid`.",
        },
        "center": {
            "$ref": "#/$defs/point",
            "description": "Centre `[x, y, z]` of a `sphere` or `torus`.",
        },
        "radius": {
            "$ref": "#/$defs/length_more_than_0",
            "description": "Radius of a `cylinder`, `cone`, or `sphere`.",
        },
        "major_radius": {
            "$ref": "#/$defs/length_more_than_0",
            "description": "Distance from the torus centre to the tube centre.",
        },
        "minor_radius": {
            "$ref": "#/$defs/length_more_than_0",
            "description": "Tube radius of a `torus` (must be less than `major_radius`).",
        },
        "base_size": {
            "$ref": "#/$defs/length_more_than_0",
            "description": "Square base side length of a `pyramid`.",
        },
        "height": {
            "$ref": "#/$defs/length",
            "description": "Height along `+Z` (positive for `cylinder`/`cone`/`pyramid`, non-zero for `extrude`).",
        },
        "profile": {
            "description": "Closed planar profile vertices (`>= 3` points, optionally with `bulge` or `arc_angle`) for `extrude` or `revolve`.",
            "type": "array",
            "minItems": 3,
            "items": {
                "anyOf": [
                    { "$ref": "#/$defs/point" },
                    { "$ref": "#/$defs/polyline_point" },
                ],
            },
        },
        "direction": {
            "$ref": "#/$defs/point",
            "description": "Optional non-zero extrusion direction vector for `extrude` (default `[0, 0, 1]`).",
        },
        "taper_angle": {
            "description": "Optional extrusion draft/taper angle in degrees between -85 and 85 (positive tapers inward; default 0).",
            "anyOf": [
                { "type": "number", "minimum": -85, "maximum": 85 },
                { "type": "string", "pattern": format!("^{NUMBER}$") },
            ],
        },
        "axis_origin": {
            "$ref": "#/$defs/point",
            "description": "A point on the axis of revolution for `revolve`.",
        },
        "axis_direction": {
            "$ref": "#/$defs/point",
            "description": "Non-zero direction vector of the axis of revolution for `revolve`.",
        },
        "angle": {
            "description": "Sweep angle in degrees in `(0, 360]` for `revolve` (default 360).",
            "anyOf": [
                { "type": "number", "exclusiveMinimum": 0, "maximum": 360 },
                { "type": "string", "pattern": format!("^{POSITIVE_NUMBER}$") },
            ],
        },
        "operands": {
            "description": "Two or more 3D solid bodies combined by `union`, `subtract` (first minus the rest), or `intersect`.",
            "type": "array",
            "minItems": 2,
            "items": { "$ref": "#/$defs/solid_body" },
        },
        "vertices": {
            "description": "Corner positions of a closed 2-manifold planar `mesh` (`>= 4` points).",
            "type": "array",
            "minItems": 4,
            "items": { "$ref": "#/$defs/point" },
        },
        "faces": {
            "description": "Outward-CCW planar polygon faces of a `mesh`, each a list of `>= 3` vertex indices.",
            "type": "array",
            "minItems": 4,
            "items": {
                "type": "array",
                "minItems": 3,
                "items": { "type": "integer", "minimum": 0 },
            },
        },
    })
}

/// JSON Schema definition for `#/$defs/solid_body` (nested boolean operands).
pub(crate) fn solid_body_def() -> Value {
    json!({
        "description": "A 3D solid body definition (`box`, `cylinder`, `cone`, `sphere`, \
                        `torus`, `wedge`, `pyramid`, `extrude`, `revolve`, `union`, `subtract`, \
                        `intersect`, or `mesh`).",
        "type": "object",
        "required": ["kind"],
        "additionalProperties": false,
        "properties": solid_body_properties(),
    })
}

/// JSON Schema definition for `#/$defs/solid3d` (a `solid3d` object on a sheet or block).
pub(crate) fn solid3d_def() -> Value {
    object_def(
        "A native AutoCAD 3D solid (`3DSOLID`, ACIS B-rep): primitives (`box`, `cylinder`, \
         `cone`, `sphere`, `torus`, `wedge`, `pyramid`), swept solids (`extrude`, `revolve`), \
         CSG booleans (`union`, `subtract`, `intersect`), or a closed 2-manifold planar polyhedron \
         (`mesh`). Drawn as 2D wireframe in plan view or through a viewport's `view_direction`.",
        &["kind"],
        solid_body_properties(),
    )
}

/// Serializes a [`SolidBody`] into a JSON map matching the sheet description
/// schema (used by `import`, `explode`, `change` operations, and transforms).
pub(crate) fn solid_body_to_map(body: &SolidBody) -> serde_json::Map<String, Value> {
    let pt = |p: Point3| json!([p.x, p.y, p.z]);
    let mut m = serde_json::Map::new();
    match body {
        SolidBody::Box { origin, size } => {
            m.insert("kind".into(), json!("box"));
            m.insert("origin".into(), pt(*origin));
            m.insert("size".into(), json!(size));
        }
        SolidBody::Wedge { origin, size } => {
            m.insert("kind".into(), json!("wedge"));
            m.insert("origin".into(), pt(*origin));
            m.insert("size".into(), json!(size));
        }
        SolidBody::Cylinder {
            base,
            radius,
            height,
        } => {
            m.insert("kind".into(), json!("cylinder"));
            m.insert("base".into(), pt(*base));
            m.insert("radius".into(), json!(radius));
            m.insert("height".into(), json!(height));
        }
        SolidBody::Cone {
            base,
            radius,
            height,
        } => {
            m.insert("kind".into(), json!("cone"));
            m.insert("base".into(), pt(*base));
            m.insert("radius".into(), json!(radius));
            m.insert("height".into(), json!(height));
        }
        SolidBody::Sphere { center, radius } => {
            m.insert("kind".into(), json!("sphere"));
            m.insert("center".into(), pt(*center));
            m.insert("radius".into(), json!(radius));
        }
        SolidBody::Torus {
            center,
            major_radius,
            minor_radius,
        } => {
            m.insert("kind".into(), json!("torus"));
            m.insert("center".into(), pt(*center));
            m.insert("major_radius".into(), json!(major_radius));
            m.insert("minor_radius".into(), json!(minor_radius));
        }
        SolidBody::Pyramid {
            base,
            base_size,
            height,
        } => {
            m.insert("kind".into(), json!("pyramid"));
            m.insert("base".into(), pt(*base));
            m.insert("base_size".into(), json!(base_size));
            m.insert("height".into(), json!(height));
        }
        SolidBody::Extrude {
            profile,
            bulges,
            height,
            direction,
            taper_angle,
        } => {
            m.insert("kind".into(), json!("extrude"));
            m.insert("profile".into(), profile_to_json(profile, bulges));
            m.insert("height".into(), json!(height));
            if let Some(dir) = direction {
                m.insert("direction".into(), pt(*dir));
            }
            if taper_angle.abs() > 1e-12 {
                m.insert("taper_angle".into(), json!(taper_angle));
            }
        }
        SolidBody::Revolve {
            profile,
            bulges,
            axis_origin,
            axis_direction,
            angle,
        } => {
            m.insert("kind".into(), json!("revolve"));
            m.insert("profile".into(), profile_to_json(profile, bulges));
            m.insert("axis_origin".into(), pt(*axis_origin));
            m.insert("axis_direction".into(), pt(*axis_direction));
            if (*angle - 360.0).abs() > 1e-9 {
                m.insert("angle".into(), json!(angle));
            }
        }
        SolidBody::Boolean { op, operands } => {
            m.insert("kind".into(), json!(op.name()));
            m.insert(
                "operands".into(),
                Value::Array(
                    operands
                        .iter()
                        .map(|o| Value::Object(solid_body_to_map(o)))
                        .collect(),
                ),
            );
        }
        SolidBody::Mesh { vertices, faces } => {
            m.insert("kind".into(), json!("mesh"));
            m.insert(
                "vertices".into(),
                Value::Array(vertices.iter().copied().map(pt).collect()),
            );
            m.insert("faces".into(), json!(faces));
        }
    }
    m
}

fn profile_to_json(profile: &[Point3], bulges: &[f64]) -> Value {
    let pt = |p: Point3| json!([p.x, p.y, p.z]);
    Value::Array(
        profile
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let b = bulges.get(i).copied().unwrap_or(0.0);
                if b.abs() > 1e-12 {
                    json!({ "point": pt(p), "bulge": b })
                } else {
                    pt(p)
                }
            })
            .collect(),
    )
}
