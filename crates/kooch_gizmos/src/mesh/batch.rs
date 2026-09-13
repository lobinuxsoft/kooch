//! Mesh batch types — what the editor pushes per frame, what the
//! renderer consumes.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec2, Vec3, Vec4};

/// Single mesh vertex: position, RGBA colour and edge UV. Near an edge (u or v at 0 or 1) the
/// shader draws an opaque outline; `(0.5, 0.5)` means none.
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

/// One mesh draw queued for the gizmo mesh pass, in world space and wound CCW; the pipeline does
/// not cull.
#[derive(Debug, Clone, Default)]
pub struct MeshDraw {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

/// Per-frame mesh draws, filled through [`crate::Gizmos`] and drained by the mesh pass.
#[derive(Debug, Default)]
pub struct MeshBatch {
    pub draws: Vec<MeshDraw>,
}

impl MeshBatch {
    pub fn clear(&mut self) {
        self.draws.clear();
    }

    /// Pushes a filled quad from four CCW corners; its edge UVs give it an opaque outline without a
    /// line pass.
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

    /// Pushes a filled axis-aligned box centred at `center`. Each face carries its own edge UVs, so
    /// it reads as a wireframe with translucent fill.
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

    /// Pushes a filled oriented box. Like [`Self::filled_aabb`] but the
    /// 6 faces are rotated by `basis` (each column is a face axis).
    /// Use to draw cubes that follow a Local-mode entity rotation.
    pub fn filled_obb(&mut self, center: Vec3, basis: Mat3, half_extents: Vec3, color: Vec4) {
        let h = half_extents;
        let bx = basis.x_axis;
        let by = basis.y_axis;
        let bz = basis.z_axis;
        let p = |sx: f32, sy: f32, sz: f32| {
            center + bx * (h.x * sx) + by * (h.y * sy) + bz * (h.z * sz)
        };
        // Same 6 faces / 24 vertices layout as `filled_aabb`, but with
        // basis-rotated face axes instead of cardinal world axes.
        let faces: [[Vec3; 4]; 6] = [
            // -X face
            [
                p(-1.0, -1.0, 1.0),
                p(-1.0, -1.0, -1.0),
                p(-1.0, 1.0, -1.0),
                p(-1.0, 1.0, 1.0),
            ],
            // +X
            [
                p(1.0, -1.0, -1.0),
                p(1.0, -1.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(1.0, 1.0, -1.0),
            ],
            // -Y
            [
                p(-1.0, -1.0, -1.0),
                p(1.0, -1.0, -1.0),
                p(1.0, -1.0, 1.0),
                p(-1.0, -1.0, 1.0),
            ],
            // +Y
            [
                p(-1.0, 1.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(1.0, 1.0, -1.0),
                p(-1.0, 1.0, -1.0),
            ],
            // -Z
            [
                p(1.0, -1.0, -1.0),
                p(-1.0, -1.0, -1.0),
                p(-1.0, 1.0, -1.0),
                p(1.0, 1.0, -1.0),
            ],
            // +Z
            [
                p(-1.0, -1.0, 1.0),
                p(1.0, -1.0, 1.0),
                p(1.0, 1.0, 1.0),
                p(-1.0, 1.0, 1.0),
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

    /// Pushes a filled torus around `axis` through `center`, 32 × 8 segments, solid with no
    /// outline.
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

    /// Pushes a solid 3D arrow from `base` to `tip`: an octagonal shaft and cone head, with no
    /// outline.
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
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
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
