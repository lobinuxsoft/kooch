//! Mesh batch types — what the editor pushes per frame, what the
//! renderer consumes.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3, Vec4};

/// Single mesh vertex — position + RGBA color + edge UV.
///
/// `edge_uv` is a per-vertex 2D coordinate the fragment shader uses to
/// detect proximity to a face edge (u or v near 0 or 1). When near an
/// edge the fragment overrides alpha to 1.0 so the geometry shows a
/// crisp outline integrated with the fill — no separate line pass.
///
/// Geometry that doesn't want edge highlighting passes `edge_uv =
/// (0.5, 0.5)` so the fragment is always interior.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub edge_uv: [f32; 2],
}

impl MeshVertex {
    pub fn new(position: Vec3, color: Vec4) -> Self {
        Self {
            position: position.to_array(),
            color: color.to_array(),
            edge_uv: [0.5, 0.5],
        }
    }

    pub fn with_edge_uv(position: Vec3, color: Vec4, edge_uv: Vec2) -> Self {
        Self {
            position: position.to_array(),
            color: color.to_array(),
            edge_uv: edge_uv.to_array(),
        }
    }
}

/// One mesh draw queued for the gizmo mesh pass.
///
/// Vertices are world-space (no model matrix). Triangles are wound
/// CCW (no culling either way — the pipeline disables culling).
#[derive(Debug, Clone, Default)]
pub struct MeshDraw {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

/// Per-frame collection of mesh draws.
///
/// Stored as a `Resources` entry next to [`crate::GizmoBatch`].
/// Editor populates it through [`crate::Gizmos`] each frame; renderer
/// drains it during the mesh pass.
#[derive(Debug, Default)]
pub struct MeshBatch {
    pub draws: Vec<MeshDraw>,
}

impl MeshBatch {
    pub fn clear(&mut self) {
        self.draws.clear();
    }

    /// Pushes a filled quad from four corners (CCW order). Color is
    /// RGBA — alpha controls transparency. Per-corner `edge_uv` is set
    /// so the shader renders a crisp 1-alpha outline at the perimeter
    /// of the quad without a separate line pass.
    pub fn filled_quad(&mut self, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, color: Vec4) {
        self.draws.push(MeshDraw {
            vertices: vec![
                MeshVertex::with_edge_uv(p0, color, Vec2::new(0.0, 0.0)),
                MeshVertex::with_edge_uv(p1, color, Vec2::new(1.0, 0.0)),
                MeshVertex::with_edge_uv(p2, color, Vec2::new(1.0, 1.0)),
                MeshVertex::with_edge_uv(p3, color, Vec2::new(0.0, 1.0)),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        });
    }

    /// Pushes a filled axis-aligned box centered at `center` with the
    /// given half-extents. Each of the 6 faces has its own 4 vertices
    /// carrying corner-aligned edge UVs (0,0)..(1,1) so the shader
    /// renders each face's perimeter with alpha 1.0 — the cube reads
    /// as a wireframe with translucent fill, no separate line pass.
    pub fn filled_aabb(&mut self, center: Vec3, half_extents: Vec3, color: Vec4) {
        let h = half_extents;
        let c = center;
        // 6 faces, each as 4 corners in CCW order viewed from outside.
        let faces: [[Vec3; 4]; 6] = [
            // -X (left): outward normal -X
            [
                c + Vec3::new(-h.x, -h.y, h.z),
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, h.y, -h.z),
                c + Vec3::new(-h.x, h.y, h.z),
            ],
            // +X (right)
            [
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(h.x, -h.y, h.z),
                c + Vec3::new(h.x, h.y, h.z),
                c + Vec3::new(h.x, h.y, -h.z),
            ],
            // -Y (bottom)
            [
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(h.x, -h.y, h.z),
                c + Vec3::new(-h.x, -h.y, h.z),
            ],
            // +Y (top)
            [
                c + Vec3::new(-h.x, h.y, h.z),
                c + Vec3::new(h.x, h.y, h.z),
                c + Vec3::new(h.x, h.y, -h.z),
                c + Vec3::new(-h.x, h.y, -h.z),
            ],
            // -Z (back)
            [
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, h.y, -h.z),
                c + Vec3::new(h.x, h.y, -h.z),
            ],
            // +Z (front)
            [
                c + Vec3::new(-h.x, -h.y, h.z),
                c + Vec3::new(h.x, -h.y, h.z),
                c + Vec3::new(h.x, h.y, h.z),
                c + Vec3::new(-h.x, h.y, h.z),
            ],
        ];

        let uvs: [Vec2; 4] = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];

