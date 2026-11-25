//! Pure-Rust 3D solid geometry kernel: polyhedron builders (`box`, `cylinder`,
//! `cone`, `sphere`, `torus`, `wedge`, `pyramid`, `extrude`, `revolve`),
//! 3D BSP CSG booleans (`union`, `subtract`, `intersect`) with T-junction
//! elimination for strict closed 2-manifold output, mass properties (volume,
//! surface area, 3D bounding box), and 2D orthographic wireframe + silhouette
//! projection (ADR-0011, ADR-0012).

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::{PI, TAU};

use crate::error::{Error, ErrorCode};
use crate::model::{BooleanOp, Point3, SolidBody};

/// Number of angular facets used when tessellating curved surfaces (cylinders,
/// cones, spheres, tori, full revolves) for CSG booleans and planar B-rep
/// export. 32 gives ~11.25° steps (smooth visually while keeping SAT/SAB compact).
pub const DEFAULT_CIRCLE_SEGMENTS: usize = 32;

const EPS: f64 = 1e-6;
const WELD_QUANT: f64 = 1e6;

/// A closed 2-manifold polyhedron with outward-CCW planar polygon faces.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyhedron {
    pub vertices: Vec<[f64; 3]>,
    pub faces: Vec<Vec<usize>>,
}

/// Computed 3D properties of a solid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolidMetrics {
    pub volume: f64,
    pub surface_area: f64,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// Orthonormal orthographic projection camera matching AutoCAD's `VIEWDIR`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrthoView {
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub forward: [f64; 3],
}

impl OrthoView {
    /// Standard top-down plan view (`+Z`).
    pub const TOP: Self = Self {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
    };

    /// Builds an AutoCAD-convention orthographic camera looking from `+dir`
    /// toward the origin.
    pub fn from_direction(dir: Point3) -> Self {
        let f = normalize([dir.x, dir.y, dir.z]);
        if len(f) < 1e-12 {
            return Self::TOP;
        }
        // Looking straight along +Z or -Z: keep +X right (and +Y up for +Z, -Y up for -Z).
        if f[0].abs() < 1e-9 && f[1].abs() < 1e-9 {
            return if f[2] >= 0.0 {
                Self::TOP
            } else {
                Self {
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, -1.0, 0.0],
                    forward: [0.0, 0.0, -1.0],
                }
            };
        }
        // World +Z up: right = normalize(Z × forward), up = forward × right.
        let right = normalize(cross([0.0, 0.0, 1.0], f));
        let up = normalize(cross(f, right));
        Self {
            right,
            up,
            forward: f,
        }
    }

    /// Projects a 3D world point to 2D view coordinates.
    #[inline]
    pub fn project(&self, p: [f64; 3]) -> [f64; 2] {
        [dot(p, self.right), dot(p, self.up)]
    }

    /// Whether this view is the standard top-down XY plan view.
    #[inline]
    pub fn is_top(&self) -> bool {
        (self.forward[0]).abs() < 1e-9
            && (self.forward[1]).abs() < 1e-9
            && (self.forward[2] - 1.0).abs() < 1e-9
    }
}

// ── Vector math helpers ──────────────────────────────────────────────────────

#[inline]
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[inline]
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = len(a);
    if l > 1e-12 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[inline]
fn p3(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn rotate_around_axis(pt: [f64; 3], origin: [f64; 3], axis: [f64; 3], angle_rad: f64) -> [f64; 3] {
    let v = sub(pt, origin);
    let c = angle_rad.cos();
    let s = angle_rad.sin();
    let k_cross_v = cross(axis, v);
    let k_dot_v = dot(axis, v);
    add(
        origin,
        add(
            add(scale(v, c), scale(k_cross_v, s)),
            scale(axis, k_dot_v * (1.0 - c)),
        ),
    )
}

fn face_normal(verts: &[[f64; 3]], ring: &[usize]) -> [f64; 3] {
    // Newell's method for robust planar polygon normal.
    let mut n = [0.0; 3];
    for i in 0..ring.len() {
        let a = verts[ring[i]];
        let b = verts[ring[(i + 1) % ring.len()]];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    normalize(n)
}

// ── Public evaluation API ────────────────────────────────────────────────────

/// Evaluates any [`SolidBody`] into a closed 2-manifold [`Polyhedron`].
pub fn evaluate_polyhedron(body: &SolidBody) -> Result<Polyhedron, Error> {
    match body {
        SolidBody::Box { origin, size } => Ok(box_polyhedron(p3(*origin), *size)),
        SolidBody::Cylinder {
            base,
            radius,
            height,
        } => Ok(cylinder_polyhedron(
            p3(*base),
            *radius,
            *height,
            DEFAULT_CIRCLE_SEGMENTS,
        )),
        SolidBody::Cone {
            base,
            radius,
            height,
        } => Ok(cone_polyhedron(
            p3(*base),
            *radius,
            *height,
            DEFAULT_CIRCLE_SEGMENTS,
        )),
        SolidBody::Sphere { center, radius } => Ok(sphere_polyhedron(p3(*center), *radius, 12, 24)),
        SolidBody::Torus {
            center,
            major_radius,
            minor_radius,
        } => Ok(torus_polyhedron(
            p3(*center),
            *major_radius,
            *minor_radius,
            24,
            14,
        )),
        SolidBody::Wedge { origin, size } => Ok(wedge_polyhedron(p3(*origin), *size)),
        SolidBody::Pyramid {
            base,
            base_size,
            height,
        } => Ok(pyramid_polyhedron(p3(*base), *base_size, *height)),
        SolidBody::Extrude {
            profile,
            bulges,
            height,
            direction,
            taper_angle,
        } => extrude_polyhedron(profile, bulges, *height, *direction, *taper_angle),
        SolidBody::Revolve {
            profile,
            bulges,
            axis_origin,
            axis_direction,
            angle,
        } => revolve_polyhedron(profile, bulges, *axis_origin, *axis_direction, *angle),
        SolidBody::Boolean { op, operands } => {
            if operands.len() < 2 {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "a `{}` solid needs at least two operands, found {}",
                        op.name(),
                        operands.len()
                    ),
                ));
            }
            let mut acc = evaluate_polyhedron(&operands[0])?;
            for operand in &operands[1..] {
                let rhs = evaluate_polyhedron(operand)?;
                acc = csg_boolean(*op, &acc, &rhs)?;
            }
            Ok(acc)
        }
        SolidBody::Mesh { vertices, faces } => {
            let verts: Vec<[f64; 3]> = vertices.iter().map(|&p| p3(p)).collect();
            let poly = Polyhedron {
                vertices: verts,
                faces: faces.clone(),
            };
            validate_polyhedron(&poly)?;
            Ok(poly)
        }
    }
}

