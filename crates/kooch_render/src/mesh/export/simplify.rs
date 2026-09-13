//! Mesh decimation, for turning a visual mesh into a collision mesh.

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

/// Decimates `mesh` towards `target`, preserving its silhouette as far as the collapse allows.
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

/// Rebuilds a mesh from a surviving index list, dropping orphaned vertices and renumbering.
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
