//! 3D solids in the AutoCAD style (`3DSOLID`): primitives (`BOX`, `CYLINDER`,
//! `CONE`, `SPHERE`, `TORUS`, `WEDGE`, `PYRAMID`), swept solids (`EXTRUDE`,
//! `REVOLVE`), CSG booleans (`UNION`, `SUBTRACT`, `INTERSECT`), and general
//! closed 2-manifold planar polyhedra (ADR-0012).

use serde::{Deserialize, Serialize};

use super::{is_zero, layer_zero, Point3, Properties};

pub(crate) fn full_turn() -> f64 {
    360.0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_full_turn(angle: &f64) -> bool {
    (*angle - 360.0).abs() < 1e-9
}

/// Which CSG boolean operation combines two or more 3D solids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    /// AutoCAD `UNION`: volume inside any operand.
    Union,
    /// AutoCAD `SUBTRACT`: volume inside the first operand and outside the rest.
    Subtract,
    /// AutoCAD `INTERSECT`: volume inside every operand.
    Intersect,
}

impl BooleanOp {
    /// Canonical lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            BooleanOp::Union => "union",
            BooleanOp::Subtract => "subtract",
            BooleanOp::Intersect => "intersect",
        }
    }
}

/// How a 3D solid body is constructed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SolidBody {
    /// Axis-aligned box (`BOX`) spanning `origin` to `origin + size`.
    Box {
        /// Minimum corner `[x, y, z]`.
        origin: Point3,
        /// Positive dimensions `[length_x, width_y, height_z]`.
        size: [f64; 3],
    },
    /// Right circular cylinder (`CYLINDER`) with circular base centered at
    /// `base` and extending `height` along `+Z`.
    Cylinder {
        base: Point3,
        radius: f64,
        height: f64,
    },
    /// Right circular cone (`CONE`) with circular base centered at `base` and
    /// apex at `base.z + height`.
    Cone {
        base: Point3,
        radius: f64,
        height: f64,
    },
    /// Sphere (`SPHERE`) centered at `center` with `radius`.
    Sphere { center: Point3, radius: f64 },
    /// Torus (`TORUS`) centered at `center` in the XY plane (axis `+Z`).
    Torus {
        center: Point3,
        major_radius: f64,
        minor_radius: f64,
    },
    /// Right triangular prism (`WEDGE`) with legs along `+X` (`size[0]`) and
    /// `+Y` (`size[1]`) from `origin`, extruded along `+Z` by `size[2]`.
    Wedge { origin: Point3, size: [f64; 3] },
    /// Square-base pyramid (`PYRAMID`) with base centered at `base`, side
    /// `base_size`, and apex at `base.z + height`.
    Pyramid {
        base: Point3,
        base_size: f64,
        height: f64,
    },
    /// Linear extrusion (`EXTRUDE`) of a closed planar profile loop.
    Extrude {
        /// Ordered vertices of the closed profile (`>= 3` points).
        profile: Vec<Point3>,
        /// Optional bulge per segment (as on a closed polyline).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bulges: Vec<f64>,
        /// Extrusion distance (positive; negative reverses `direction`).
        height: f64,
        /// Extrusion direction vector (defaults to `[0, 0, 1]`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<Point3>,
        /// Optional taper angle in degrees (default `0`, positive tapers inward).
        #[serde(default, skip_serializing_if = "is_zero")]
        taper_angle: f64,
    },
    /// Solid of revolution (`REVOLVE`) sweeping a closed planar profile around
    /// an axis.
    Revolve {
        /// Ordered vertices of the closed profile (`>= 3` points).
        profile: Vec<Point3>,
        /// Optional bulge per segment (as on a closed polyline).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bulges: Vec<f64>,
        /// A point on the axis of revolution.
        axis_origin: Point3,
        /// Non-zero direction vector of the axis of revolution.
        axis_direction: Point3,
        /// Sweep angle in degrees in `(0, 360]` (default `360`).
        #[serde(default = "full_turn", skip_serializing_if = "is_full_turn")]
        angle: f64,
    },
    /// CSG boolean combination (`UNION`, `SUBTRACT`, `INTERSECT`) of two or
    /// more solids.
    Boolean {
        op: BooleanOp,
        operands: Vec<SolidBody>,
    },
    /// Closed 2-manifold planar polyhedron (outward-CCW polygon faces).
    Mesh {
        vertices: Vec<Point3>,
        faces: Vec<Vec<usize>>,
    },
}