/// Computes exact (for analytic primitives) or polyhedral volume, surface area,
/// and 3D axis-aligned bounding box of `body`.
pub fn solid_metrics(body: &SolidBody) -> SolidMetrics {
    match body {
        SolidBody::Box { origin, size } => {
            let [l, w, h] = *size;
            SolidMetrics {
                volume: (l * w * h).abs(),
                surface_area: 2.0 * (l * w + w * h + h * l),
                min: [origin.x, origin.y, origin.z],
                max: [origin.x + l, origin.y + w, origin.z + h],
            }
        }
        SolidBody::Cylinder {
            base,
            radius,
            height,
        } => {
            let r = *radius;
            let h = *height;
            SolidMetrics {
                volume: PI * r * r * h,
                surface_area: TAU * r * (r + h),
                min: [base.x - r, base.y - r, base.z],
                max: [base.x + r, base.y + r, base.z + h],
            }
        }
        SolidBody::Cone {
            base,
            radius,
            height,
        } => {
            let r = *radius;
            let h = *height;
            let slant = r.hypot(h);
            SolidMetrics {
                volume: PI * r * r * h / 3.0,
                surface_area: PI * r * (r + slant),
                min: [base.x - r, base.y - r, base.z],
                max: [base.x + r, base.y + r, base.z + h],
            }
        }
        SolidBody::Sphere { center, radius } => {
            let r = *radius;
            SolidMetrics {
                volume: (4.0 / 3.0) * PI * r * r * r,
                surface_area: 4.0 * PI * r * r,
                min: [center.x - r, center.y - r, center.z - r],
                max: [center.x + r, center.y + r, center.z + r],
            }
        }
        SolidBody::Torus {
            center,
            major_radius,
            minor_radius,
        } => {
            let rm = *major_radius;
            let rn = *minor_radius;
            let r = rm + rn;
            SolidMetrics {
                volume: 2.0 * PI * PI * rm * rn * rn,
                surface_area: 4.0 * PI * PI * rm * rn,
                min: [center.x - r, center.y - r, center.z - rn],
                max: [center.x + r, center.y + r, center.z + rn],
            }
        }
        _ => match evaluate_polyhedron(body) {
            Ok(poly) => polyhedron_metrics(&poly),
            Err(_) => SolidMetrics {
                volume: 0.0,
                surface_area: 0.0,
                min: [0.0; 3],
                max: [0.0; 3],
            },
        },
    }
}

pub fn polyhedron_metrics(poly: &Polyhedron) -> SolidMetrics {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for v in &poly.vertices {
        for k in 0..3 {
            min[k] = min[k].min(v[k]);
            max[k] = max[k].max(v[k]);
        }
    }
    if poly.vertices.is_empty() {
        min = [0.0; 3];
        max = [0.0; 3];
    }
    let mut vol6 = 0.0;
    let mut area = 0.0;
    for face in &poly.faces {
        if face.len() < 3 {
            continue;
        }
        let v0 = poly.vertices[face[0]];
        for i in 1..(face.len() - 1) {
            let v1 = poly.vertices[face[i]];
            let v2 = poly.vertices[face[i + 1]];
            vol6 += dot(v0, cross(v1, v2));
            area += 0.5 * len(cross(sub(v1, v0), sub(v2, v0)));
        }
    }
    SolidMetrics {
        volume: (vol6 / 6.0).abs(),
        surface_area: area,
        min,
        max,
    }
}

/// Computes 3D wireframe polylines (each polyline is a sequence of 3D points)
/// representing the visible feature edges and view-dependent silhouettes of
/// `body` as seen from `view` (matching AutoCAD's `2D Wireframe` display/plot).
pub fn wireframe_polylines_3d(body: &SolidBody, view: &OrthoView) -> Vec<Vec<[f64; 3]>> {
    match body {
        SolidBody::Cylinder {
            base,
            radius,
            height,
        } => cylinder_wireframe(p3(*base), *radius, *height, view),
        SolidBody::Cone {
            base,
            radius,
            height,
        } => cone_wireframe(p3(*base), *radius, *height, view),
        SolidBody::Sphere { center, radius } => sphere_wireframe(p3(*center), *radius, view),
        SolidBody::Torus {
            center,
            major_radius,
            minor_radius,
        } => torus_wireframe(p3(*center), *major_radius, *minor_radius, view),
        _ => match evaluate_polyhedron(body) {
            Ok(poly) => polyhedron_wireframe(&poly, view),
            Err(_) => Vec::new(),
        },
    }
}

/// Computes projected 2D wireframe strokes for `body` under `view`.
pub fn wireframe_strokes_2d(body: &SolidBody, view: &OrthoView) -> Vec<Vec<[f64; 2]>> {
    wireframe_polylines_3d(body, view)
        .into_iter()
        .map(|pts| pts.into_iter().map(|p| view.project(p)).collect())
        .collect()
}

/// Returns distinct 3D corner/key points of `body` (for object snaps).
pub fn solid_key_points(body: &SolidBody) -> (Vec<Point3>, Point3) {
    let m = solid_metrics(body);
    let center = Point3::new(
        0.5 * (m.min[0] + m.max[0]),
        0.5 * (m.min[1] + m.max[1]),
        0.5 * (m.min[2] + m.max[2]),
    );
    match body {
        SolidBody::Cylinder {
            base,
            radius: _,
            height,
        } => (
            vec![*base, Point3::new(base.x, base.y, base.z + height)],
            center,
        ),
        SolidBody::Cone { base, height, .. } => (
            vec![*base, Point3::new(base.x, base.y, base.z + height)],
            center,
        ),
        SolidBody::Sphere { center: c, .. } | SolidBody::Torus { center: c, .. } => (vec![*c], *c),
        _ => match evaluate_polyhedron(body) {
            Ok(poly) => (
                poly.vertices
                    .into_iter()
                    .map(|[x, y, z]| Point3::new(x, y, z))
                    .collect(),
                center,
            ),
            Err(_) => (Vec::new(), center),
        },
    }
}

/// Applies an arbitrary 3D vertex transform (`p -> f(p)`) to `body`, returning
/// a `Mesh` body when exact primitive parameters cannot represent the transform
/// (for example, rotating an axis-aligned box by 30°).
pub fn transform_solid_body(
    body: &SolidBody,
    f: impl Fn([f64; 3]) -> [f64; 3],
    flips_orientation: bool,
) -> Result<SolidBody, Error> {
    let mut poly = evaluate_polyhedron(body)?;
    for v in &mut poly.vertices {
        *v = f(*v);
    }
    if flips_orientation {
        for face in &mut poly.faces {
            face.reverse();
        }
    }
    Ok(SolidBody::Mesh {
        vertices: poly
            .vertices
            .into_iter()
            .map(|[x, y, z]| Point3::new(x, y, z))
            .collect(),
        faces: poly.faces,
    })
}

// ── Wireframe helpers ────────────────────────────────────────────────────────

