//! Scene-graph walk: composes per-node transforms top-down and bakes
//! every reached primitive's geometry into the cumulative output pool.

use glam::{Mat3, Mat4, Vec3};

use super::GltfMeshError;
use crate::mesh::vertex::{Aabb, MeshVertex};

/// Composes `parent_xform` with the node's local transform and
/// recurses. Every primitive reached is ingested at the cumulative
/// world transform.
pub(super) fn walk_node(
    node: &gltf::Node<'_>,
    parent_xform: Mat4,
    buffers: &[Vec<u8>],
    out_vertices: &mut Vec<MeshVertex>,
    out_indices: &mut Vec<u32>,
    aabb: &mut Aabb,
) -> Result<(), GltfMeshError> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world_xform = parent_xform * local;

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            ingest_primitive(
                &primitive,
                world_xform,
                buffers,
                out_vertices,
                out_indices,
                aabb,
            )?;
        }
    }

    for child in node.children() {
        walk_node(
            &child,
            world_xform,
            buffers,
            out_vertices,
            out_indices,
            aabb,
        )?;
    }
    Ok(())
}

/// Reads a single primitive, applies `world_xform` to positions
/// (and normal-correct transform to normals), appends the result to
/// the output buffers with indices rebased into the cumulative pool.
pub(super) fn ingest_primitive(
    primitive: &gltf::Primitive<'_>,
    world_xform: Mat4,
    buffers: &[Vec<u8>],
    out_vertices: &mut Vec<MeshVertex>,
    out_indices: &mut Vec<u32>,
    aabb: &mut Aabb,
) -> Result<(), GltfMeshError> {
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(GltfMeshError::MissingAttribute("POSITION"))?
        .collect();
    let vertex_count = positions.len();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|iter| iter.collect())
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; vertex_count]);

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; vertex_count]);

    // Normals transform with the inverse-transpose of the upper 3×3
    // (handles non-uniform scale correctly). Falls back to the
    // identity slice if the matrix is singular — signals a degenerate
    // node we still want to ingest rather than abort the whole load.
    let normal_xform = Mat3::from_mat4(world_xform).inverse().transpose();

    let vertex_offset = out_vertices.len() as u32;

    for i in 0..vertex_count {
        let p_local = Vec3::from_array(positions[i]);
        let p_world = world_xform.transform_point3(p_local);
        aabb.expand(p_world);

        let n_local = Vec3::from_array(normals[i]);
        let n_world = (normal_xform * n_local).normalize_or_zero();
        let n_out = if n_world == Vec3::ZERO {
            [0.0, 1.0, 0.0]
        } else {
            n_world.to_array()
        };

        out_vertices.push(MeshVertex {
            position: p_world.to_array(),
            normal: n_out,
            uv: *uvs.get(i).unwrap_or(&[0.0, 0.0]),
        });
    }

    let primitive_indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..vertex_count as u32).collect(),
    };
    out_indices.extend(primitive_indices.into_iter().map(|i| i + vertex_offset));
    Ok(())
}
