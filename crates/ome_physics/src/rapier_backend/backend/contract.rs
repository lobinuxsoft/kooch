//! The [`PhysicsBackend`] implementation.
//!
//! One file because Rust does not allow an `impl Trait for Type` to be
//! spread across several. It reads in the order the trait declares:
//! stepping and gravity, bodies, colliders, joints, events, then queries.

use glam::{Quat, Vec3};
use rapier3d::geometry::ColliderHandle as RapierColliderHandle;
use rapier3d::prelude::*;
use slotmap::SlotMap;

use super::RapierBackend;
use crate::backend::{
    BodyDesc, BodyHandle, BodyKind, BrokenJoint, ColliderHandle, ColliderInteraction,
    CollisionEvent, CollisionShape, ContactForceEvent, JointDesc, JointHandle, PhysicsBackend,
    RayHit, SurfaceMaterial,
};

use super::super::conv::{collider_for, collider_for_pose, mass_properties_for};
use super::super::events::{EventCollector, parent_of};
use super::super::joints::{JointEntry, JointRef, generic_joint_for, linear_impulse};

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
            &(),             // physics hooks (#561 leaves these for later)
            &self.collector, // collected now, delivered after the step
        );
        self.break_overloaded_joints();
    }

    fn gravity(&self) -> Vec3 {
        self.gravity
    }

    fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    fn add_body(&mut self, desc: BodyDesc) -> BodyHandle {
        let body_type = match desc.kind {
            BodyKind::Dynamic => RigidBodyType::Dynamic,
            BodyKind::Kinematic => RigidBodyType::KinematicPositionBased,
            BodyKind::Static => RigidBodyType::Fixed,
        };
        // `additional_mass_properties`, not `additional_mass`: the latter
        // is *added* to whatever the colliders' density implies, so the
        // authored number would mean a different weight for every shape.
        // With massless colliders this is the body's entire mass
        // properties, and it is exactly what the author typed.
        let rb = RigidBodyBuilder::new(body_type)
            .pose(Pose::from_parts(desc.position, desc.rotation))
            .additional_mass_properties(mass_properties_for(
                desc.shape,
                desc.mass,
                desc.center_of_mass,
            ))
            .gravity_scale(desc.gravity_scale)
            .linear_damping(desc.damping.sanitised().linear)
            .angular_damping(desc.damping.sanitised().angular)
            .build();
        let collider = collider_for(
            desc.shape,
            desc.shape_offset,
            desc.material,
            desc.interaction,
        );
        let rb_handle = self.bodies.insert(rb);
        let collider_handle =
            self.colliders
                .insert_with_parent(collider, rb_handle, &mut self.bodies);
        self.publish_aabb(collider_handle);
        // Rapier defers this to the next step. The editor authors a world
        // it does not simulate, so without it every mass and centre of
        // mass read before pressing Play is stale — and a physics debug
        // view (#563) would draw the wrong point.
        self.recompute_mass_properties(rb_handle);
        let handle = self.handles.insert(rb_handle);
        self.body_lookup.insert(rb_handle, handle);
        handle
    }

    fn attach_collider(
        &mut self,
        body: BodyHandle,
        shape: CollisionShape,
        offset: Vec3,
        rotation: Quat,
        material: SurfaceMaterial,
        interaction: ColliderInteraction,
    ) -> Option<ColliderHandle> {
        let rb_handle = *self.handles.get(body)?;
        let collider = collider_for_pose(shape, offset, rotation, material, interaction);
        let handle = self
            .colliders
            .insert_with_parent(collider, rb_handle, &mut self.bodies);
        self.publish_aabb(handle);
        // Massless, so this changes nothing today — but it keeps "the
        // properties are current" true after every shape change rather
        // than only after the first.
        self.recompute_mass_properties(rb_handle);
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
        self.body_lookup.remove(&rb_handle);
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

    fn mass(&self, handle: BodyHandle) -> Option<f32> {
        let rb_handle = *self.handles.get(handle)?;
        Some(self.bodies.get(rb_handle)?.mass())
    }

    fn center_of_mass(&self, handle: BodyHandle) -> Option<Vec3> {
        let rb_handle = *self.handles.get(handle)?;
        Some(
            self.bodies
                .get(rb_handle)?
                .mass_properties()
                .local_mprops
                .local_com,
        )
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

    fn take_collision_events(&mut self) -> Vec<CollisionEvent> {
        self.collector
            .drain_collisions()
            .into_iter()
            .filter_map(|raw| {
                // A pair either side of which has no body is dropped: an
                // unparented collider has no entity to report against, and
                // inventing one would be worse than saying nothing.
                let (a, b) = self.bodies_of(raw.colliders)?;
                Some(CollisionEvent {
                    a,
                    b,
                    started: raw.started,
                    sensor: raw.sensor,
                })
            })
            .collect()
    }

    fn take_contact_force_events(&mut self) -> Vec<ContactForceEvent> {
        self.collector
            .drain_forces()
            .into_iter()
            .filter_map(|raw| {
                let (a, b) = self.bodies_of(raw.colliders)?;
                Some(ContactForceEvent {
                    a,
                    b,
                    total_force_magnitude: raw.total_force_magnitude,
                    max_force_magnitude: raw.max_force_magnitude,
                })
            })
            .collect()
    }

    fn take_broken_joints(&mut self) -> Vec<BrokenJoint> {
        std::mem::take(&mut self.broken_joints)
    }

    #[cfg(feature = "debug-render")]
    fn debug_lines(
        &self,
        categories: crate::backend::DebugCategories,
        out: &mut Vec<crate::backend::DebugLine>,
    ) {
        self.collect_debug_lines(categories, out);
    }

    fn apply_impulse(&mut self, handle: BodyHandle, impulse: Vec3, wake: bool) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        body.apply_impulse(impulse, wake);
    }

    fn is_sleeping(&self, handle: BodyHandle) -> Option<bool> {
        let rb_handle = *self.handles.get(handle)?;
        Some(self.bodies.get(rb_handle)?.is_sleeping())
    }

    fn angular_velocity(&self, handle: BodyHandle) -> Option<Vec3> {
        let rb_handle = *self.handles.get(handle)?;
        Some(self.bodies.get(rb_handle)?.angvel())
    }

    fn set_angular_velocity(&mut self, handle: BodyHandle, velocity: Vec3) {
        let Some(&rb_handle) = self.handles.get(handle) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(rb_handle) else {
            return;
        };
        body.set_angvel(velocity, true);
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
        // The reverse map, rather than the linear scan this used to do:
        // events made the same lookup a per-event cost, so it earned an
        // index, and the raycast gets it for free.
        let body_handle = *self.body_lookup.get(&parent)?;
        Some(RayHit {
            body: body_handle,
            t: toi.time_of_impact,
            point: ray.origin + ray.dir * toi.time_of_impact,
            normal: toi.normal,
        })
    }
}