fn sample_circle_3d(center: [f64; 3], u: [f64; 3], v: [f64; 3], r: f64, n: usize) -> Vec<[f64; 3]> {
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let a = (i as f64 / n as f64) * TAU;
        let p = add(center, add(scale(u, r * a.cos()), scale(v, r * a.sin())));
        pts.push(p);
    }
    pts
}

fn cylinder_wireframe(
    base: [f64; 3],
    radius: f64,
    height: f64,
    view: &OrthoView,
) -> Vec<Vec<[f64; 3]>> {
    let n = 48;
    let top = [base[0], base[1], base[2] + height];
    let mut lines = vec![
        sample_circle_3d(base, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], radius, n),
        sample_circle_3d(top, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], radius, n),
    ];
    // Silhouette generator lines perpendicular to view direction in XY plane.
    let horiz = [view.forward[0], view.forward[1], 0.0];
    if len(horiz) > 1e-6 {
        let perp = normalize([-horiz[1], horiz[0], 0.0]);
        for s in [-1.0, 1.0] {
            let offset = scale(perp, s * radius);
            lines.push(vec![add(base, offset), add(top, offset)]);
        }
    }
    lines
}

fn cone_wireframe(
    base: [f64; 3],
    radius: f64,
    height: f64,
    view: &OrthoView,
) -> Vec<Vec<[f64; 3]>> {
    let n = 48;
    let apex = [base[0], base[1], base[2] + height];
    let mut lines = vec![sample_circle_3d(
        base,
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        radius,
        n,
    )];
    let horiz = [view.forward[0], view.forward[1], 0.0];
    if len(horiz) > 1e-6 {
        let perp = normalize([-horiz[1], horiz[0], 0.0]);
        for s in [-1.0, 1.0] {
            let offset = scale(perp, s * radius);
            lines.push(vec![add(base, offset), apex]);
        }
    }
    lines
}

fn sphere_wireframe(center: [f64; 3], radius: f64, view: &OrthoView) -> Vec<Vec<[f64; 3]>> {
    let n = 48;
    vec![
        // Exact silhouette great circle perpendicular to view direction.
        sample_circle_3d(center, view.right, view.up, radius, n),
        // Equator (matches AutoCAD's default isolines/silhouette clarity).
        sample_circle_3d(center, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], radius, n),
    ]
}

fn torus_wireframe(
    center: [f64; 3],
    major: f64,
    minor: f64,
    view: &OrthoView,
) -> Vec<Vec<[f64; 3]>> {
    let n = 48;
    let mut lines = vec![
        sample_circle_3d(center, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], major + minor, n),
        sample_circle_3d(
            [center[0], center[1], center[2] + minor],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            major,
            n,
        ),
        sample_circle_3d(
            [center[0], center[1], center[2] - minor],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            major,
            n,
        ),
    ];
    if major > minor + 1e-9 {
        lines.push(sample_circle_3d(
            center,
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            major - minor,
            n,
        ));
    }
    if !view.is_top() {
        for &angle in &[0.0, PI * 0.5, PI, PI * 1.5] {
            let radial = [angle.cos(), angle.sin(), 0.0];
            let tube_center = add(center, scale(radial, major));
            lines.push(sample_circle_3d(
                tube_center,
                radial,
                [0.0, 0.0, 1.0],
                minor,
                24,
            ));
        }
    }
    lines
}

fn polyhedron_wireframe(poly: &Polyhedron, view: &OrthoView) -> Vec<Vec<[f64; 3]>> {
    let normals: Vec<[f64; 3]> = poly
        .faces
        .iter()
        .map(|f| face_normal(&poly.vertices, f))
        .collect();
    // Map undirected edge (min, max) -> list of adjacent face indices.
    let mut edge_faces: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (fi, face) in poly.faces.iter().enumerate() {
        let n = face.len();
        for i in 0..n {
            let a = face[i];
            let b = face[(i + 1) % n];
            let key = if a < b { (a, b) } else { (b, a) };
            edge_faces.entry(key).or_default().push(fi);
        }
    }
    // Emit sharp feature edges (dihedral > ~5 deg, dot < 0.996) AND silhouette
    // edges (front-facing vs back-facing relative to view.forward), suppressing
    // coplanar triangulation diagonals.
    let mut out = Vec::new();
    for ((a, b), faces) in edge_faces {
        let keep = match faces.as_slice() {
            [f0, f1] => {
                let n0 = normals[*f0];
                let n1 = normals[*f1];
                let crease = dot(n0, n1) < 0.996;
                let d0 = dot(n0, view.forward);
                let d1 = dot(n1, view.forward);
                let silhouette = (d0 >= -1e-9 && d1 < -1e-9) || (d1 >= -1e-9 && d0 < -1e-9);
                crease || silhouette
            }
            _ => true,
        };
        if keep {
            out.push(vec![poly.vertices[a], poly.vertices[b]]);
        }
    }
    out
}

// ── Primitive polyhedron builders ────────────────────────────────────────────

pub fn box_polyhedron(origin: [f64; 3], size: [f64; 3]) -> Polyhedron {
    let [x, y, z] = origin;
    let [l, w, h] = size;
    let vertices = vec![
        [x, y, z],
        [x + l, y, z],
        [x + l, y + w, z],
        [x, y + w, z],
        [x, y, z + h],
        [x + l, y, z + h],
        [x + l, y + w, z + h],
        [x, y + w, z + h],
    ];
    let faces = vec![
        vec![0, 3, 2, 1], // bottom (-Z)
        vec![4, 5, 6, 7], // top (+Z)
        vec![0, 1, 5, 4], // front (-Y)
        vec![2, 3, 7, 6], // back (+Y)
        vec![1, 2, 6, 5], // right (+X)
        vec![3, 0, 4, 7], // left (-X)
    ];
    Polyhedron { vertices, faces }
}

pub fn wedge_polyhedron(origin: [f64; 3], size: [f64; 3]) -> Polyhedron {
    let [x, y, z] = origin;
    let [l, w, h] = size;
    let vertices = vec![
        [x, y, z],
        [x + l, y, z],
        [x, y + w, z],
        [x, y, z + h],
        [x + l, y, z + h],
        [x, y + w, z + h],
    ];
    let faces = vec![
        vec![0, 2, 1],    // bottom
        vec![3, 4, 5],    // top
        vec![0, 1, 4, 3], // front
        vec![2, 0, 3, 5], // left
        vec![1, 2, 5, 4], // hypotenuse
    ];
    Polyhedron { vertices, faces }
}

pub fn pyramid_polyhedron(base: [f64; 3], base_size: f64, height: f64) -> Polyhedron {
    let [cx, cy, cz] = base;
    let hb = base_size * 0.5;
    let vertices = vec![
        [cx - hb, cy - hb, cz],
        [cx + hb, cy - hb, cz],
        [cx + hb, cy + hb, cz],
        [cx - hb, cy + hb, cz],
        [cx, cy, cz + height],
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![0, 1, 4],
        vec![1, 2, 4],
        vec![2, 3, 4],
        vec![3, 0, 4],
    ];
    Polyhedron { vertices, faces }
}

