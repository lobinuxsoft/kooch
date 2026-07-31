//! The Rapier backend: its state, and how it meets the engine's contract.
//!
//! Split by that distinction rather than by method count. This file is
//! *what the backend is* — the pipeline state it owns, the knobs it
//! exposes, and the private helpers that keep Rapier's bookkeeping
//! consistent. [`contract`] is *how it satisfies* [`PhysicsBackend`],
//! which Rust requires to live in a single `impl` block and therefore a
//! single file.

mod contract;

use glam::Vec3;
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use crate::backend::{BodyHandle, BrokenJoint, ColliderHandle, JointHandle, PhysicsBackend};

use super::events::{EventCollector, parent_of};
use super::joints::{JointEntry, JointRef, linear_impulse};

/// Rapier-backed [`PhysicsBackend`].
///
/// Stores its own Rapier pipeline state plus a slotmap mapping engine
/// [`BodyHandle`]s to `(RigidBodyHandle, ColliderHandle)` pairs. Handles
/// are stable across `step` calls; `remove_body` evicts both Rapier-side
/// and slotmap-side entries.
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
    /// Rapier body → engine body, so an event does not cost a linear scan
    /// of every body to answer "whose collider was that".
    ///
    /// `query_ray` used to do exactly that scan; it uses this now too.
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
