//! ACIS `3DSOLID` read and write behind the `dwg` seam (ADR-0009, ADR-0012).
//!
//! Writes native AutoCAD `3DSOLID` entities via `acadrust`'s ACIS primitive
//! builders (`build_box`, `build_cylinder`, `build_cone`, `build_sphere`,
//! `build_torus`, `build_wedge`, `build_pyramid`) for analytic primitives and
//! `build_planar_body` for extrusions, revolutions, CSG booleans, and planar
//! polyhedra. `acadrust::DwgWriter` encodes the SAT document as binary SAB in
//! the `AcDsPrototype_1b` section for R2013+/2018 DWG (`AC1032`) and as
//! encrypted SAT for DXF and older DWG versions.
//!
//! Reads back analytic spheres, tori, Z-axis cylinders and cones, axis-aligned
//! boxes, and closed 2-manifold planar polyhedra from both binary SAB and text
//! SAT payloads. Complex curved plant solids (NURBS faces, multi-loop planar
//! faces with holes, tilted cylinders) return `None` so `read`/`revise` reports
//! them under `not_read` and preserves their original ACIS bytes untouched
//! (ADR-0006).

use std::collections::HashMap;

use acadrust::entities::acis::{
    primitives, SatCoedge, SatConeSurface, SatDocument, SatEdge, SatLoop, SatPlaneSurface,
    SatPoint, SatSphereSurface, SatTorusSurface, SatVertex, Sense,
};
use acadrust::entities::Solid3D as DwgSolid3D;
use acadrust::types::Vector3;

use crate::error::{Error, ErrorCode};
use crate::geometry::solid3d::{evaluate_polyhedron, solid_metrics};
use crate::model::{Point3, Properties, Solid3D, SolidBody};

const TOL: f64 = 1e-6;

/// Converts an AgentDrafter [`Solid3D`] into a native AutoCAD ACIS `3DSOLID`
/// entity.
pub(super) fn dwg_solid3d(solid: &Solid3D) -> Result<DwgSolid3D, Error> {
    let sat = build_sat_document(&solid.body)?;
    let sat_text = sat.to_sat_string();
    let mut dwg = DwgSolid3D::from_sat(&sat_text);
    let metrics = solid_metrics(&solid.body);
    let mid = [
        0.5 * (metrics.min[0] + metrics.max[0]),
        0.5 * (metrics.min[1] + metrics.max[1]),
        0.5 * (metrics.min[2] + metrics.max[2]),
    ];
    dwg.point_of_reference = Vector3::new(mid[0], mid[1], mid[2]);
    Ok(dwg)
}

/// Builds a validated ACIS [`SatDocument`] for `body`.
fn build_sat_document(body: &SolidBody) -> Result<SatDocument, Error> {
    let sat = match body {
        SolidBody::Box { origin, size } => {
            if size[0] <= 0.0 || size[1] <= 0.0 || size[2] <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D box must have positive size along X, Y and Z",
                ));
            }
            let center = [
                origin.x + 0.5 * size[0],
                origin.y + 0.5 * size[1],
                origin.z + 0.5 * size[2],
            ];
            primitives::build_box(center, size[0], size[1], size[2])
        }
        SolidBody::Cylinder {
            base,
            radius,
            height,
        } => {
            if *radius <= 0.0 || *height <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D cylinder must have positive radius and height",
                ));
            }
            primitives::build_cylinder([base.x, base.y, base.z], *radius, *height)
        }
        SolidBody::Cone {
            base,
            radius,
            height,
        } => {
            if *radius <= 0.0 || *height <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D cone must have positive radius and height",
                ));
            }
            primitives::build_cone([base.x, base.y, base.z], *radius, *height)
        }
        SolidBody::Sphere { center, radius } => {
            if *radius <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D sphere must have positive radius",
                ));
            }
            primitives::build_sphere([center.x, center.y, center.z], *radius)
        }
        SolidBody::Torus {
            center,
            major_radius,
            minor_radius,
        } => {
            if *major_radius <= 0.0 || *minor_radius <= 0.0 || *minor_radius >= *major_radius {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D torus must have positive major_radius > minor_radius > 0",
                ));
            }
            primitives::build_torus([center.x, center.y, center.z], *major_radius, *minor_radius)
        }
        SolidBody::Wedge { origin, size } => {
            if size[0] <= 0.0 || size[1] <= 0.0 || size[2] <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D wedge must have positive size along X, Y and Z",
                ));
            }
            primitives::build_wedge([origin.x, origin.y, origin.z], size[0], size[1], size[2])
        }
        SolidBody::Pyramid {
            base,
            base_size,
            height,
        } => {
            if *base_size <= 0.0 || *height <= 0.0 {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    "a 3D pyramid must have positive base_size and height",
                ));
            }
            primitives::build_pyramid([base.x, base.y, base.z], *base_size, *height)
        }
        SolidBody::Extrude { .. }
        | SolidBody::Revolve { .. }
        | SolidBody::Boolean { .. }
        | SolidBody::Mesh { .. } => {
            let poly = evaluate_polyhedron(body).map_err(|err| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not build 3D solid B-rep: {}", err.message),
                )
            })?;
            primitives::build_planar_body(&poly.vertices, &poly.faces).ok_or_else(|| {
                Error::new(
                    ErrorCode::DwgWrite,
                    "3D solid polyhedron did not form a valid closed 2-manifold ACIS body",
                )
            })?
        }
    };
    let errs = sat.validate();
    if !errs.is_empty() {
        return Err(Error::new(
            ErrorCode::DwgWrite,
            format!(
                "generated ACIS 3DSOLID failed B-rep validation ({} errors)",
                errs.len()
            ),
        ));
    }
    Ok(sat)
}