pub fn cylinder_polyhedron(
    base: [f64; 3],
    radius: f64,
    height: f64,
    segments: usize,
) -> Polyhedron {
    let n = segments.max(8);
    let [cx, cy, cz] = base;
    let mut vertices = Vec::with_capacity(2 * n);
    for i in 0..n {
        let a = (i as f64 / n as f64) * TAU;
        vertices.push([cx + radius * a.cos(), cy + radius * a.sin(), cz]);
    }
    for i in 0..n {
        let a = (i as f64 / n as f64) * TAU;
        vertices.push([cx + radius * a.cos(), cy + radius * a.sin(), cz + height]);
    }
    let mut faces = Vec::with_capacity(n + 2);
    // Bottom cap (outward -Z -> clockwise when viewed from +Z)
    faces.push((0..n).rev().collect());
    // Top cap (outward +Z -> CCW)
    faces.push((n..2 * n).collect());
    // Side quads
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, n + j, n + i]);
    }
    Polyhedron { vertices, faces }
}

pub fn cone_polyhedron(base: [f64; 3], radius: f64, height: f64, segments: usize) -> Polyhedron {
    let n = segments.max(8);
    let [cx, cy, cz] = base;
    let mut vertices = Vec::with_capacity(n + 1);
    for i in 0..n {
        let a = (i as f64 / n as f64) * TAU;
        vertices.push([cx + radius * a.cos(), cy + radius * a.sin(), cz]);
    }
    let apex = n;
    vertices.push([cx, cy, cz + height]);
    let mut faces = Vec::with_capacity(n + 1);
    faces.push((0..n).rev().collect());
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, apex]);
    }
    Polyhedron { vertices, faces }
}

pub fn sphere_polyhedron(
    center: [f64; 3],
    radius: f64,
    stacks: usize,
    slices: usize,
) -> Polyhedron {
    let stacks = stacks.max(6);
    let slices = slices.max(12);
    let [cx, cy, cz] = center;
    let mut vertices = Vec::new();
    let south = 0usize;
    vertices.push([cx, cy, cz - radius]);
    for s in 1..stacks {
        let phi = -PI * 0.5 + (s as f64 / stacks as f64) * PI;
        let cp = phi.cos();
        let sp = phi.sin();
        for l in 0..slices {
            let theta = (l as f64 / slices as f64) * TAU;
            vertices.push([
                cx + radius * cp * theta.cos(),
                cy + radius * cp * theta.sin(),
                cz + radius * sp,
            ]);
        }
    }
    let north = vertices.len();
    vertices.push([cx, cy, cz + radius]);

    let ring_idx = |s: usize, l: usize| 1 + (s - 1) * slices + (l % slices);
    let mut faces = Vec::new();
    // South pole fan
    for l in 0..slices {
        faces.push(vec![south, ring_idx(1, l + 1), ring_idx(1, l)]);
    }
    // Middle bands
    for s in 1..(stacks - 1) {
        for l in 0..slices {
            let a = ring_idx(s, l);
            let b = ring_idx(s, l + 1);
            let c = ring_idx(s + 1, l + 1);
            let d = ring_idx(s + 1, l);
            faces.push(vec![a, b, c]);
            faces.push(vec![a, c, d]);
        }
    }
    // North pole fan
    for l in 0..slices {
        faces.push(vec![
            ring_idx(stacks - 1, l),
            ring_idx(stacks - 1, l + 1),
            north,
        ]);
    }
    Polyhedron { vertices, faces }
}

pub fn torus_polyhedron(
    center: [f64; 3],
    major: f64,
    minor: f64,
    seg_u: usize,
    seg_v: usize,
) -> Polyhedron {
    let nu = seg_u.max(12);
    let nv = seg_v.max(8);
    let [cx, cy, cz] = center;
    let mut vertices = Vec::with_capacity(nu * nv);
    for iu in 0..nu {
        let u = (iu as f64 / nu as f64) * TAU;
        let cu = u.cos();
        let su = u.sin();
        for iv in 0..nv {
            let v = (iv as f64 / nv as f64) * TAU;
            let r = major + minor * v.cos();
            vertices.push([cx + r * cu, cy + r * su, cz + minor * v.sin()]);
        }
    }
    let idx = |iu: usize, iv: usize| (iu % nu) * nv + (iv % nv);
    let mut faces = Vec::with_capacity(nu * nv * 2);
    for iu in 0..nu {
        for iv in 0..nv {
            let a = idx(iu, iv);
            let b = idx(iu + 1, iv);
            let c = idx(iu + 1, iv + 1);
            let d = idx(iu, iv + 1);
            faces.push(vec![a, b, c]);
            faces.push(vec![a, c, d]);
        }
    }
    Polyhedron { vertices, faces }
}

// ── Profile expansion (bulge arcs -> chords) ─────────────────────────────────

fn expand_closed_profile(profile: &[Point3], bulges: &[f64]) -> Result<Vec<[f64; 3]>, Error> {
    if profile.len() < 3 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a 3D solid profile needs at least 3 vertices",
        ));
    }
    let mut pts: Vec<[f64; 3]> = Vec::with_capacity(profile.len());
    // Compute plane normal first from the control vertices.
    let raw: Vec<[f64; 3]> = profile.iter().map(|&p| p3(p)).collect();
    let mut plane_n = face_normal(&raw, &(0..raw.len()).collect::<Vec<_>>());
    if len(plane_n) < 1e-9 {
        plane_n = [0.0, 0.0, 1.0];
    }
    for i in 0..raw.len() {
        let a = raw[i];
        let b = raw[(i + 1) % raw.len()];
        pts.push(a);
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        if bulge.abs() > 1e-9 {
            let chord = sub(b, a);
            let chord_len = len(chord);
            if chord_len > 1e-9 {
                let sweep = 4.0 * bulge.atan();
                let steps = ((sweep.abs() / (PI / 8.0)).ceil() as usize).clamp(4, 32);
                let left = normalize(cross(plane_n, chord));
                let half = chord_len * 0.5;
                let sag = half * (1.0 - bulge * bulge) / (2.0 * bulge);
                let mid = scale(add(a, b), 0.5);
                let center = add(mid, scale(left, sag));
                for k in 1..steps {
                    let ang = sweep * (k as f64 / steps as f64);
                    pts.push(rotate_around_axis(a, center, plane_n, ang));
                }
            }
        }
    }
    // Deduplicate consecutive identical points.
    let mut clean: Vec<[f64; 3]> = Vec::with_capacity(pts.len());
    for p in pts {
        if clean.last().is_none_or(|&prev| len(sub(p, prev)) > 1e-7) {
            clean.push(p);
        }
    }
    if clean.len() >= 2 && len(sub(*clean.last().unwrap(), clean[0])) <= 1e-7 {
        clean.pop();
    }
    if clean.len() < 3 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the closed profile collapses to fewer than 3 distinct points",
        ));
    }
    Ok(clean)
}

