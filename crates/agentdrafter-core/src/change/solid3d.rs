//! Shared 3D solid modeling operations for change sets and original DWG
//! revisions (`EXTRUDE`, `REVOLVE`, `UNION`, `SUBTRACT`, `INTERSECT`)
//! (ADR-0012).

use std::f64::consts::TAU;

use crate::error::{Error, ErrorCode};
use crate::geometry::solid3d::evaluate_polyhedron;
use crate::model::{BooleanOp, Object, Point3, Properties, Solid3D, SolidBody};

const PROFILE_SEGMENTS: usize = 32;

/// Extracted closed planar 2D profile from an existing sheet object.
pub(crate) struct ExtractedProfile {
    pub points: Vec<Point3>,
    pub bulges: Vec<f64>,
    /// Exact `(center, radius)` when the source object was a `Circle`, so an
    /// untapered Z-axis `EXTRUDE` produces an exact analytic ACIS `Cylinder`.
    pub exact_circle: Option<(Point3, f64)>,
    pub layer: String,
    pub properties: Properties,
}

/// Extracts a closed planar profile from an existing `Polyline`, `Circle`, or
/// `Ellipse` object on a sheet.
pub(crate) fn extract_profile(object: &Object, op_name: &str) -> Result<ExtractedProfile, Error> {
    match object {
        Object::Polyline(poly) => {
            if !poly.closed {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("`{op_name}` requires a closed polyline; this polyline is open"),
                ));
            }
            if poly.points.len() < 3 {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("`{op_name}` requires a profile with at least 3 vertices"),
                ));
            }
            Ok(ExtractedProfile {
                points: poly.points.clone(),
                bulges: poly.bulges.clone(),
                exact_circle: None,
                layer: poly.layer.clone(),
                properties: poly.properties.clone(),
            })
        }
        Object::Circle(circle) => {
            let mut pts = Vec::with_capacity(PROFILE_SEGMENTS);
            for i in 0..PROFILE_SEGMENTS {
                let a = (i as f64) * TAU / (PROFILE_SEGMENTS as f64);
                pts.push(Point3::new(
                    circle.center.x + circle.radius * a.cos(),
                    circle.center.y + circle.radius * a.sin(),
                    circle.center.z,
                ));
            }
            Ok(ExtractedProfile {
                points: pts,
                bulges: Vec::new(),
                exact_circle: Some((circle.center, circle.radius)),
                layer: circle.layer.clone(),
                properties: circle.properties.clone(),
            })
        }
        Object::Ellipse(e) => {
            if !e.is_whole() {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("`{op_name}` requires a closed (whole) ellipse, not an elliptical arc"),
                ));
            }
            let maj = [e.major_axis.x, e.major_axis.y];
            let min = [-maj[1] * e.ratio, maj[0] * e.ratio];
            let mut pts = Vec::with_capacity(PROFILE_SEGMENTS);
            for i in 0..PROFILE_SEGMENTS {
                let t = (i as f64) * TAU / (PROFILE_SEGMENTS as f64);
                pts.push(Point3::new(
                    e.center.x + t.cos() * maj[0] + t.sin() * min[0],
                    e.center.y + t.cos() * maj[1] + t.sin() * min[1],
                    e.center.z,
                ));
            }
            Ok(ExtractedProfile {
                points: pts,
                bulges: Vec::new(),
                exact_circle: None,
                layer: e.layer.clone(),
                properties: e.properties.clone(),
            })
        }
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`{op_name}` works on a closed polyline, circle, or whole ellipse, not a {}",
                other.type_name()
            ),
        )),
    }
}

/// Constructs a validated [`Solid3D`] for `extrude` (`EXTRUDE` / `EXT`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_extrude_solid(
    profile: Vec<Point3>,
    bulges: Vec<f64>,
    exact_circle: Option<(Point3, f64)>,
    height: f64,
    direction: Option<Point3>,
    taper_angle: f64,
    layer: String,
    properties: Properties,
) -> Result<Solid3D, Error> {
    if !height.is_finite() || height.abs() <= 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`extrude` requires a non-zero `height`",
        ));
    }
    if !(-85.0..=85.0).contains(&taper_angle) || !taper_angle.is_finite() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`taper_angle` must be between -85 and 85 degrees",
        ));
    }
    let is_z_up = match direction {
        None => true,
        Some(d) => {
            let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
            if !len.is_finite() || len <= 1e-12 {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "`direction` cannot be [0, 0, 0]",
                ));
            }
            d.x.abs() <= 1e-9 * len && d.y.abs() <= 1e-9 * len && d.z > 0.0
        }
    };
    let body = if let (Some((center, radius)), true) =
        (exact_circle, is_z_up && taper_angle.abs() <= 1e-9)
    {
        let (base_z, h) = if height >= 0.0 {
            (center.z, height)
        } else {
            (center.z + height, -height)
        };
        SolidBody::Cylinder {
            base: Point3::new(center.x, center.y, base_z),
            radius,
            height: h,
        }
    } else {
        let body = SolidBody::Extrude {
            profile,
            bulges,
            height,
            direction,
            taper_angle,
        };
        evaluate_polyhedron(&body)?;
        body
    };
    Ok(Solid3D {
        body,
        layer,
        properties,
    })
}

/// Constructs a validated [`Solid3D`] for `revolve` (`REVOLVE` / `REV`).
pub(crate) fn build_revolve_solid(
    profile: Vec<Point3>,
    bulges: Vec<f64>,
    axis_origin: Point3,
    axis_direction: Point3,
    angle: f64,
    layer: String,
    properties: Properties,
) -> Result<Solid3D, Error> {
    let dir_len = (axis_direction.x * axis_direction.x
        + axis_direction.y * axis_direction.y
        + axis_direction.z * axis_direction.z)
        .sqrt();
    if !dir_len.is_finite() || dir_len <= 1e-12 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`axis_direction` cannot be [0, 0, 0]",
        ));
    }
    if !angle.is_finite() || angle <= 0.0 || angle > 360.0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`angle` must be more than 0 and at most 360 degrees",
        ));
    }
    let body = SolidBody::Revolve {
        profile,
        bulges,
        axis_origin,
        axis_direction,
        angle,
    };
    evaluate_polyhedron(&body)?;
    Ok(Solid3D {
        body,
        layer,
        properties,
    })
}

/// Evaluates a CSG boolean (`UNION`, `SUBTRACT`, `INTERSECT`) over two or more
/// existing [`Solid3D`] objects, validating that the result is a non-empty closed
/// 2-manifold solid.
pub(crate) fn build_boolean_solid(
    op: BooleanOp,
    solids: &[&Solid3D],
    layer_override: Option<String>,
) -> Result<Solid3D, Error> {
    if solids.len() < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("`{}` requires at least 2 3D solids", op.name()),
        ));
    }
    let operands: Vec<SolidBody> = solids.iter().map(|s| s.body.clone()).collect();
    let body = SolidBody::Boolean { op, operands };
    // Validate that the boolean produces a non-empty closed 2-manifold before mutating.
    evaluate_polyhedron(&body)?;
    Ok(Solid3D {
        body,
        layer: layer_override.unwrap_or_else(|| solids[0].layer.clone()),
        properties: solids[0].properties.clone(),
    })
}
