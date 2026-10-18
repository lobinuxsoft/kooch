//! Bending a mesh along a [`Spline`](kooch_ecs::spline::Spline) — Blender's Curve modifier (#1446).
//!
//! 🔴 **The source is an asset and the result is a generated mesh.** Bending in place would destroy
//! what the next edit has to bend, so the author's asset is read and never written; the bent copy
//! goes out under a GUID of this entity's own, the way a block's geometry does.
//!
//! 🔴 **The meshlet vertices are bent directly.** Only positions and normals move — the topology,
//! the meshlet partition and the triangle bytes are untouched — which makes the deform exact and
//! cheap. Routing through `BlockMesh` would lose fidelity for nothing.
//!
//! Baked here, at authoring time. A mesh that bends *while the game runs* is a vertex-offset
//! shader, which is #1180.

use glam::Vec3;

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::FieldChoice;
use kooch_ecs::spline::{DEFAULT_RADIUS, Knot, arc, eval};
use kooch_render::mesh::Aabb;
use kooch_render::meshlet::MeshletMesh;

/// Which local axis of the mesh runs along the curve.
pub const AXIS_X: u32 = 0;
/// The mesh's local Y runs along the curve.
pub const AXIS_Y: u32 = 1;
/// The mesh's local Z runs along the curve.
pub const AXIS_Z: u32 = 2;

/// What the Inspector offers for `axis`.
pub static AXIS_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "X",
        value: AXIS_X as i64,
    },
    FieldChoice {
        label: "Y",
        value: AXIS_Y as i64,
    },
    FieldChoice {
        label: "Z",
        value: AXIS_Z as i64,
    },
];

/// Bends a mesh asset along the `Spline` on the same entity.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Spline")]
pub struct SplineDeform {
    /// The mesh to bend. Read, never written — it is what every re-bend starts from.
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub source: Option<Guid>,
    /// Which of the mesh's own axes is stretched along the curve. Its extent on that axis maps to
    /// the curve's whole length.
    #[reflect(choices = AXIS_CHOICES)]
    pub axis: u32,
    /// Whether each knot's `radius` scales the cross-section there, which is where a taper comes
    /// from. Off leaves the mesh its authored thickness.
    pub taper: bool,
}

impl Default for SplineDeform {
    fn default() -> Self {
        Self {
            source: None,
            axis: AXIS_Z,
            taper: false,
        }
    }
}

impl Component for SplineDeform {}

/// The mesh bent along the curve, or `None` when there is nothing to bend.
///
/// `None` rather than an empty mesh: a curve of one knot and a mesh with no extent along its
/// chosen axis are both states an author passes through while building, and replacing their
/// geometry with nothing would make the entity blink out mid-edit.
pub fn deform(
    mesh: &MeshletMesh,
    points: &[Knot],
    closed: bool,
    of: &SplineDeform,
) -> Option<MeshletMesh> {
    if eval::segments(points, closed) == 0 || mesh.vertices.is_empty() {
        return None;
    }
    let length = arc::length(points, closed);
    // 🔴 `!(x > 0.0)`, never `x <= 0.0`: a NaN here would pass the `<=` and divide every vertex
    // into NaN, which is a mesh the GPU draws as nothing with no error anywhere.
    if !(length > 0.0) {
        return None;
    }
    let (first, last) = span(mesh, of.axis);
    let extent = last - first;
    if !(extent > 0.0) {
        return None;
    }

    // 🔴 Built once. `arc::parameter_at` rebuilds this table on every call, and a call per vertex
    // turns a 10k mesh into ten thousand walks of the curve.
    let table = arc::walked(points, closed);
    let mut bent = mesh.clone();
    let mut bounds = Aabb::empty();
    for vertex in &mut bent.vertices {
        let (along, right, up) = split(Vec3::from(vertex.position), of.axis);
        let reached = ((along - first) / extent).clamp(0.0, 1.0);
        let frame = eval::frame(points, closed, arc::parameter_in(&table, reached * length));
        // Against the default rather than against the radius authored at the first knot: a curve
        // nobody tapered then carries the mesh at the size it was modelled.
        let asked = match of.taper {
            true => eval::radius_at(points, closed, reached) / DEFAULT_RADIUS,
            false => 1.0,
        };
        // 🔴 `is_finite`, because a radius left at NaN by a bad import would scale every vertex
        // into NaN — and a NaN mesh draws as nothing, with no error anywhere to say why.
        let scale = match asked.is_finite() {
            true => asked,
            false => 1.0,
        };
        // 🔴 `up × tangent`, not `Frame::side`. `side` is `tangent × up`, which makes
        // (side, up, tangent) LEFT-handed — fine for a sweep, whose rings are symmetric, and a
        // mirrored mesh for anything that is not. This way a straight curve along the chosen
        // axis leaves the mesh exactly as it was modelled.
        let sideways = frame.up.cross(frame.tangent);
        let placed = frame.at + sideways * right * scale + frame.up * up * scale;
        bounds.expand(placed);
        vertex.position = placed.into();

        // ⚠️ The normal turns with the frame too. Leaving it alone bends the silhouette correctly
        // and lights the mesh as if it were still straight — which reads as a material fault, not
        // a deform one, and is why this has a test of its own.
        let (n_along, n_right, n_up) = split(Vec3::from(vertex.normal), of.axis);
        vertex.normal = (frame.tangent * n_along + sideways * n_right + frame.up * n_up)
            .normalize_or(Vec3::Y)
            .into();
    }

    bent.aabb = bounds;
    rebound(&mut bent);
    Some(bent)
}