/// Triangulates a simple planar 3D polygon (assumed CCW relative to `normal`)
/// via ear clipping, returning index triples into `ring`.
fn triangulate_simple_polygon(
    pts: &[[f64; 3]],
    ring: &[usize],
    normal: [f64; 3],
) -> Vec<[usize; 3]> {
    let n = ring.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[ring[0], ring[1], ring[2]]];
    }
    // Project onto an orthonormal 2D basis (u, v) in the face plane.
    let mut u = normalize(sub(pts[ring[1]], pts[ring[0]]));
    if len(u) < 1e-9 {
        u = if normal[0].abs() < 0.9 {
            normalize(cross([1.0, 0.0, 0.0], normal))
        } else {
            normalize(cross([0.0, 1.0, 0.0], normal))
        };
    }
    let v = normalize(cross(normal, u));
    let origin = pts[ring[0]];
    let proj = |idx: usize| -> [f64; 2] {
        let d = sub(pts[idx], origin);
        [dot(d, u), dot(d, v)]
    };

    let mut rem: Vec<usize> = ring.to_vec();
    let mut tris = Vec::with_capacity(n - 2);
    let mut guard = 0usize;
    while rem.len() > 3 && guard < rem.len() * rem.len() + 16 {
        guard += 1;
        let m = rem.len();
        let mut clipped = false;
        for i in 0..m {
            let i_prev = rem[(i + m - 1) % m];
            let i_curr = rem[i];
            let i_next = rem[(i + 1) % m];
            let a = proj(i_prev);
            let b = proj(i_curr);
            let c = proj(i_next);
            let area2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area2 <= 1e-10 {
                continue;
            }
            let mut has_inside = false;
            for &other in &rem {
                if other == i_prev || other == i_curr || other == i_next {
                    continue;
                }
                let p = proj(other);
                if point_in_triangle_2d(p, a, b, c) {
                    has_inside = true;
                    break;
                }
            }
            if !has_inside {
                tris.push([i_prev, i_curr, i_next]);
                rem.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if rem.len() == 3 {
        tris.push([rem[0], rem[1], rem[2]]);
    } else if rem.len() > 3 {
        // Fallback fan if nearly collinear.
        for i in 1..(rem.len() - 1) {
            tris.push([rem[0], rem[i], rem[i + 1]]);
        }
    }
    tris
}

fn point_in_triangle_2d(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let c1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let c2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
    let c3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);
    c1 >= -1e-9 && c2 >= -1e-9 && c3 >= -1e-9
}

// ── Extrude & Revolve builders ───────────────────────────────────────────────

pub fn extrude_polyhedron(
    profile: &[Point3],
    bulges: &[f64],
    height: f64,
    direction: Option<Point3>,
    taper_angle_deg: f64,
) -> Result<Polyhedron, Error> {
    if !height.is_finite() || height.abs() < 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude `height` must be a non-zero finite number",
        ));
    }
    if !taper_angle_deg.is_finite() || taper_angle_deg.abs() >= 89.0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude `taper_angle` must be strictly between -89 and 89 degrees",
        ));
    }
    let mut base = expand_closed_profile(profile, bulges)?;
    let n = base.len();
    let dir_raw = direction.map(p3).unwrap_or([0.0, 0.0, 1.0]);
    let mut dir = normalize(dir_raw);
    if len(dir) < 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude `direction` cannot be zero",
        ));
    }
    let mut h = height;
    if h < 0.0 {
        h = -h;
        dir = scale(dir, -1.0);
    }
    let mut prof_n = face_normal(&base, &(0..n).collect::<Vec<_>>());
    if len(prof_n) < 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude profile is degenerate (collinear points)",
        ));
    }
    let align = dot(prof_n, dir);
    if align.abs() < 1e-6 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude direction lies in the profile plane",
        ));
    }
    // Ensure base loop is CCW as seen from `+dir` (so `prof_n · dir > 0`).
    if align < 0.0 {
        base.reverse();
        prof_n = scale(prof_n, -1.0);
    }

    let delta = scale(dir, h);
    let inset = h * taper_angle_deg.to_radians().tan();
    let mut top = Vec::with_capacity(n);
    for i in 0..n {
        let p_prev = base[(i + n - 1) % n];
        let p_curr = base[i];
        let p_next = base[(i + 1) % n];
        let mut shifted = add(p_curr, delta);
        if inset.abs() > 1e-9 {
            let e1 = normalize(sub(p_curr, p_prev));
            let e2 = normalize(sub(p_next, p_curr));
            // Inward normals in the profile plane (CCW loop -> inward = prof_n × edge).
            let n1 = normalize(cross(prof_n, e1));
            let n2 = normalize(cross(prof_n, e2));
            let bis = add(n1, n2);
            let denom = dot(bis, n1);
            let offset = if denom.abs() > 1e-6 {
                scale(bis, inset / denom)
            } else {
                scale(n1, inset)
            };
            shifted = add(shifted, offset);
        }
        top.push(shifted);
    }

    // Verify top didn't self-invert under excessive taper.
    let top_n = face_normal(&top, &(0..n).collect::<Vec<_>>());
    if dot(top_n, dir) <= 1e-6 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "extrude `taper_angle` collapses or inverts the top profile",
        ));
    }

    let mut vertices = Vec::with_capacity(2 * n);
    vertices.extend_from_slice(&base);
    vertices.extend_from_slice(&top);

    let mut faces = Vec::new();
    // Bottom cap (outward normal = -prof_n -> reversed winding)
    let bot_ring: Vec<usize> = (0..n).rev().collect();
    if is_convex_planar(&vertices, &bot_ring, scale(prof_n, -1.0)) {
        faces.push(bot_ring);
    } else {
        for tri in triangulate_simple_polygon(&vertices, &bot_ring, scale(prof_n, -1.0)) {
            faces.push(tri.to_vec());
        }
    }
    // Top cap (outward normal = +prof_n -> CCW winding)
    let top_ring: Vec<usize> = (n..2 * n).collect();
    if is_convex_planar(&vertices, &top_ring, prof_n) {
        faces.push(top_ring);
    } else {
        for tri in triangulate_simple_polygon(&vertices, &top_ring, prof_n) {
            faces.push(tri.to_vec());
        }
    }
    // Side walls (planarity holds for untapered / constant-taper prism; triangulate if skewed)
    for i in 0..n {
        let j = (i + 1) % n;
        let quad = [i, j, n + j, n + i];
        if is_planar_quad(&vertices, quad) {
            faces.push(quad.to_vec());
        } else {
            faces.push(vec![quad[0], quad[1], quad[2]]);
            faces.push(vec![quad[0], quad[2], quad[3]]);
        }
    }

    let poly = Polyhedron { vertices, faces };
    validate_polyhedron(&poly)?;
    Ok(poly)
}

