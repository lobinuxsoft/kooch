//! Mesh decimation, for turning a visual mesh into a collision mesh.
//!
//! A 50k-triangle prop is the wrong thing to collide against: rapier will
//! do it, and it will cost more per contact than the rest of the frame.
//! The usual answer is a hand-made low-poly proxy. This is the automatic
//! first draft of that proxy — export it (see [`to_glb`]), look at it,
//! fix it if it is wrong, and hand it back as the collider source.
//!
//! Built on `meshopt`, already a workspace dependency driving the meshlet
//! LOD chain. Nothing hand-rolled: the LOD chain uses
//! `simplify_with_locks` because it must preserve cell boundaries across
//! independently-simplified groups, and a whole-mesh decimation has no
//! such constraint, so plain `simplify` is the right call here.
//!
//! [`to_glb`]: super::to_glb

use crate::mesh::{Mesh, MeshVertex};

/// How aggressively to decimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimplifyTarget {
    /// Keep this fraction of the triangles, in `(0, 1]`.
    Ratio(f32),
    /// Aim for this many triangles.
    Triangles(u32),
}

impl SimplifyTarget {
    /// Resolves to a target index count for a mesh of `triangles`.
    fn index_count(&self, triangles: usize) -> usize {
        let target = match *self {
            // Clamped, not rejected: a ratio above 1 asks for more
            // triangles than exist, which `meshopt` answers by returning
            // the input — a confusing no-op rather than an error.
            SimplifyTarget::Ratio(r) => (triangles as f32 * r.clamp(0.0, 1.0)).round() as usize,
            SimplifyTarget::Triangles(t) => (t as usize).min(triangles),
        };
        // A collider needs a closed surface; one triangle is the floor
        // below which there is nothing left to collide with.
        target.max(1) * 3
    }
}

/// Decimates `mesh` towards `target`, preserving its silhouette as far as
/// the collapse allows.
///
/// Returns the mesh unchanged when it is already at or below the target,
/// or when `meshopt` cannot reduce it further — a mesh of disconnected
/// triangles has no edges to collapse, and reporting that as an error
/// would make the caller handle a case where "unchanged" is the answer.
///
/// The error `meshopt` reports for the collapse is returned alongside, in
/// mesh units, so a caller can refuse a proxy that drifted too far from
/// the original.
pub fn simplify(mesh: &Mesh, target: SimplifyTarget) -> (Mesh, f32) {
    let triangles = mesh.indices.len() / 3;
    if triangles <= 1 || mesh.vertices.is_empty() {
        return (mesh.clone(), 0.0);
    }

    let target_indices = target.index_count(triangles);
    if target_indices >= mesh.indices.len() {
        return (mesh.clone(), 0.0);
    }

    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let stride = std::mem::size_of::<MeshVertex>();
    // Offset 0: position is the first field of `MeshVertex`, and position
    // is what the collapse metric is defined over.
    let Ok(adapter) = meshopt::VertexDataAdapter::new(vertex_bytes, stride, 0) else {
        return (mesh.clone(), 0.0);
    };

    let mut error = 0.0f32;
    let indices = meshopt::simplify(
        &mesh.indices,
        &adapter,
        target_indices,
        // No error ceiling: the caller asked for a triangle budget, so
        // hitting the budget is the goal and the resulting error is
        // reported back rather than used to stop early.
        f32::MAX,
        meshopt::SimplifyOptions::None,
        Some(&mut error),
    );

    if indices.is_empty() || indices.len() >= mesh.indices.len() {
        return (mesh.clone(), 0.0);
    }

    (compact(mesh, &indices), error)
}

/// Rebuilds a mesh from a surviving index list, dropping orphaned
/// vertices and renumbering.
///
/// `meshopt::simplify` returns indices into the *original* vertex array,
/// so the collapsed vertices are still in the buffer, unreferenced.
/// Exporting that writes a file whose vertex count says nothing about its
/// complexity, and whose AABB still covers geometry that no longer exists.
fn compact(mesh: &Mesh, indices: &[u32]) -> Mesh {
    let mut remap = vec![u32::MAX; mesh.vertices.len()];
    let mut vertices = Vec::new();
    let mut out = Vec::with_capacity(indices.len());

    for &old in indices {
        let slot = &mut remap[old as usize];
        if *slot == u32::MAX {
            *slot = vertices.len() as u32;
            vertices.push(mesh.vertices[old as usize]);
        }
        out.push(*slot);
    }

    Mesh::from_arrays(vertices, out)
}
