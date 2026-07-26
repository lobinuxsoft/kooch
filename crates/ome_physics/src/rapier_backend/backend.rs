use glam::{Quat, Vec3};
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{
    BodyDesc, BodyHandle, BodyKind, BrokenJoint, ColliderHandle, CollisionShape, JointDesc,
    JointHandle, PhysicsBackend, RayHit,
};

use super::conv::{collider_for, collider_for_pose};
use super::joints::{JointEntry, JointRef, generic_joint_for, linear_impulse};

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
    /// Live joints, whichever of rapier's two sets holds each one.
    joint_handles: SlotMap<JointHandle, JointEntry>,
    /// Joints that broke since the last drain — see
    /// [`PhysicsBackend::take_broken_joints`].
    broken_joints: Vec<BrokenJoint>,
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
            joint_handles: SlotMap::with_key(),
            broken_joints: Vec::new(),
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

    /// Removes the joints the last step overloaded.
    ///
    /// Rapier has no breaking of its own — it reports the impulse it
    /// applied to hold each joint together, and this compares that against
    /// the author's threshold. Reading the solver's own output and removing
    /// a constraint is not a second solver; nothing here computes a force.
    ///
    /// Impulse joints only. A multibody joint is solved in reduced
    /// coordinates, where the constraint impulse is not a quantity that
    /// exists to be read — [`add_joint`](PhysicsBackend::add_joint) warns
    /// rather than pretending otherwise.
    fn break_overloaded_joints(&mut self) {
        // Collected first: removing a joint borrows the set mutably, and
        // breaking is rare enough that the allocation never happens on the
        // common path.
        let mut broken = Vec::new();
        for (handle, entry) in &self.joint_handles {
            if !entry.break_impulse.is_finite() {
                continue;
            }
            let JointRef::Impulse(rapier_handle) = entry.reference else {
                continue;
            };
            let Some(joint) = self.impulse_joints.get(rapier_handle) else {
                continue;
            };
            let impulse = linear_impulse(&joint.impulses);
            if impulse > entry.break_impulse {
                broken.push(BrokenJoint {
                    joint: handle,
                    body_a: entry.body_a,
                    body_b: entry.body_b,
                    impulse,
                });
            }
        }

        for event in broken {
            self.remove_joint(event.joint);
            self.broken_joints.push(event);
        }
    }

    /// Drops the bookkeeping for joints attached to a body rapier is about
    /// to remove.
    ///
    /// Rapier removes the joints themselves; what it cannot do is retire
    /// the engine-side handles that addressed them, and a
    /// [`JointHandle`] outliving its joint is how a later `remove_joint`
    /// would reach into the set with a handle rapier has reissued.
    fn forget_joints_of(&mut self, body: BodyHandle) {
        self.joint_handles
            .retain(|_, entry| entry.body_a != body && entry.body_b != body);
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
        // No event collector or hooks yet — collision events arrive with
        // #561. Joint breaking is handled after the step, from the impulses
        // the solver reports.
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
        self.break_overloaded_joints();
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
        // Before rapier removes them, so the engine-side handles retire
        // with the joints rather than outliving them.
        self.forget_joints_of(handle);
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

    fn add_joint(&mut self, desc: JointDesc) -> Option<JointHandle> {
        let body_a = *self.handles.get(desc.body_a)?;
        let body_b = *self.handles.get(desc.body_b)?;
        // A joint from a body to itself is not a constraint, it is a
        // degenerate system the solver has no answer for.
        if body_a == body_b {
            tracing::warn!(
                target: "ome_physics",
                "a joint cannot constrain a body to itself",
            );
            return None;
        }

        let joint = generic_joint_for(&desc);
        let reference = match desc.articulated {
            // `insert` returns `None` when the joint would close a cycle:
            // a multibody is a tree by construction, and a loop has to stay
            // on impulse joints.
            true => match self.multibody_joints.insert(body_a, body_b, joint, true) {
                Some(handle) => JointRef::Multibody(handle),
                None => {
                    tracing::warn!(
                        target: "ome_physics",
                        "an articulated joint cannot close a loop — rapier solves \
                         multibodies in reduced coordinates, where a cycle is not \
                         representable. Turn off Articulated for this joint",
                    );
                    return None;
                }
            },
            false => JointRef::Impulse(self.impulse_joints.insert(body_a, body_b, joint, true)),
        };

        if desc.articulated && desc.break_impulse.is_finite() {
            tracing::warn!(
                target: "ome_physics",
                "an articulated joint cannot break — its constraint impulse is not a \
                 quantity reduced-coordinate solving produces. Turn off Articulated to \
                 use a break threshold",
            );
        }

        Some(self.joint_handles.insert(JointEntry {
            reference,
            body_a: desc.body_a,
            body_b: desc.body_b,
            break_impulse: desc.break_impulse,
        }))
    }

    fn remove_joint(&mut self, handle: JointHandle) {
        let Some(entry) = self.joint_handles.remove(handle) else {
            return;
        };
        // `wake_up: true` — the bodies' constraints changed, and a sleeping
        // body would otherwise stay held by a joint that no longer exists.
        match entry.reference {
            JointRef::Impulse(joint) => {
                self.impulse_joints.remove(joint, true);
            }
            JointRef::Multibody(joint) => self.multibody_joints.remove(joint, true),
        }
    }

    fn joint_count(&self) -> usize {
        self.joint_handles.len()
    }

    fn joint_impulse(&self, handle: JointHandle) -> Option<f32> {
        match self.joint_handles.get(handle)?.reference {
            JointRef::Impulse(joint) => {
                Some(linear_impulse(&self.impulse_joints.get(joint)?.impulses))
            }
            // Reduced coordinates never form a constraint impulse: the
            // stretched configuration is not representable, so there is
            // nothing holding the joint together to measure.
            JointRef::Multibody(_) => None,
        }
    }

    fn take_broken_joints(&mut self) -> Vec<BrokenJoint> {
        std::mem::take(&mut self.broken_joints)
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