pub fn revolve_polyhedron(
    profile: &[Point3],
    bulges: &[f64],
    axis_origin: Point3,
    axis_direction: Point3,
    angle_deg: f64,
) -> Result<Polyhedron, Error> {
    if !angle_deg.is_finite() || angle_deg <= 0.0 || angle_deg > 360.0 + 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "revolve `angle` must be in (0, 360] degrees",
        ));
    }
    let axis = normalize(p3(axis_direction));
    if len(axis) < 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "revolve `axis_direction` cannot be zero",
        ));
    }
    let origin = p3(axis_origin);
    let mut prof = expand_closed_profile(profile, bulges)?;
    let m = prof.len();
    let full = (angle_deg - 360.0).abs() < 1e-6;
    let sweep_rad = if full { TAU } else { angle_deg.to_radians() };
    let steps = ((angle_deg / 11.25).ceil() as usize).clamp(8, 64);

    // Ensure profile lies strictly on one side of the axis (or touches axis at vertices)
    // and determine its winding relative to the sweep tangent direction at the profile centroid.
    let centroid = scale(
        prof.iter().fold([0.0; 3], |acc, &p| add(acc, p)),
        1.0 / m as f64,
    );
    let radial = sub(
        sub(centroid, origin),
        scale(axis, dot(sub(centroid, origin), axis)),
    );
    if len(radial) < 1e-7 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "revolve profile centroid lies on the axis of revolution",
        ));
    }
    let sweep_tangent = normalize(cross(axis, radial));
    let prof_n = face_normal(&prof, &(0..m).collect::<Vec<_>>());
    if len(prof_n) < 1e-9 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "revolve profile is degenerate (collinear points)",
        ));
    }
    // We want the sweep to move from ring k to ring k+1 in `+sweep_tangent`,
    // so the start cap faces `-sweep_tangent` and the end cap faces `+sweep_tangent`.
    // Making `prof` CCW as seen from `+sweep_tangent` ensures side quads `(a0, b0, b1, a1)`
    // have outward normals!
    if dot(prof_n, sweep_tangent) < 0.0 {
        prof.reverse();
    }

    // Track which profile vertices sit right on the revolution axis (shared across all rings).
    let on_axis: Vec<bool> = prof
        .iter()
        .map(|&p| {
            let d = sub(p, origin);
            let proj = scale(axis, dot(d, axis));
            len(sub(d, proj)) < 1e-6
        })
        .collect();

    let ring_count = if full { steps } else { steps + 1 };
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut ring_verts: Vec<Vec<usize>> = vec![vec![0; m]; ring_count];
    let mut axis_vert_idx: Vec<Option<usize>> = vec![None; m];

    for (r, ring) in ring_verts.iter_mut().enumerate() {
        let ang = sweep_rad * (r as f64 / steps as f64);
        for i in 0..m {
            if on_axis[i] {
                let idx = *axis_vert_idx[i].get_or_insert_with(|| {
                    let id = vertices.len();
                    vertices.push(prof[i]);
                    id
                });
                ring[i] = idx;
            } else {
                let id = vertices.len();
                vertices.push(rotate_around_axis(prof[i], origin, axis, ang));
                ring[i] = id;
            }
        }
    }

    let mut faces: Vec<Vec<usize>> = Vec::new();
    let seg_count = steps;
    for r in 0..seg_count {
        let r_next = (r + 1) % ring_count;
        for i in 0..m {
            let j = (i + 1) % m;
            let a0 = ring_verts[r][i];
            let b0 = ring_verts[r][j];
            let a1 = ring_verts[r_next][i];
            let b1 = ring_verts[r_next][j];
            if a0 == a1 && b0 == b1 {
                // Edge lies completely on the revolution axis — no side face.
                continue;
            } else if a0 == a1 {
                // Vertex i is on the axis -> outward triangle (a0, b0, b1)
                faces.push(vec![a0, b0, b1]);
            } else if b0 == b1 {
                // Vertex j is on the axis -> outward triangle (a0, b0, a1)
                faces.push(vec![a0, b0, a1]);
            } else {
                let quad = [a0, b0, b1, a1];
                if is_planar_quad(&vertices, quad) {
                    faces.push(quad.to_vec());
                } else {
                    faces.push(vec![a0, b0, b1]);
                    faces.push(vec![a0, b1, a1]);
                }
            }
        }
    }

    if !full {
        // Start cap (faces -sweep_tangent -> reversed ring 0)
        let start_ring: Vec<usize> = ring_verts[0].iter().copied().rev().collect();
        let start_n = scale(sweep_tangent, -1.0);
        for tri in triangulate_simple_polygon(&vertices, &start_ring, start_n) {
            faces.push(tri.to_vec());
        }
        // End cap (faces +rotated sweep_tangent -> forward last ring)
        let end_ring = ring_verts[steps].clone();
        let end_n = rotate_around_axis(sweep_tangent, [0.0; 3], axis, sweep_rad);
        for tri in triangulate_simple_polygon(&vertices, &end_ring, end_n) {
            faces.push(tri.to_vec());
        }
    }

    let mut poly = Polyhedron { vertices, faces };
    // Ensure outward orientation (positive signed volume).
    ensure_outward_orientation(&mut poly);
    validate_polyhedron(&poly)?;
    Ok(poly)
}

fn is_planar_quad(verts: &[[f64; 3]], q: [usize; 4]) -> bool {
    let n = cross(sub(verts[q[1]], verts[q[0]]), sub(verts[q[2]], verts[q[0]]));
    let nl = len(n);
    if nl < 1e-10 {
        return false;
    }
    let unit_n = scale(n, 1.0 / nl);
    dot(sub(verts[q[3]], verts[q[0]]), unit_n).abs() < 1e-7
}

fn is_convex_planar(verts: &[[f64; 3]], ring: &[usize], normal: [f64; 3]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let a = verts[ring[i]];
        let b = verts[ring[(i + 1) % n]];
        let c = verts[ring[(i + 2) % n]];
        let cr = cross(sub(b, a), sub(c, b));
        if dot(cr, normal) < -1e-9 {
            return false;
        }
    }
    true
}

fn ensure_outward_orientation(poly: &mut Polyhedron) {
    let mut vol6 = 0.0;
    for face in &poly.faces {
        if face.len() < 3 {
            continue;
        }
        let v0 = poly.vertices[face[0]];
        for i in 1..(face.len() - 1) {
            vol6 += dot(
                v0,
                cross(poly.vertices[face[i]], poly.vertices[face[i + 1]]),
            );
        }
    }
    if vol6 < 0.0 {
        for face in &mut poly.faces {
            face.reverse();
        }
    }
}