/// Applies ACIS body placement `(M, T, s)` using ACIS row-vector convention:
/// `world = s * (p * M) + T`.
fn apply_placement(p: [f64; 3], m: [[f64; 3]; 3], t: [f64; 3], s: f64) -> [f64; 3] {
    [
        s * (p[0] * m[0][0] + p[1] * m[1][0] + p[2] * m[2][0]) + t[0],
        s * (p[0] * m[0][1] + p[1] * m[1][1] + p[2] * m[2][1]) + t[1],
        s * (p[0] * m[0][2] + p[1] * m[1][2] + p[2] * m[2][2]) + t[2],
    ]
}

fn apply_rotation(v: [f64; 3], m: [[f64; 3]; 3]) -> [f64; 3] {
    [
        v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0],
        v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1],
        v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2],
    ]
}

fn vlen(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn vnorm(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = vlen(v);
    (l > 1e-12).then_some([v[0] / l, v[1] / l, v[2] / l])
}

fn vsub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vdot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vcross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn is_z_axis(v: [f64; 3]) -> bool {
    v[0].abs() <= TOL && v[1].abs() <= TOL && (v[2].abs() - 1.0).abs() <= TOL
}

/// Reads an AutoCAD ACIS `3DSOLID` entity back into an AgentDrafter [`Solid3D`]
/// when the core has a lossless model for it (analytic sphere, torus, Z-axis
/// cylinder/cone, axis-aligned box, or closed 2-manifold planar polyhedron).
/// Returns `None` for unsupported curved or multi-loop ACIS bodies so `read`
/// and `revise` report them as `not_read` and keep them untouched.
pub(super) fn read_solid3d(dwg: &DwgSolid3D, layer: String) -> Option<Solid3D> {
    let sat = dwg.acis_data.parse()?;
    let body = read_sat_body(&sat)?;
    Some(Solid3D {
        body,
        layer,
        properties: Properties::default(),
    })
}

fn read_sat_body(sat: &SatDocument) -> Option<SolidBody> {
    let faces = sat.faces();
    if faces.is_empty() {
        return None;
    }
    let (rot, trans, scale) = sat.placement();
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }

    // Check that the body's rotation matrix is orthogonal (no shear).
    let r0 = rot[0];
    let r1 = rot[1];
    let r2 = rot[2];
    let ortho = (vlen(r0) - 1.0).abs() <= 1e-4
        && (vlen(r1) - 1.0).abs() <= 1e-4
        && (vlen(r2) - 1.0).abs() <= 1e-4
        && vdot(r0, r1).abs() <= 1e-4
        && vdot(r1, r2).abs() <= 1e-4
        && vdot(r2, r0).abs() <= 1e-4;
    if !ortho {
        return None;
    }

    // 1. Single-face closed analytic primitives: Sphere & Torus.
    if faces.len() == 1 {
        let f = &faces[0];
        let surf_rec = sat.resolve(f.surface())?;
        if let Some(sp) = SatSphereSurface::from_record(surf_rec) {
            if !f.first_loop().is_null() {
                return None;
            }
            let (cx, cy, cz) = sp.center();
            let c = apply_placement([cx, cy, cz], rot, trans, scale);
            let r = sp.radius().abs() * scale;
            if r > 0.0 && r.is_finite() {
                return Some(SolidBody::Sphere {
                    center: Point3::new(c[0], c[1], c[2]),
                    radius: r,
                });
            }
            return None;
        }
        if let Some(tor) = SatTorusSurface::from_record(surf_rec) {
            if !f.first_loop().is_null() {
                return None;
            }
            let (nx, ny, nz) = tor.normal();
            let world_n = vnorm(apply_rotation([nx, ny, nz], rot))?;
            if !is_z_axis(world_n) {
                return None;
            }
            let (cx, cy, cz) = tor.center();
            let c = apply_placement([cx, cy, cz], rot, trans, scale);
            let major = tor.major_radius().abs() * scale;
            let minor = tor.minor_radius().abs() * scale;
            if major > minor && minor > 0.0 && major.is_finite() {
                return Some(SolidBody::Torus {
                    center: Point3::new(c[0], c[1], c[2]),
                    major_radius: major,
                    minor_radius: minor,
                });
            }
            return None;
        }
        return None;
    }

    // 2. Analytic 2-face Cone or 3-face Cylinder along Z.
    if faces.len() == 3 {
        if let Some(cyl) = try_read_cylinder(sat, &faces, rot, trans, scale) {
            return Some(cyl);
        }
    }
    if faces.len() == 2 {
        if let Some(cone) = try_read_cone(sat, &faces, rot, trans, scale) {
            return Some(cone);
        }
    }

    // 3. Purely planar closed 2-manifold B-rep (Box, Wedge, Pyramid, Extrude,
    //    Revolve, CSG Boolean, Mesh).
    read_planar_brep(sat, &faces, rot, trans, scale)
}

