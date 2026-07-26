use glam::{Quat, Vec3};
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{
    BodyDesc, BodyHandle, BodyKind, ColliderHandle, CollisionShape, PhysicsBackend, RayHit,
};

use super::conv::{collider_for, collider_for_pose};

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
    physics_pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    gravity: Vec3,
    handles: SlotMap<BodyHandle, RigidBodyHandle>,
    /// Shapes attached beyond the one each body was created with.
    collider_handles: SlotMap<ColliderHandle, RapierColliderHandle>,
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
            physics_pipeline: PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
            gravity: Vec3::new(0.0, -9.81, 0.0),
            handles: SlotMap::with_key(),
            collider_handles: SlotMap::with_key(),
        }
    }

    /// Overrides the gravity vector. Default is `(0, -9.81, 0)`.
    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    /// Returns current gravity.
    pub fn gravity(&self) -> Vec3 {
        self.gravity
    }

    /// Sets the world's unit of length, in metres.
    ///
    /// The solver's internal tolerances — contact slop, linear sleep
    /// thresholds, prediction distance — are all expressed as fractions
    /// of this. A planet-scale world working in kilometres with the
    /// default 1 m gets tolerances a thousand times too tight, which
    /// shows up as jitter that reads like a solver bug rather than a
    /// units mistake.
    pub fn set_length_unit(&mut self, metres: f32) {
        self.integration_parameters.length_unit = metres.max(f32::EPSILON);
    }

    /// The world's unit of length, in metres.
    pub fn length_unit(&self) -> f32 {
        self.integration_parameters.length_unit
    }

    /// Sets the number of solver iterations per step.
    ///
    /// More iterations buy stiffer stacks and less penetration for
    /// linear cost. Clamped to at least 1 — zero would leave contacts
    /// entirely unresolved.
    pub fn set_solver_iterations(&mut self, iterations: usize) {
        self.integration_parameters.num_solver_iterations = iterations.max(1);
    }

    /// Number of solver iterations per step.
    pub fn solver_iterations(&self) -> usize {
        self.integration_parameters.num_solver_iterations
    }

    /// Publishes a collider's AABB into the broad-phase BVH.
    ///
    /// Scene queries read that BVH directly, and the broad-phase only
    /// fills it while stepping — so without this a body spawned or
    /// teleported since the last step is invisible to a raycast. Tools
    /// query a world they are not simulating (click-to-pick in the
    /// editor), so "visible only after a step" is not good enough.
    ///
    /// Goes through `set_aabb` rather than a broad-phase update so the
    /// modified-collider bookkeeping `step` depends on is left alone.
    fn publish_aabb(&mut self, collider: RapierColliderHandle) {
        let Some(aabb) = self.colliders.get(collider).map(|c| c.compute_aabb()) else {
            return;
        };
        self.broad_phase
            .set_aabb(&self.integration_parameters, collider, aabb);
    }

    /// Republishes every collider attached to a body. Used after a
    /// teleport, which can move several colliders at once.
    fn publish_body_aabbs(&mut self, body: RigidBodyHandle) {
        let colliders: Vec<RapierColliderHandle> = self
            .bodies
            .get(body)
            .map(|b| b.colliders().to_vec())
            .unwrap_or_default();
        for collider in colliders {
            self.publish_aabb(collider);
        }
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
        // breaking arrive in follow-ups.
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
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
        let rb = RigidBodyBuilder::new(body_type)
            .pose(Pose::from_parts(desc.position, desc.rotation))
            .additional_mass(desc.mass.max(0.0))
            .build();
        let collider = collider_for(desc.shape, desc.shape_offset);
        let rb_handle = self.bodies.insert(rb);
        let collider_handle =
            self.colliders
                .insert_with_parent(collider, rb_handle, &mut self.bodies);
        self.publish_aabb(collider_handle);
        self.handles.insert(rb_handle)
    }

    fn attach_collider(
        &mut self,
        body: BodyHandle,
        shape: CollisionShape,
        offset: Vec3,
        rotation: Quat,
    ) -> Option<ColliderHandle> {
        let rb_handle = *self.handles.get(body)?;
        let collider = collider_for_pose(shape, offset, rotation);
        let handle = self
            .colliders
            .insert_with_parent(collider, rb_handle, &mut self.bodies);
        self.publish_aabb(handle);
        Some(self.collider_handles.insert(handle))
    }

    fn detach_collider(&mut self, handle: ColliderHandle) {
        let Some(collider_handle) = self.collider_handles.remove(handle) else {
            return;
        };
        // `wake_up: true` — the body's shape changed, so a sleeping body
        // has to re-evaluate contacts or it keeps colliding with a shape
        // that is gone.
        self.colliders
            .remove(collider_handle, &mut self.islands, &mut self.bodies, true);
    }

    fn collider_count(&self, body: BodyHandle) -> Option<usize> {
        let rb_handle = *self.handles.get(body)?;
        Some(self.bodies.get(rb_handle)?.colliders().len())
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
        Some((body.translation(), *body.rotation()))
    }

    fn set_transform(&mut self, handle: BodyHandle, position: Vec3, rotation: Quat) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        let pose = Pose::from_parts(position, rotation);
        // A kinematic body driven by `set_position` teleports: the solver
        // sees no motion, so it passes through dynamic bodies instead of
        // pushing them. `set_next_kinematic_position` makes the step
        // derive a velocity from the delta, which is what "kinematic
        // bodies push dynamics out of the way" actually means.
        match body.body_type() {
            RigidBodyType::KinematicPositionBased | RigidBodyType::KinematicVelocityBased => {
                body.set_next_kinematic_position(pose)
            }
            _ => body.set_position(pose, true),
        }
        self.publish_body_aabbs(rb_handle);
    }

    fn linear_velocity(&self, handle: BodyHandle) -> Option<Vec3> {
        let rb_handle = *self.handles.get(handle)?;
        let body = self.bodies.get(rb_handle)?;
        Some(body.linvel())
    }

    fn set_linear_velocity(&mut self, handle: BodyHandle, velocity: Vec3) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        body.set_linvel(velocity, true);
    }

    fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<RayHit> {
        let ray = Ray::new(origin, dir);
        // Since 0.34 the query pipeline is a view borrowed from the
        // broad-phase BVH rather than a mirror kept in sync by hand — so
        // a query always sees the current colliders, with no `update`
        // call to forget after a spawn or a teleport.
        let pipeline = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::default(),
        );
        let (collider_handle, toi) = pipeline.cast_ray_and_get_normal(&ray, max_t, true)?;
        let collider = self.colliders.get(collider_handle)?;
        let parent = collider.parent()?;
        // Reverse-lookup: which BodyHandle owns this Rapier handle?
        let body_handle = self
            .handles
            .iter()
            .find(|&(_, rb)| *rb == parent)
            .map(|(h, _)| h)?;
        Some(RayHit {
            body: body_handle,
            t: toi.time_of_impact,
            point: ray.origin + ray.dir * toi.time_of_impact,
            normal: toi.normal,
        })
    }
}
