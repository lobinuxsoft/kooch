//! The Rapier backend's state and helpers; [`contract`] is its [`PhysicsBackend`] impl, which must
//! be one `impl` block.

mod contract;
mod queries;

use glam::Vec3;
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{BodyHandle, BrokenJoint, ColliderHandle, JointHandle, PhysicsBackend};

use super::events::{EventCollector, parent_of};
use super::joints::{JointEntry, JointRef, linear_impulse};

/// Rapier-backed [`PhysicsBackend`]: its pipeline state plus a slotmap from [`BodyHandle`] to
/// Rapier handles, stable across steps.
pub struct RapierBackend {
    // Visible to the sibling `debug` module, which walks them to describe
    // the world; private to everything else.
    pub(super) bodies: RigidBodySet,
    pub(super) colliders: ColliderSet,
    pub(super) impulse_joints: ImpulseJointSet,
    pub(super) multibody_joints: MultibodyJointSet,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    pub(super) narrow_phase: NarrowPhase,
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
    /// What the last step reported. Filled from inside `step`, drained
    /// afterwards — see [`super::events`].
    collector: EventCollector,
    /// Rapier body → engine body, so events and `query_ray` avoid scanning every body.
    body_lookup: std::collections::HashMap<RigidBodyHandle, BodyHandle>,
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
            collector: EventCollector::default(),
            body_lookup: std::collections::HashMap::new(),
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

    /// The world's length unit in metres; solver tolerances scale with it, and a km world at 1 m
    /// jitters like a solver bug.
    pub fn set_length_unit(&mut self, metres: f32) {
        self.integration_parameters.length_unit = metres.max(f32::EPSILON);
    }

    /// The world's unit of length, in metres.
    pub fn length_unit(&self) -> f32 {
        self.integration_parameters.length_unit
    }

    /// Solver iterations per step: stiffer stacks for linear cost, at least 1.
    pub fn set_solver_iterations(&mut self, iterations: usize) {
        self.integration_parameters.num_solver_iterations = iterations.max(1);
    }

    /// Number of solver iterations per step.
    pub fn solver_iterations(&self) -> usize {
        self.integration_parameters.num_solver_iterations
    }

    /// Publishes a collider's AABB so queries see bodies spawned or moved since the last step — the
    /// editor queries a world it never steps. `set_aabb` leaves `step`'s bookkeeping alone.
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

    /// Settles a body's mass properties now instead of at the next step.
    fn recompute_mass_properties(&mut self, body: RigidBodyHandle) {
        let colliders = &self.colliders;
        if let Some(rb) = self.bodies.get_mut(body) {
            rb.recompute_mass_properties_from_colliders(colliders);
        }
    }

    /// The engine bodies owning a reported collider pair.
    fn bodies_of(
        &self,
        colliders: (RapierColliderHandle, RapierColliderHandle),
    ) -> Option<(BodyHandle, BodyHandle)> {
        let a = parent_of(&self.colliders, colliders.0)?;
        let b = parent_of(&self.colliders, colliders.1)?;
        Some((*self.body_lookup.get(&a)?, *self.body_lookup.get(&b)?))
    }

    /// Removes joints whose solver impulse exceeds the author's threshold — reading output, not a
    /// second solver. Impulse joints only; multibody impulses do not exist to read.
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

    /// Retires engine handles of joints on a body rapier removes, or a later `remove_joint` hits a
    /// reissued handle.
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
