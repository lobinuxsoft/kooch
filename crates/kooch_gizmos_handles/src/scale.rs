//! [`ScaleHandle`] — drag a small cube to scale the entity.
//!
//! Two flavors:
//!
//! - `axis = Some(Axis::X | Y | Z)` → axis-aligned scale handle. Cube
//!   sits at `origin + frame.world_axis(axis) * length` (rotates with
//!   the handle frame so Local mode tracks entity rotation).
//! - `axis = None` → center cube at `origin`. Drag scales uniformly.
//!
//! The math respects the Local/World toggle:
//!
//! - **Local mode** (`frame.basis = entity rotation`): the user drags
//!   along an entity-local axis. We multiply that local-axis scale
//!   by the factor — straightforward.
//! - **World mode** (`frame.basis = identity`): the user drags along a
//!   world axis. We construct a world-space stretch matrix
//!   `S_world = I + (f - 1) · outer(d, d)` along the dragged
//!   direction `d`, then convert it to entity-local via
//!   `S_local = R⁻¹ · S_world · R` (where `R = entity_world_rotation`).
//!   The diagonal of `S_local` is the per-axis multiplicative factor.
//!   This produces correct world-space stretching whenever the
//!   entity rotation is axis-aligned; for arbitrary rotations it
//!   approximates by discarding the off-diagonal shear (lossy but
//!   matches the standard editor compromise).

use glam::{Mat3, Vec3, Vec4};
use kooch_gizmos::Gizmos;

use crate::{Axis, DragInfo, Handle, HandleFrame, HandleMode, HandleState, Ray, TransformDelta};

/// Axis-aligned or center scale handle. See module docs.
pub struct ScaleHandle {
    pub axis: Option<Axis>,
    pub length: f32,
    pub cube_half_size: f32,
}

impl ScaleHandle {
    pub fn axis(axis: Axis) -> Self {
        Self {
            axis: Some(axis),
            length: 1.0,
            cube_half_size: 0.08,
        }
    }

    pub fn center() -> Self {
        Self {
            axis: None,
            length: 1.0,
            cube_half_size: 0.06,
        }
    }

    fn idle_color(&self) -> Vec3 {
        match self.axis {
            Some(a) => a.base_color(),
            None => Vec3::new(0.95, 0.95, 0.95),
        }
    }

    /// Local-space cube center (before applying frame.basis). For axis
    /// handles it's `axis * length`; for the center cube it's the origin.
    fn local_cube_center(&self) -> Vec3 {
        match self.axis {
            Some(axis) => axis.vec() * self.length,
            None => Vec3::ZERO,
        }
    }

    /// World-space cube center (after applying frame.basis).
    fn cube_center(&self, frame: HandleFrame) -> Vec3 {
        frame.origin + frame.basis * self.local_cube_center()
    }
}

impl Handle for ScaleHandle {
    fn mode(&self) -> HandleMode {
        HandleMode::Scale
    }

    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState) {
        let rgb = match state {
            HandleState::Idle => self.idle_color(),
            HandleState::Hover => bright(self.idle_color()),
            HandleState::Dragging => Vec3::new(1.0, 0.85, 0.2),
        };
        // OBB rotated by frame.basis: cubes follow entity rotation in
        // Local mode, world-aligned in World mode.
        gizmos.filled_obb(
            self.cube_center(frame),
            frame.basis,
            Vec3::splat(self.cube_half_size),
            Vec4::new(rgb.x, rgb.y, rgb.z, 1.0),
        );
    }

    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32> {
        // Transform the ray into the basis-local frame so the cube is
        // axis-aligned for picking.
        let basis_inv = frame.basis.transpose();
        let local_origin = basis_inv * (ray.origin - frame.origin);
        let local_dir = basis_inv * ray.direction;
        let center = self.local_cube_center();
        let half = Vec3::splat(self.cube_half_size);
        ray_vs_aabb(local_origin, local_dir, center - half, center + half)
    }

    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> TransformDelta {
        let direction = match self.axis {
            Some(axis) => frame.world_axis(axis),
            None => {
                // Uniform scale doesn't have a single direction; use
                // world X as a stable scalar reference. Returns the
                // same factor regardless because the resulting matrix
                // is `f * I`.
                Vec3::X
            }
        };

        let last_s = project_ray_to_axis(drag.last_ray, frame.origin, direction);
        let current_s = project_ray_to_axis(drag.current_ray, frame.origin, direction);
        let distance = current_s - last_s;
        let factor = (1.0 + distance / self.length.max(0.001)).max(0.01);

        if self.axis.is_none() {
            // Uniform scale: f * I, identical in any rotation frame.
            return TransformDelta::Scale(Vec3::splat(factor));
        }

        // World-space stretch along `direction`:
        //   S_world = I + (f - 1) * outer(direction, direction)
        let d = direction;
        let outer = Mat3::from_cols(d * d.x, d * d.y, d * d.z);
        let s_world = Mat3::IDENTITY + outer * (factor - 1.0);

        // Convert to entity-local: S_local = R⁻¹ * S_world * R.
        // For Local mode this collapses to a diagonal `factor` on the
        // dragged local axis (rotation cancels out). For World mode
        // it produces the correct world-axis stretch in local space,
        // approximated as a diagonal (off-diagonal shear discarded).
        let r = frame.entity_world_rotation;
        let s_local = r.transpose() * s_world * r;
        let local_factor = Vec3::new(s_local.x_axis.x, s_local.y_axis.y, s_local.z_axis.z);
        TransformDelta::Scale(local_factor.max(Vec3::splat(0.01)))
    }
}

// ---------------------------------------------------------------------------
// Math helpers
// ---------------------------------------------------------------------------

fn bright(c: Vec3) -> Vec3 {
    c.lerp(Vec3::ONE, 0.4)
}

fn project_ray_to_axis(ray: Ray, axis_origin: Vec3, axis_dir: Vec3) -> f32 {
    let u = ray.origin - axis_origin;
    let b = ray.direction.dot(axis_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return 0.0;
    }
    let d_ru = ray.direction.dot(u);
    let e_au = axis_dir.dot(u);
    (e_au - b * d_ru) / denom
}

/// Slab-method ray-vs-AABB taking the ray endpoints already in the
/// box's coordinate frame (caller responsible for transforming).
fn ray_vs_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let inv = Vec3::ONE / dir;
    let t1 = (min - origin) * inv;
    let t2 = (max - origin) * inv;
    let t_min = t1.min(t2);
    let t_max = t1.max(t2);
    let t_enter = t_min.max_element();
    let t_exit = t_max.min_element();
    if t_enter > t_exit || t_exit < 0.0 {
        None
    } else {
        Some(t_enter.max(0.0))
    }
}