/// A 3D solid (`3DSOLID`) object on a sheet or in a block definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solid3D {
    #[serde(flatten)]
    pub body: SolidBody,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl SolidBody {
    /// Canonical lowercase kind name (`"box"`, `"cylinder"`, `"extrude"`, etc.).
    pub const fn kind_name(&self) -> &'static str {
        match self {
            SolidBody::Box { .. } => "box",
            SolidBody::Cylinder { .. } => "cylinder",
            SolidBody::Cone { .. } => "cone",
            SolidBody::Sphere { .. } => "sphere",
            SolidBody::Torus { .. } => "torus",
            SolidBody::Wedge { .. } => "wedge",
            SolidBody::Pyramid { .. } => "pyramid",
            SolidBody::Extrude { .. } => "extrude",
            SolidBody::Revolve { .. } => "revolve",
            SolidBody::Boolean { op, .. } => op.name(),
            SolidBody::Mesh { .. } => "mesh",
        }
    }

    /// Translates the solid body by `delta`.
    pub fn translate(&mut self, delta: Point3) {
        let shift = |p: &mut Point3| {
            p.x += delta.x;
            p.y += delta.y;
            p.z += delta.z;
        };
        match self {
            SolidBody::Box { origin, .. } | SolidBody::Wedge { origin, .. } => shift(origin),
            SolidBody::Cylinder { base, .. }
            | SolidBody::Cone { base, .. }
            | SolidBody::Pyramid { base, .. } => shift(base),
            SolidBody::Sphere { center, .. } | SolidBody::Torus { center, .. } => shift(center),
            SolidBody::Extrude { profile, .. } => {
                for p in profile {
                    shift(p);
                }
            }
            SolidBody::Revolve {
                profile,
                axis_origin,
                ..
            } => {
                for p in profile {
                    shift(p);
                }
                shift(axis_origin);
            }
            SolidBody::Boolean { operands, .. } => {
                for op in operands {
                    op.translate(delta);
                }
            }
            SolidBody::Mesh { vertices, .. } => {
                for v in vertices {
                    shift(v);
                }
            }
        }
    }

    /// Applies an arbitrary 3D point transform (`p -> f(p)`) by evaluating to a
    /// polyhedron when non-axis-aligned, or transforming exact parameters when
    /// possible (uniform scale / pure translation).
    pub fn scale_about(&mut self, base_pt: Point3, factor: f64) {
        let sc = |p: &mut Point3| {
            p.x = base_pt.x + (p.x - base_pt.x) * factor;
            p.y = base_pt.y + (p.y - base_pt.y) * factor;
            p.z = base_pt.z + (p.z - base_pt.z) * factor;
        };
        match self {
            SolidBody::Box { origin, size } | SolidBody::Wedge { origin, size } => {
                sc(origin);
                size[0] *= factor;
                size[1] *= factor;
                size[2] *= factor;
            }
            SolidBody::Cylinder {
                base,
                radius,
                height,
            }
            | SolidBody::Cone {
                base,
                radius,
                height,
            } => {
                sc(base);
                *radius *= factor;
                *height *= factor;
            }
            SolidBody::Sphere { center, radius } => {
                sc(center);
                *radius *= factor;
            }
            SolidBody::Torus {
                center,
                major_radius,
                minor_radius,
            } => {
                sc(center);
                *major_radius *= factor;
                *minor_radius *= factor;
            }
            SolidBody::Pyramid {
                base,
                base_size,
                height,
            } => {
                sc(base);
                *base_size *= factor;
                *height *= factor;
            }
            SolidBody::Extrude {
                profile, height, ..
            } => {
                for p in profile {
                    sc(p);
                }
                *height *= factor;
            }
            SolidBody::Revolve {
                profile,
                axis_origin,
                ..
            } => {
                for p in profile {
                    sc(p);
                }
                sc(axis_origin);
            }
            SolidBody::Boolean { operands, .. } => {
                for op in operands {
                    op.scale_about(base_pt, factor);
                }
            }
            SolidBody::Mesh { vertices, .. } => {
                for v in vertices {
                    sc(v);
                }
            }
        }
    }
}

/// Named standard AutoCAD orthographic & isometric view directions.
pub const VIEW_DIRECTION_PRESETS: [(&str, [f64; 3]); 10] = [
    ("top", [0.0, 0.0, 1.0]),
    ("bottom", [0.0, 0.0, -1.0]),
    ("front", [0.0, -1.0, 0.0]),
    ("back", [0.0, 1.0, 0.0]),
    ("left", [-1.0, 0.0, 0.0]),
    ("right", [1.0, 0.0, 0.0]),
    ("sw_isometric", [-1.0, -1.0, 1.0]),
    ("se_isometric", [1.0, -1.0, 1.0]),
    ("ne_isometric", [1.0, 1.0, 1.0]),
    ("nw_isometric", [-1.0, 1.0, 1.0]),
];

/// Resolves a preset view name (case-insensitive) to a 3D view direction vector.
pub fn preset_view_direction(name: &str) -> Option<Point3> {
    let lower = name.trim().to_ascii_lowercase();
    VIEW_DIRECTION_PRESETS
        .iter()
        .find(|(k, _)| *k == lower)
        .map(|(_, [x, y, z])| Point3::new(*x, *y, *z))
}