fn validate_polyhedron(poly: &Polyhedron) -> Result<(), Error> {
    if poly.vertices.len() < 4 || poly.faces.len() < 4 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a 3D solid must have at least 4 vertices and 4 faces",
        ));
    }
    let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
    for face in &poly.faces {
        if face.len() < 3 {
            return Err(Error::new(
                ErrorCode::BadValue,
                "every face of a 3D solid needs at least 3 vertices",
            ));
        }
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            if a >= poly.vertices.len() || b >= poly.vertices.len() || a == b {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "a 3D solid face references an invalid or repeated vertex",
                ));
            }
            *directed.entry((a, b)).or_insert(0) += 1;
        }
    }
    for (&(a, b), &count) in &directed {
        if count != 1 || directed.get(&(b, a)).copied().unwrap_or(0) != 1 {
            return Err(Error::new(
                ErrorCode::BadValue,
                "3D solid faces do not form a closed 2-manifold (open or non-manifold edge)",
            ));
        }
    }
    Ok(())
}

// ── 3D BSP CSG Boolean Engine (Union / Subtract / Intersect) ─────────────────

#[derive(Clone, Debug)]
struct Plane {
    normal: [f64; 3],
    w: f64,
}

impl Plane {
    fn from_points(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Self> {
        let n = cross(sub(b, a), sub(c, a));
        let l = len(n);
        if l < 1e-10 {
            return None;
        }
        let normal = scale(n, 1.0 / l);
        Some(Self {
            normal,
            w: dot(normal, a),
        })
    }

    fn flip(&mut self) {
        self.normal = scale(self.normal, -1.0);
        self.w = -self.w;
    }

    fn split_polygon(
        &self,
        poly: CsgPoly,
        coplanar_front: &mut Vec<CsgPoly>,
        coplanar_back: &mut Vec<CsgPoly>,
        front: &mut Vec<CsgPoly>,
        back: &mut Vec<CsgPoly>,
    ) {
        const COPLANAR: u8 = 0;
        const FRONT: u8 = 1;
        const BACK: u8 = 2;
        const SPANNING: u8 = 3;

        let mut poly_type = 0u8;
        let mut types = Vec::with_capacity(poly.verts.len());
        for &v in &poly.verts {
            let t = dot(self.normal, v) - self.w;
            let cls = if t < -EPS {
                BACK
            } else if t > EPS {
                FRONT
            } else {
                COPLANAR
            };
            poly_type |= cls;
            types.push(cls);
        }

        match poly_type {
            COPLANAR => {
                if dot(self.normal, poly.plane.normal) > 0.0 {
                    coplanar_front.push(poly);
                } else {
                    coplanar_back.push(poly);
                }
            }
            FRONT => front.push(poly),
            BACK => back.push(poly),
            SPANNING => {
                let mut f_verts = Vec::new();
                let mut b_verts = Vec::new();
                let n = poly.verts.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    let ti = types[i];
                    let tj = types[j];
                    let vi = poly.verts[i];
                    let vj = poly.verts[j];
                    if ti != BACK {
                        f_verts.push(vi);
                    }
                    if ti != FRONT {
                        b_verts.push(vi);
                    }
                    if (ti | tj) == SPANNING {
                        let denom = dot(self.normal, sub(vj, vi));
                        let t = if denom.abs() > 1e-12 {
                            ((self.w - dot(self.normal, vi)) / denom).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        let v = add(vi, scale(sub(vj, vi), t));
                        f_verts.push(v);
                        b_verts.push(v);
                    }
                }
                if let Some(p) = CsgPoly::from_verts_with_plane(f_verts, poly.plane.clone()) {
                    front.push(p);
                }
                if let Some(p) = CsgPoly::from_verts_with_plane(b_verts, poly.plane) {
                    back.push(p);
                }
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug)]
struct CsgPoly {
    verts: Vec<[f64; 3]>,
    plane: Plane,
}

impl CsgPoly {
    fn from_verts_with_plane(verts: Vec<[f64; 3]>, plane: Plane) -> Option<Self> {
        let mut clean: Vec<[f64; 3]> = Vec::with_capacity(verts.len());
        for v in verts {
            if clean.last().is_none_or(|&prev| len(sub(v, prev)) > 1e-7) {
                clean.push(v);
            }
        }
        if clean.len() >= 2 && len(sub(*clean.last().unwrap(), clean[0])) <= 1e-7 {
            clean.pop();
        }
        if clean.len() < 3 {
            return None;
        }
        // Check non-zero area.
        let mut area = 0.0;
        let v0 = clean[0];
        for i in 1..(clean.len() - 1) {
            area += 0.5 * len(cross(sub(clean[i], v0), sub(clean[i + 1], v0)));
        }
        (area > 1e-10).then_some(Self {
            verts: clean,
            plane,
        })
    }

    fn flip(&mut self) {
        self.verts.reverse();
        self.plane.flip();
    }
}

struct BspNode {
    plane: Option<Plane>,
    front: Option<Box<BspNode>>,
    back: Option<Box<BspNode>>,
    polygons: Vec<CsgPoly>,
}

impl BspNode {
    fn new(polygons: Vec<CsgPoly>) -> Self {
        let mut node = Self {
            plane: None,
            front: None,
            back: None,
            polygons: Vec::new(),
        };
        if !polygons.is_empty() {
            node.build(polygons);
        }
        node
    }

    fn invert(&mut self) {
        for p in &mut self.polygons {
            p.flip();
        }
        if let Some(plane) = &mut self.plane {
            plane.flip();
        }
        if let Some(front) = &mut self.front {
            front.invert();
        }
        if let Some(back) = &mut self.back {
            back.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polygons: Vec<CsgPoly>) -> Vec<CsgPoly> {
        let Some(plane) = &self.plane else {
            return polygons;
        };
        let mut front = Vec::new();
        let mut back = Vec::new();
        let mut co_front = Vec::new();
        let mut co_back = Vec::new();
        for p in polygons {
            plane.split_polygon(p, &mut co_front, &mut co_back, &mut front, &mut back);
        }
        front.extend(co_front);
        back.extend(co_back);
        if let Some(f) = &self.front {
            front = f.clip_polygons(front);
        }
        if let Some(b) = &self.back {
            back = b.clip_polygons(back);
        } else {
            back.clear();
        }
        front.extend(back);
        front
    }

    fn clip_to(&mut self, bsp: &BspNode) {
        self.polygons = bsp.clip_polygons(std::mem::take(&mut self.polygons));
        if let Some(front) = &mut self.front {
            front.clip_to(bsp);
        }
        if let Some(back) = &mut self.back {
            back.clip_to(bsp);
        }
    }

    fn all_polygons(&self) -> Vec<CsgPoly> {
        let mut list = self.polygons.clone();
        if let Some(front) = &self.front {
            list.extend(front.all_polygons());
        }
        if let Some(back) = &self.back {
            list.extend(back.all_polygons());
        }
        list
    }

    fn build(&mut self, polygons: Vec<CsgPoly>) {
        if polygons.is_empty() {
            return;
        }
        if self.plane.is_none() {
            self.plane = Some(polygons[0].plane.clone());
        }
        let plane = self.plane.clone().unwrap();
        let mut front = Vec::new();
        let mut back = Vec::new();
        let mut co_front = Vec::new();
        let mut co_back = Vec::new();
        for p in polygons {
            plane.split_polygon(p, &mut co_front, &mut co_back, &mut front, &mut back);
        }
        self.polygons.extend(co_front);
        self.polygons.extend(co_back);
        if !front.is_empty() {
            self.front
                .get_or_insert_with(|| Box::new(BspNode::new(Vec::new())))
                .build(front);
        }
        if !back.is_empty() {
            self.back
                .get_or_insert_with(|| Box::new(BspNode::new(Vec::new())))
                .build(back);
        }
    }
}

fn polyhedron_to_csg_polys(poly: &Polyhedron) -> Vec<CsgPoly> {
    let mut out = Vec::new();
    for face in &poly.faces {
        if face.len() < 3 {
            continue;
        }
        let n = face_normal(&poly.vertices, face);
        if len(n) < 1e-9 {
            continue;
        }
        if is_convex_planar(&poly.vertices, face, n) {
            if let Some(plane) = Plane::from_points(
                poly.vertices[face[0]],
                poly.vertices[face[1]],
                poly.vertices[face[2]],
            ) {
                let verts: Vec<[f64; 3]> = face.iter().map(|&i| poly.vertices[i]).collect();
                if let Some(p) = CsgPoly::from_verts_with_plane(verts, plane) {
                    out.push(p);
                }
            }
        } else {
            for [i0, i1, i2] in triangulate_simple_polygon(&poly.vertices, face, n) {
                if let Some(plane) =
                    Plane::from_points(poly.vertices[i0], poly.vertices[i1], poly.vertices[i2])
                {
                    let verts = vec![poly.vertices[i0], poly.vertices[i1], poly.vertices[i2]];
                    if let Some(p) = CsgPoly::from_verts_with_plane(verts, plane) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

/// Performs a 3D CSG boolean (`Union`, `Subtract`, `Intersect`) on two closed
/// 2-manifold polyhedra, repairing T-junctions so the result is a strict closed
/// 2-manifold ready for ACIS `build_planar_body`.
pub fn csg_boolean(op: BooleanOp, lhs: &Polyhedron, rhs: &Polyhedron) -> Result<Polyhedron, Error> {
    let mut a = BspNode::new(polyhedron_to_csg_polys(lhs));
    let mut b = BspNode::new(polyhedron_to_csg_polys(rhs));

    match op {
        BooleanOp::Union => {
            a.clip_to(&b);
            b.clip_to(&a);
            b.invert();
            b.clip_to(&a);
            b.invert();
            a.build(b.all_polygons());
        }
        BooleanOp::Subtract => {
            a.invert();
            a.clip_to(&b);
            b.clip_to(&a);
            b.invert();
            b.clip_to(&a);
            b.invert();
            a.build(b.all_polygons());
            a.invert();
        }
        BooleanOp::Intersect => {
            a.invert();
            b.clip_to(&a);
            b.invert();
            a.clip_to(&b);
            b.clip_to(&a);
            a.build(b.all_polygons());
            a.invert();
        }
    }

    let polys = a.all_polygons();
    if polys.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("boolean `{}` produced an empty solid", op.name()),
        ));
    }
    csg_polys_to_manifold_polyhedron(polys, op)
}

fn csg_polys_to_manifold_polyhedron(
    polys: Vec<CsgPoly>,
    op: BooleanOp,
) -> Result<Polyhedron, Error> {
    // 1. Weld coincident vertices within 1e-6 tolerance.
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let weld = |p: [f64; 3],
                vertices: &mut Vec<[f64; 3]>,
                grid: &mut HashMap<(i64, i64, i64), Vec<usize>>|
     -> usize {
        let bx = (p[0] * WELD_QUANT).round() as i64;
        let by = (p[1] * WELD_QUANT).round() as i64;
        let bz = (p[2] * WELD_QUANT).round() as i64;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(bucket) = grid.get(&(bx + dx, by + dy, bz + dz)) {
                        for &idx in bucket {
                            if len(sub(vertices[idx], p)) < 1e-5 {
                                return idx;
                            }
                        }
                    }
                }
            }
        }
        let idx = vertices.len();
        vertices.push(p);
        grid.entry((bx, by, bz)).or_default().push(idx);
        idx
    };

    let mut faces: Vec<Vec<usize>> = Vec::with_capacity(polys.len());
    for p in polys {
        let mut ring = Vec::with_capacity(p.verts.len());
        for v in p.verts {
            let idx = weld(v, &mut vertices, &mut grid);
            if ring.last().copied() != Some(idx) {
                ring.push(idx);
            }
        }
        if ring.len() >= 2 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            // Check non-zero area after welding.
            let n = face_normal(&vertices, &ring);
            if len(n) > 1e-9 {
                faces.push(ring);
            }
        }
    }

    // 2. Eliminate T-junctions: split any edge (u, v) that passes through another
    //    vertex w strictly between u and v.
    let mut resolved_faces: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    for face in faces {
        let mut new_ring: Vec<usize> = Vec::with_capacity(face.len());
        let n = face.len();
        for i in 0..n {
            let u = face[i];
            let v = face[(i + 1) % n];
            new_ring.push(u);
            let pu = vertices[u];
            let pv = vertices[v];
            let dir = sub(pv, pu);
            let seg_len = len(dir);
            if seg_len < 1e-9 {
                continue;
            }
            let unit = scale(dir, 1.0 / seg_len);
            let mut mid_verts: Vec<(f64, usize)> = Vec::new();
            for (w, &pw) in vertices.iter().enumerate() {
                if w == u || w == v {
                    continue;
                }
                let to_w = sub(pw, pu);
                let t = dot(to_w, unit);
                if t > 1e-5 && t < seg_len - 1e-5 {
                    let proj = add(pu, scale(unit, t));
                    if len(sub(pw, proj)) < 1e-5 {
                        mid_verts.push((t, w));
                    }
                }
            }
            if !mid_verts.is_empty() {
                mid_verts.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, w) in mid_verts {
                    if new_ring.last().copied() != Some(w) && w != v {
                        new_ring.push(w);
                    }
                }
            }
        }
        if new_ring.len() >= 3 {
            resolved_faces.push(new_ring);
        }
    }

    // 3. Remove any exact opposing duplicate faces (can appear on tangent coplanar contacts).
    let mut poly = Polyhedron {
        vertices,
        faces: resolved_faces,
    };
    ensure_outward_orientation(&mut poly);
    if poly.faces.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("boolean `{}` produced an empty solid", op.name()),
        ));
    }
    validate_polyhedron(&poly).map_err(|_| {
        Error::new(
            ErrorCode::BadValue,
            format!(
                "boolean `{}` did not yield a closed 2-manifold solid (check for tangent/degenerate overlap)",
                op.name()
            ),
        )
    })?;
    Ok(poly)
}
