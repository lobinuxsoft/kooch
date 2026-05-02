//! [`RapierBackend`] — CPU rigid body solver via Rapier3D 0.22.
//!
//! Owns the Rapier pipeline + body / collider sets. Maps engine
//! [`BodyHandle`]s (slotmap keys) to Rapier's internal handles. All public
//! API uses glam types; nalgebra conversions are confined to this module.
//!
//! # Defaults
//!
//! - Gravity: `(0, -9.81, 0)`. Override via [`RapierBackend::set_gravity`].
//! - Integration parameters: Rapier defaults (60 Hz hint, default solver
//!   iterations, CCD enabled per-body but on-demand).
//! - Friction / restitution coefficients: Rapier defaults (0.5 / 0.0).
//!   PR-1 doesn't expose material override; lands with #137.

use glam::{Quat, Vec3};
use rapier3d::na::{self, Translation3, Unit, UnitQuaternion, Vector3};
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{
    BodyDesc, BodyHandle, BodyKind, CollisionShape, PhysicsBackend, RayHit,
};

/// Rapier-backed [`PhysicsBackend`].
///
/// Stores its own Rapier pipeline state plus a slotmap mapping engine
/// [`BodyHandle`]s to `(RigidBodyHandle, ColliderHandle)` pairs. Handles
/// are stable across `step` calls; `remove_body` evicts both Rapier-side
/// and slotmap-side entries.
pub struct RapierBackend {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    physics_pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    gravity: Vector3<f32>,
    handles: SlotMap<BodyHandle, RigidBodyHandle>,
}

impl RapierBackend {
    pub fn new() -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            physics_pipeline: PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            handles: SlotMap::with_key(),
        }
    }

    /// Overrides the gravity vector. Default is `(0, -9.81, 0)`.
    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = vec3_to_na(gravity);
    }

    /// Returns current gravity.
    pub fn gravity(&self) -> Vec3 {
        na_to_vec3(self.gravity)
    }
}

impl Default for RapierBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsBackend for RapierBackend {
    fn step(&mut self, dt: f32) {
        self.integration_parameters.dt = dt;
        // No event collector or hooks for PR-1 — collision events / joint
        // breaking arrive in follow-ups. Pass the query pipeline so Rapier
        // auto-syncs it after the simulation step.
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(), // physics hooks
            &(), // event handler
        );
    }

    fn add_body(&mut self, desc: BodyDesc) -> BodyHandle {
        let body_type = match desc.kind {
            BodyKind::Dynamic => RigidBodyType::Dynamic,
            BodyKind::Kinematic => RigidBodyType::KinematicPositionBased,
            BodyKind::Static => RigidBodyType::Fixed,
        };
        let isometry = isometry(desc.position, desc.rotation);
        let rb = RigidBodyBuilder::new(body_type)
            .position(isometry)
            .additional_mass(desc.mass.max(0.0))
            .build();
        let collider = collider_for(desc.shape);
        let rb_handle = self.bodies.insert(rb);
        self.colliders
            .insert_with_parent(collider, rb_handle, &mut self.bodies);
        // Refresh the spatial index so `query_ray` works without
        // requiring a `step` first — important for tests + tools.
        self.query_pipeline.update(&self.colliders);
        self.handles.insert(rb_handle)
    }

    fn remove_body(&mut self, handle: BodyHandle) {
        let Some(rb_handle) = self.handles.remove(handle) else {
            return;
        };
        // Remove rigid body + its colliders + any joints.
        self.bodies.remove(
            rb_handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true, // remove attached colliders
        );
        self.query_pipeline.update(&self.colliders);
    }

    fn contains(&self, handle: BodyHandle) -> bool {
        self.handles.contains_key(handle)
    }

    fn body_count(&self) -> usize {
        self.handles.len()
    }

    fn get_transform(&self, handle: BodyHandle) -> Option<(Vec3, Quat)> {
        let rb_handle = *self.handles.get(handle)?;
        let body = self.bodies.get(rb_handle)?;
        let pos = body.translation();
        let rot = body.rotation();
        Some((
            Vec3::new(pos.x, pos.y, pos.z),
            Quat::from_xyzw(rot.i, rot.j, rot.k, rot.w),
        ))
    }

    fn set_transform(&mut self, handle: BodyHandle, position: Vec3, rotation: Quat) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        body.set_position(isometry(position, rotation), true);
        self.query_pipeline.update(&self.colliders);
    }

    fn linear_velocity(&self, handle: BodyHandle) -> Option<Vec3> {
        let rb_handle = *self.handles.get(handle)?;
        let body = self.bodies.get(rb_handle)?;
        let v = body.linvel();
        Some(Vec3::new(v.x, v.y, v.z))
    }

    fn set_linear_velocity(&mut self, handle: BodyHandle, velocity: Vec3) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        body.set_linvel(vec3_to_na(velocity), true);
    }

    fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit> {
        let ray = Ray::new(point(origin), vec3_to_na(dir));
        let filter = QueryFilter::default();
        let (collider_handle, toi) = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_t,
            true, // solid
            filter,
        )?;
        let collider = self.colliders.get(collider_handle)?;
        let parent = collider.parent()?;
        // Reverse-lookup: which BodyHandle owns this Rapier handle?
        let body_handle = self
            .handles
            .iter()
            .find(|&(_, rb)| *rb == parent)
            .map(|(h, _)| h)?;
        let hit_point = ray.origin + ray.dir * toi.time_of_impact;
        Some(RayHit {
            body: body_handle,
            t: toi.time_of_impact,
            point: Vec3::new(hit_point.x, hit_point.y, hit_point.z),
            normal: na_to_vec3(toi.normal),
        })
    }
}