fn try_read_cylinder(
    sat: &SatDocument,
    faces: &[acadrust::entities::acis::SatFace<'_>],
    rot: [[f64; 3]; 3],
    trans: [f64; 3],
    scale: f64,
) -> Option<SolidBody> {
    let mut cone_surf: Option<SatConeSurface<'_>> = None;
    let mut plane_zs: Vec<f64> = Vec::new();
    for f in faces {
        let surf_rec = sat.resolve(f.surface())?;
        if let Some(cs) = SatConeSurface::from_record(surf_rec) {
            if cone_surf.is_some() {
                return None;
            }
            cone_surf = Some(cs);
        } else {
            let ps = SatPlaneSurface::from_record(surf_rec)?;
            let (nx, ny, nz) = ps.normal();
            let wn = vnorm(apply_rotation([nx, ny, nz], rot))?;
            if !is_z_axis(wn) {
                return None;
            }
            let (rx, ry, rz) = ps.root_point();
            let wr = apply_placement([rx, ry, rz], rot, trans, scale);
            plane_zs.push(wr[2]);
        }
    }
    let cs = cone_surf?;
    if plane_zs.len() != 2 {
        return None;
    }
    if cs.sin_half_angle().abs() > 1e-5 || (cs.ratio() - 1.0).abs() > 1e-4 {
        return None;
    }
    let (ax, ay, az) = cs.axis();
    let wa = vnorm(apply_rotation([ax, ay, az], rot))?;
    if !is_z_axis(wa) {
        return None;
    }
    let (cx, cy, cz) = cs.center();
    let wc = apply_placement([cx, cy, cz], rot, trans, scale);
    let radius = cs.radius() * scale;
    let z_min = plane_zs[0].min(plane_zs[1]);
    let z_max = plane_zs[0].max(plane_zs[1]);
    let height = z_max - z_min;
    if radius > 0.0 && height > 1e-9 && radius.is_finite() && height.is_finite() {
        Some(SolidBody::Cylinder {
            base: Point3::new(wc[0], wc[1], z_min),
            radius,
            height,
        })
    } else {
        None
    }
}

fn try_read_cone(
    sat: &SatDocument,
    faces: &[acadrust::entities::acis::SatFace<'_>],
    rot: [[f64; 3]; 3],
    trans: [f64; 3],
    scale: f64,
) -> Option<SolidBody> {
    let mut cone_surf: Option<SatConeSurface<'_>> = None;
    let mut base_z: Option<f64> = None;
    for f in faces {
        let surf_rec = sat.resolve(f.surface())?;
        if let Some(cs) = SatConeSurface::from_record(surf_rec) {
            if cone_surf.is_some() {
                return None;
            }
            cone_surf = Some(cs);
        } else {
            let ps = SatPlaneSurface::from_record(surf_rec)?;
            let (nx, ny, nz) = ps.normal();
            let wn = vnorm(apply_rotation([nx, ny, nz], rot))?;
            if !is_z_axis(wn) {
                return None;
            }
            let (rx, ry, rz) = ps.root_point();
            let wr = apply_placement([rx, ry, rz], rot, trans, scale);
            base_z = Some(wr[2]);
        }
    }
    let cs = cone_surf?;
    let bz = base_z?;
    let sin_h = cs.sin_half_angle();
    let cos_h = cs.cos_half_angle();
    if sin_h.abs() <= 1e-6 || cos_h.abs() <= 1e-6 || (cs.ratio() - 1.0).abs() > 1e-4 {
        return None;
    }
    let (ax, ay, az) = cs.axis();
    let wa = vnorm(apply_rotation([ax, ay, az], rot))?;
    if !is_z_axis(wa) {
        return None;
    }
    let (cx, cy, cz) = cs.center();
    let wc = apply_placement([cx, cy, cz], rot, trans, scale);
    let radius = cs.radius() * scale;
    // Distance from cone center plane to apex along axis:
    // `r_at_z = radius + (z - cz) * (sin_h / cos_h) = 0` => `height = -radius * cos_h / sin_h`.
    let signed_h = -radius * cos_h / sin_h;
    let apex_z = wc[2] + signed_h * wa[2].signum();
    let height = apex_z - bz;
    if radius > 0.0 && height > 1e-9 && radius.is_finite() && height.is_finite() {
        Some(SolidBody::Cone {
            base: Point3::new(wc[0], wc[1], bz),
            radius,
            height,
        })
    } else {
        None
    }
}

fn read_planar_brep(
    sat: &SatDocument,
    faces: &[acadrust::entities::acis::SatFace<'_>],
    rot: [[f64; 3]; 3],
    trans: [f64; 3],
    scale: f64,
) -> Option<SolidBody> {
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut point_map: HashMap<usize, usize> = HashMap::new();
    let mut poly_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());

    for f in faces {
        let surf_rec = sat.resolve(f.surface())?;
        let plane = SatPlaneSurface::from_record(surf_rec)?;
        let (nx, ny, nz) = plane.normal();
        let mut expected_n = vnorm(apply_rotation([nx, ny, nz], rot))?;
        if f.sense() == Sense::Reversed {
            expected_n = [-expected_n[0], -expected_n[1], -expected_n[2]];
        }

        let first_loop_ptr = f.first_loop();
        if first_loop_ptr.is_null() {
            return None;
        }
        let loop_rec = sat.resolve(first_loop_ptr)?;
        let sat_loop = SatLoop::from_record(loop_rec)?;
        // Faces with inner hole loops are not representable as a single polygon ring.
        if !sat_loop.next_loop().is_null() {
            return None;
        }

        let start_co_ptr = sat_loop.first_coedge();
        if start_co_ptr.is_null() {
            return None;
        }
        let mut ring: Vec<usize> = Vec::new();
        let mut cur_ptr = start_co_ptr;
        let max_steps = sat.record_count().max(64);
        for _ in 0..max_steps {
            let co_rec = sat.resolve(cur_ptr)?;
            let co = SatCoedge::from_record(co_rec)?;
            let edge_rec = sat.resolve(co.edge())?;
            let edge = SatEdge::from_record(edge_rec)?;
            let curve_rec = sat.resolve(edge.curve())?;
            if curve_rec.entity_type != "straight-curve" {
                return None;
            }
            let forward = co.sense() == Sense::Forward;
            let start_v = if forward {
                edge.start_vertex()
            } else {
                edge.end_vertex()
            };
            let v_rec = sat.resolve(start_v)?;
            let vert = SatVertex::from_record(v_rec)?;
            let p_ptr = vert.point();
            let p_idx = p_ptr.index()?;
            let vi = if let Some(&idx) = point_map.get(&p_idx) {
                idx
            } else {
                let p_rec = sat.resolve(p_ptr)?;
                let pt = SatPoint::from_record(p_rec)?;
                let (x, y, z) = pt.position();
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return None;
                }
                let wp = apply_placement([x, y, z], rot, trans, scale);
                let idx = vertices.len();
                vertices.push(wp);
                point_map.insert(p_idx, idx);
                idx
            };
            if ring.last().copied() != Some(vi) {
                ring.push(vi);
            }
            let next_ptr = co.next();
            if next_ptr == start_co_ptr {
                break;
            }
            cur_ptr = next_ptr;
            if cur_ptr.is_null() {
                return None;
            }
        }
        if ring.len() >= 2 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            return None;
        }
        // Orient ring CCW relative to the face's outward surface normal.
        let mut poly_n = [0.0; 3];
        let v0 = vertices[ring[0]];
        for i in 1..(ring.len() - 1) {
            let c = vcross(vsub(vertices[ring[i]], v0), vsub(vertices[ring[i + 1]], v0));
            poly_n[0] += c[0];
            poly_n[1] += c[1];
            poly_n[2] += c[2];
        }
        if vdot(poly_n, expected_n) < 0.0 {
            ring.reverse();
        }
        poly_faces.push(ring);
    }

    if vertices.len() < 4 || poly_faces.len() < 4 {
        return None;
    }

    // Recognize axis-aligned box (6 quad faces, 8 vertices, exact corners of its AABB).
    if let Some(box_body) = try_recognize_axis_aligned_box(&vertices, &poly_faces) {
        return Some(box_body);
    }
    if let Some(wedge_body) = try_recognize_wedge(&vertices, &poly_faces) {
        return Some(wedge_body);
    }
    if let Some(pyr_body) = try_recognize_pyramid(&vertices, &poly_faces) {
        return Some(pyr_body);
    }

    Some(SolidBody::Mesh {
        vertices: vertices
            .into_iter()
            .map(|[x, y, z]| Point3::new(x, y, z))
            .collect(),
        faces: poly_faces,
    })
}

