use glam::{Quat, Vec3};
use rapier3d::na::Vector3;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{BodyDesc, BodyHandle, BodyKind, PhysicsBackend, RayHit};

use super::conv::{collider_for, isometry, na_to_vec3, point, vec3_to_na};

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
