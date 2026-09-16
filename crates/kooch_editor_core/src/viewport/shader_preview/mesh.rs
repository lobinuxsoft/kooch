//! The primitive the preview draws: the engine's shape, uploaded with the tangents its meshes lack.

use glam::Vec3;
use kooch_render::mesh::{Mesh, Primitive};
use wgpu::util::DeviceExt;

/// A vertex of the preview mesh. The engine's `MeshVertex` has no tangent, and a normal map cannot
/// be previewed without one, so it is computed here and carried alongside.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct PreviewVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent: [f32; 4],
}

/// The primitive currently uploaded.
pub(super) struct PreviewMesh {
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    pub(super) count: u32,
}

/// Which primitive a preview opens on: the sphere, which is what a material is judged on.
pub(super) fn default_primitive() -> usize {
    Primitive::CANONICAL
        .iter()
        .position(|(name, _)| *name == "sphere")
        .unwrap_or(0)
}

/// Uploads a primitive, tangents and all.
pub(super) fn upload(device: &wgpu::Device, mesh: &Mesh) -> PreviewMesh {
    let vertices = with_tangents(mesh);
    PreviewMesh {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_preview_vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shader_preview_indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        count: mesh.indices.len() as u32,
    }
}

/// The tangent frame the engine's meshes do not carry, accumulated per triangle from the uv
/// derivatives — the standard construction, and what **Unpack Normal** needs to mean anything here.
fn with_tangents(mesh: &Mesh) -> Vec<PreviewVertex> {
    let mut accumulated = vec![Vec3::ZERO; mesh.vertices.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| i as usize);
        let (va, vb, vc) = (&mesh.vertices[a], &mesh.vertices[b], &mesh.vertices[c]);
        let edge1 = Vec3::from(vb.position) - Vec3::from(va.position);
        let edge2 = Vec3::from(vc.position) - Vec3::from(va.position);
        let duv1 = [vb.uv[0] - va.uv[0], vb.uv[1] - va.uv[1]];
        let duv2 = [vc.uv[0] - va.uv[0], vc.uv[1] - va.uv[1]];
        let determinant = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        // A degenerate uv triangle says nothing about which way the texture runs.
        if determinant.abs() < 1e-12 {
            continue;
        }
        let tangent = (edge1 * duv2[1] - edge2 * duv1[1]) / determinant;
        for index in [a, b, c] {
            accumulated[index] += tangent;
        }
    }

    mesh.vertices
        .iter()
        .zip(accumulated)
        .map(|(vertex, tangent)| {
            let normal = Vec3::from(vertex.normal).normalize_or(Vec3::Y);
            // Gram-Schmidt, so the tangent is square to the normal the shader will use.
            let tangent = (tangent - normal * normal.dot(tangent)).normalize_or(any_square(normal));
            PreviewVertex {
                position: vertex.position,
                normal: normal.to_array(),
                uv: vertex.uv,
                tangent: [tangent.x, tangent.y, tangent.z, 1.0],
            }
        })
        .collect()
}

/// Any direction square to `normal`, for a vertex no triangle gave a tangent.
fn any_square(normal: Vec3) -> Vec3 {
    let axis = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    normal.cross(axis).normalize_or(Vec3::X)
}

#[cfg(test)]
mod tests;