fn try_recognize_axis_aligned_box(
    vertices: &[[f64; 3]],
    faces: &[Vec<usize>],
) -> Option<SolidBody> {
    if vertices.len() != 8 || faces.len() != 6 || !faces.iter().all(|f| f.len() == 4) {
        return None;
    }
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for &v in vertices {
        for a in 0..3 {
            min[a] = min[a].min(v[a]);
            max[a] = max[a].max(v[a]);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    if size.iter().any(|&s| s <= TOL) {
        return None;
    }
    let eps = TOL * size.iter().copied().fold(1.0_f64, f64::max);
    // Every vertex must sit on one of the 8 distinct AABB corners.
    let mut mask = 0u8;
    for &v in vertices {
        let mut bit = 0u8;
        for a in 0..3 {
            if (v[a] - min[a]).abs() <= eps {
                // 0
            } else if (v[a] - max[a]).abs() <= eps {
                bit |= 1 << a;
            } else {
                return None;
            }
        }
        mask |= 1 << bit;
    }
    if mask == 0xFF {
        Some(SolidBody::Box {
            origin: Point3::new(min[0], min[1], min[2]),
            size,
        })
    } else {
        None
    }
}

fn try_recognize_wedge(vertices: &[[f64; 3]], faces: &[Vec<usize>]) -> Option<SolidBody> {
    if vertices.len() != 6 || faces.len() != 5 {
        return None;
    }
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for &v in vertices {
        for a in 0..3 {
            min[a] = min[a].min(v[a]);
            max[a] = max[a].max(v[a]);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    if size.iter().any(|&s| s <= TOL) {
        return None;
    }
    let expected = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [min[0], max[1], max[2]],
    ];
    let eps = TOL * size.iter().copied().fold(1.0_f64, f64::max);
    let matched = expected.iter().all(|e| {
        vertices.iter().any(|v| {
            (v[0] - e[0]).abs() <= eps && (v[1] - e[1]).abs() <= eps && (v[2] - e[2]).abs() <= eps
        })
    });
    matched.then(|| SolidBody::Wedge {
        origin: Point3::new(min[0], min[1], min[2]),
        size,
    })
}

fn try_recognize_pyramid(vertices: &[[f64; 3]], faces: &[Vec<usize>]) -> Option<SolidBody> {
    if vertices.len() != 5 || faces.len() != 5 {
        return None;
    }
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for &v in vertices {
        for a in 0..3 {
            min[a] = min[a].min(v[a]);
            max[a] = max[a].max(v[a]);
        }
    }
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    if dx <= TOL || dy <= TOL || dz <= TOL || (dx - dy).abs() > TOL * dx.max(1.0) {
        return None;
    }
    let cx = 0.5 * (min[0] + max[0]);
    let cy = 0.5 * (min[1] + max[1]);
    let expected = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [cx, cy, max[2]],
    ];
    let eps = TOL * dx.max(dz).max(1.0);
    let matched = expected.iter().all(|e| {
        vertices.iter().any(|v| {
            (v[0] - e[0]).abs() <= eps && (v[1] - e[1]).abs() <= eps && (v[2] - e[2]).abs() <= eps
        })
    });
    matched.then(|| SolidBody::Pyramid {
        base: Point3::new(cx, cy, min[2]),
        base_size: dx,
        height: dz,
    })
}