// ─── conversion helpers ────────────────────────────────────────────────

fn vec3_to_na(v: Vec3) -> Vector3<f32> {
    Vector3::new(v.x, v.y, v.z)
}

fn na_to_vec3(v: Vector3<f32>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn point(v: Vec3) -> na::Point3<f32> {
    na::Point3::new(v.x, v.y, v.z)
}

fn isometry(pos: Vec3, rot: Quat) -> Isometry<Real> {
    let translation = Translation3::new(pos.x, pos.y, pos.z);
    let rotation = UnitQuaternion::from_quaternion(na::Quaternion::new(
        rot.w, rot.x, rot.y, rot.z,
    ));
    Isometry::from_parts(translation, rotation)
}

fn collider_for(shape: CollisionShape) -> Collider {
    match shape {
        CollisionShape::Sphere { radius } => ColliderBuilder::ball(radius).build(),
        CollisionShape::Cuboid { half_extents } => ColliderBuilder::cuboid(
            half_extents.x.max(1e-4),
            half_extents.y.max(1e-4),
            half_extents.z.max(1e-4),
        )
        .build(),
        CollisionShape::Capsule {
            radius,
            half_height,
        } => ColliderBuilder::capsule_y(half_height, radius).build(),
    }
}

#[allow(unused)]
fn _unit_y_helper() -> Unit<Vector3<f32>> {
    Vector3::y_axis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_backend_is_empty() {
        let backend = RapierBackend::new();
        assert_eq!(backend.body_count(), 0);
        assert_eq!(backend.gravity(), Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn add_body_increments_count_and_returns_live_handle() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 1.0 },
            1.0,
        ));
        assert_eq!(backend.body_count(), 1);
        assert!(backend.contains(handle));
        assert!(backend.get_transform(handle).is_some());
    }

    #[test]
    fn remove_body_invalidates_handle() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 1.0 },
            1.0,
        ));
        backend.remove_body(handle);
        assert_eq!(backend.body_count(), 0);
        assert!(!backend.contains(handle));
        assert!(backend.get_transform(handle).is_none());
    }

    #[test]
    fn dynamic_body_falls_under_gravity() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        let (initial_pos, _) = backend.get_transform(handle).unwrap();
        // Step ~16ms ten times (~160ms total) — body should fall noticeably.
        for _ in 0..10 {
            backend.step(0.016);
        }
        let (final_pos, _) = backend.get_transform(handle).unwrap();
        assert!(
            final_pos.y < initial_pos.y - 0.1,
            "body should have fallen at least 10 cm under gravity, got {} -> {}",
            initial_pos.y,
            final_pos.y,
        );
    }

    #[test]
    fn static_body_does_not_move() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::static_at(
            CollisionShape::Cuboid {
                half_extents: Vec3::splat(1.0),
            },
            Vec3::new(5.0, 0.0, 0.0),
        ));
        for _ in 0..30 {
            backend.step(0.016);
        }
        let (pos, _) = backend.get_transform(handle).unwrap();
        assert_eq!(pos, Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn dynamic_body_lands_on_static_floor() {
        let mut backend = RapierBackend::new();
        // Floor: large flat box at y = -0.5 (top surface at y = 0).
        let _floor = backend.add_body(BodyDesc::static_at(
            CollisionShape::Cuboid {
                half_extents: Vec3::new(10.0, 0.5, 10.0),
            },
            Vec3::new(0.0, -0.5, 0.0),
        ));
        // Sphere: radius 0.5 at y = 5.
        let mut desc = BodyDesc::dynamic(CollisionShape::Sphere { radius: 0.5 }, 1.0);
        desc.position = Vec3::new(0.0, 5.0, 0.0);
        let sphere = backend.add_body(desc);

        // Simulate ~2 seconds.
        for _ in 0..120 {
            backend.step(0.016);
        }

        let (pos, _) = backend.get_transform(sphere).unwrap();
        // Sphere should rest on top of the floor — center at ~y = 0.5
        // (radius). Allow some tolerance for solver settling + restitution.
        assert!(
            pos.y > 0.0 && pos.y < 1.0,
            "sphere should rest on floor, got y = {}",
            pos.y,
        );
    }

    #[test]
    fn ray_hits_static_body() {
        let mut backend = RapierBackend::new();
        let body = backend.add_body(BodyDesc::static_at(
            CollisionShape::Sphere { radius: 1.0 },
            Vec3::new(0.0, 0.0, 5.0),
        ));
        // Ray from origin shooting +Z.
        let hit = backend
            .query_ray(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 100.0)
            .expect("ray should hit the sphere at z=5");
        assert_eq!(hit.body, body);
        // Front of sphere is at z = 4, so t ≈ 4.
        assert!((hit.t - 4.0).abs() < 0.1);
    }

    #[test]
    fn ray_misses_when_no_geometry() {
        let backend = RapierBackend::new();
        let hit = backend.query_ray(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 100.0);
        assert!(hit.is_none());
    }

    #[test]
    fn set_transform_teleports_body() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        backend.set_transform(handle, Vec3::new(7.0, 8.0, 9.0), Quat::IDENTITY);
        let (pos, _) = backend.get_transform(handle).unwrap();
        assert_eq!(pos, Vec3::new(7.0, 8.0, 9.0));
    }

    #[test]
    fn linear_velocity_round_trip() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        let v = Vec3::new(1.0, 2.0, 3.0);
        backend.set_linear_velocity(handle, v);
        assert_eq!(backend.linear_velocity(handle), Some(v));
    }

    #[test]
    fn stale_handle_after_remove_is_safe() {
        let mut backend = RapierBackend::new();
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        backend.remove_body(handle);
        // No crash, no spurious data.
        assert!(backend.get_transform(handle).is_none());
        backend.set_transform(handle, Vec3::ONE, Quat::IDENTITY); // no-op
        backend.set_linear_velocity(handle, Vec3::ONE); // no-op
        assert!(backend.linear_velocity(handle).is_none());
    }

    #[test]
    fn gravity_override_takes_effect() {
        let mut backend = RapierBackend::new();
        backend.set_gravity(Vec3::new(0.0, 9.81, 0.0)); // upward
        let handle = backend.add_body(BodyDesc::dynamic(
            CollisionShape::Sphere { radius: 0.5 },
            1.0,
        ));
        let (initial, _) = backend.get_transform(handle).unwrap();
        for _ in 0..10 {
            backend.step(0.016);
        }
        let (final_pos, _) = backend.get_transform(handle).unwrap();
        assert!(
            final_pos.y > initial.y + 0.1,
            "inverted gravity should make body rise, got {} -> {}",
            initial.y,
            final_pos.y,
        );
    }
}