        let mut vertices: Vec<MeshVertex> = Vec::with_capacity(24);
        let mut indices: Vec<u32> = Vec::with_capacity(36);
        for face in &faces {
            let base = vertices.len() as u32;
            for i in 0..4 {
                vertices.push(MeshVertex::with_edge_uv(face[i], color, uvs[i]));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        self.draws.push(MeshDraw { vertices, indices });
    }

    /// Pushes a filled torus around `axis` (unit vector through
    /// `center`) with the given major and minor radii. 32 segments
    /// around the major direction × 8 around the minor. Solid fill
    /// (no shader edges) so it reads as a smooth ring.
    pub fn filled_torus(
        &mut self,
        center: Vec3,
        axis: Vec3,
        major_radius: f32,
        minor_radius: f32,
        color: Vec4,
    ) {
        let axis = axis.normalize_or_zero();
        if axis == Vec3::ZERO {
            return;
        }
        let (perp_a, perp_b) = perpendiculars(axis);

        const MAJOR: u32 = 32;
        const MINOR: u32 = 8;
        let two_pi = std::f32::consts::TAU;

        let mut vertices: Vec<MeshVertex> = Vec::with_capacity((MAJOR * MINOR) as usize);
        let mut indices: Vec<u32> = Vec::with_capacity((MAJOR * MINOR * 6) as usize);

        for i in 0..MAJOR {
            let theta = (i as f32) / (MAJOR as f32) * two_pi;
            let cos_t = theta.cos();
            let sin_t = theta.sin();
            let ring_dir = perp_a * cos_t + perp_b * sin_t;
            let ring_center = center + ring_dir * major_radius;
            for j in 0..MINOR {
                let phi = (j as f32) / (MINOR as f32) * two_pi;
                let cos_p = phi.cos();
                let sin_p = phi.sin();
                let offset = ring_dir * cos_p * minor_radius + axis * sin_p * minor_radius;
                vertices.push(MeshVertex::new(ring_center + offset, color));
            }
        }

        for i in 0..MAJOR {
            let i_next = (i + 1) % MAJOR;
            for j in 0..MINOR {
                let j_next = (j + 1) % MINOR;
                let a = i * MINOR + j;
                let b = i_next * MINOR + j;
                let c = i_next * MINOR + j_next;
                let d = i * MINOR + j_next;
                indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }

        self.draws.push(MeshDraw { vertices, indices });
    }

    /// Pushes a filled 3D arrow from `base` to `tip`: an octagonal
    /// cylinder shaft + an octagonal cone head. Color is RGBA. No
    /// edge UVs (vertices use the neutral 0.5,0.5) so the shader
    /// renders the geometry as a solid fill — arrows want solid look,
    /// not wireframe.
    pub fn filled_arrow(&mut self, base: Vec3, tip: Vec3, color: Vec4) {
        let length_vec = tip - base;
        let length = length_vec.length();
        if length < 1e-4 {
            return;
        }
        let dir = length_vec / length;
        let head_len = (length * 0.25).clamp(0.05, length * 0.4);
        let head_radius = head_len * 0.4;
        let shaft_radius = head_radius * 0.3;
        let shaft_end = tip - dir * head_len;
        let segments: u32 = 8;

        let (perp_a, perp_b) = perpendiculars(dir);

        let mut vertices: Vec<MeshVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        push_cylinder_side(
            base,
            shaft_end,
            perp_a,
            perp_b,
            shaft_radius,
            segments,
            color,
            &mut vertices,
            &mut indices,
        );
        push_cone(
            shaft_end,
            tip,
            perp_a,
            perp_b,
            head_radius,
            segments,
            color,
            &mut vertices,
            &mut indices,
        );

        self.draws.push(MeshDraw { vertices, indices });
    }
}

// ---------------------------------------------------------------------------
// Procedural primitive helpers (private)
// ---------------------------------------------------------------------------

/// Two unit vectors perpendicular to `dir`. Picks a stable up reference
/// avoiding the gimbal case when `dir` is near vertical.
fn perpendiculars(dir: Vec3) -> (Vec3, Vec3) {
    let up = if dir.y.abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let perp_a = dir.cross(up).normalize_or(Vec3::X);
    let perp_b = dir.cross(perp_a).normalize_or(Vec3::Y);
    (perp_a, perp_b)
}

/// Generates the lateral surface of a cylinder (no caps) from
/// `bottom` to `top` with the given radius and segment count.
fn push_cylinder_side(
    bottom: Vec3,
    top: Vec3,
    perp_a: Vec3,
    perp_b: Vec3,
    radius: f32,
    segments: u32,
    color: Vec4,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let base_idx = vertices.len() as u32;
    let two_pi = std::f32::consts::TAU;
    for i in 0..segments {
        let theta = (i as f32) / (segments as f32) * two_pi;
        let offset = perp_a * theta.cos() * radius + perp_b * theta.sin() * radius;
        vertices.push(MeshVertex::new(bottom + offset, color));
        vertices.push(MeshVertex::new(top + offset, color));
    }
    for i in 0..segments {
        let i0 = base_idx + i * 2;
        let i1 = base_idx + i * 2 + 1;
        let next = (i + 1) % segments;
        let i2 = base_idx + next * 2 + 1;
        let i3 = base_idx + next * 2;
        indices.extend_from_slice(&[i0, i1, i2, i0, i2, i3]);
    }
}

/// Generates a closed cone: base disc + lateral surface from
/// `base_center` to the apex `tip`.
fn push_cone(
    base_center: Vec3,
    tip: Vec3,
    perp_a: Vec3,
    perp_b: Vec3,
    radius: f32,
    segments: u32,
    color: Vec4,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let two_pi = std::f32::consts::TAU;

    // Apex.
    let apex_idx = vertices.len() as u32;
    vertices.push(MeshVertex::new(tip, color));

    // Base ring (around the cone's base).
    let ring_start = vertices.len() as u32;
    for i in 0..segments {
        let theta = (i as f32) / (segments as f32) * two_pi;
        let offset = perp_a * theta.cos() * radius + perp_b * theta.sin() * radius;
        vertices.push(MeshVertex::new(base_center + offset, color));
    }

    // Lateral triangles (apex → ring i → ring i+1).
    for i in 0..segments {
        let next = (i + 1) % segments;
        indices.extend_from_slice(&[apex_idx, ring_start + i, ring_start + next]);
    }

    // Base cap (fan from base_center).
    let center_idx = vertices.len() as u32;
    vertices.push(MeshVertex::new(base_center, color));
    for i in 0..segments {
        let next = (i + 1) % segments;
        // Wind opposite to the lateral triangles so the cap normal
        // points away from the apex.
        indices.extend_from_slice(&[center_idx, ring_start + next, ring_start + i]);
    }
}