/// A vertex split into how far along it sits, and how far across: `(along, right, up)`.
///
/// 🔴 Cyclic — `(x,y,z)`, `(y,z,x)`, `(z,x,y)` — so the triple stays right-handed whichever axis
/// the author picks. Swapping two of them instead would mirror the mesh on two axes out of three,
/// which reads as a modelling mistake rather than as a deform one.
fn split(at: Vec3, axis: u32) -> (f32, f32, f32) {
    match axis {
        AXIS_X => (at.x, at.y, at.z),
        AXIS_Y => (at.y, at.z, at.x),
        _ => (at.z, at.x, at.y),
    }
}

/// How far the mesh reaches along `axis`, from its own bounds.
fn span(mesh: &MeshletMesh, axis: u32) -> (f32, f32) {
    let (low, _, _) = split(mesh.aabb.min, axis);
    let (high, _, _) = split(mesh.aabb.max, axis);
    (low, high)
}

/// Recomputes every meshlet's bounds from the vertices it now has.
///
/// 🔴 The backface cone is **switched off**, not recomputed. A bent cluster's normals no longer
/// share one cone, and a cone that is wrong by any amount culls geometry the camera can see —
/// holes in the mesh, with nothing reporting a fault. `cone_cutoff >= 1.0` is what the cull shader
/// and `cull.rs` both read as "do not try", so the cost is some backface culling and never
/// correctness.
fn rebound(mesh: &mut MeshletMesh) {
    for index in 0..mesh.meshlets.len() {
        let meshlet = mesh.meshlets[index];
        let first = meshlet.vertex_offset as usize;
        let count = meshlet.vertex_count as usize;
        let mut bounds = Aabb::empty();
        for slot in first..first + count {
            let Some(vertex) = mesh
                .meshlet_vertices
                .get(slot)
                .and_then(|at| mesh.vertices.get(*at as usize))
            else {
                continue;
            };
            bounds.expand(Vec3::from(vertex.position));
        }
        if bounds.is_empty() {
            continue;
        }
        let centre = (bounds.min + bounds.max) / 2.0;
        let descriptor = &mut mesh.meshlets[index];
        descriptor.aabb_min = bounds.min.into();
        descriptor.aabb_max = bounds.max.into();
        descriptor.bounds_center = centre.into();
        descriptor.bounding_radius = (bounds.max - centre).length();
        descriptor.cone_cutoff = 1.0;
    }
}

/// A number that changes whenever the bend would come out different.
pub fn revision(of: &SplineDeform) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // 🔴 Field by field, so one added here and not listed edits nothing the author can see: the
    // control moves and the mesh never re-bends. The same trap `SplineSweep` fell into with `wall`.
    of.source.hash(&mut hasher);
    of.axis.hash(&mut hasher);
    of.taper.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;

mod sync;

pub use sync::{BentMeshes, sync_deforms};
