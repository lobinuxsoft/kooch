//! [`Collider`] — the shape an entity presents to the solver, and how
//! that surface behaves on contact.
//!
//! Same discriminant rule as [`PhysicsBody`](super::PhysicsBody): `shape` is
//! a `u32` with a choice set, because reflection cannot express an enum.
//! The choice set, and which fields each shape reads, live in
//! [`shapes`]; the surface and filtering vocabularies live in [`groups`].

mod groups;
mod shapes;
mod spec;

pub use groups::{
    COMBINE_AVERAGE, COMBINE_CHOICES, COMBINE_CLAMPED_SUM, COMBINE_MAX, COMBINE_MIN,
    COMBINE_MULTIPLY, GROUP_BITS,
};
pub use shapes::{
    BORDER_RADIUS_WHEN, ENDPOINTS_WHEN, HALF_EXTENTS_WHEN, HALF_HEIGHT_WHEN, MESH_DERIVED,
    MESH_WHEN, NORMAL_WHEN, POINT_C_WHEN, RADIUS_WHEN, SHAPE_CAPSULE, SHAPE_CHOICES, SHAPE_CONE,
    SHAPE_CONVEX_DECOMPOSITION, SHAPE_CONVEX_HULL, SHAPE_CUBOID, SHAPE_CYLINDER, SHAPE_HALF_SPACE,
    SHAPE_POLYLINE, SHAPE_ROUND_CYLINDER, SHAPE_SEGMENT, SHAPE_SPHERE, SHAPE_TRIANGLE,
    SHAPE_TRIMESH, SHAPE_VOXELIZED_MESH, SHAPE_VOXELS, VOXEL_SIZE_WHEN, VOXEL_SOLID_WHEN,
    is_mesh_derived,
};
pub use spec::ShapeSpec;

use groups::combine_rule;

use glam::Vec3;

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::FieldCondition;

use crate::backend::{
    ColliderInteraction, ColliderMeshCache, CollisionShape, InteractionMask, SurfaceMaterial,
};

/// The collision geometry attached to a body.
///
/// Named for what it becomes rather than for its geometry: a collider is
/// eventually geometry *plus* material and filtering (friction,
/// restitution, sensor flag, collision groups — #137), while
/// [`CollisionShape`] stays the pure geometry the backend consumes.
///
/// Only the fields belonging to the selected `shape` are read, and only
/// those are *shown* — see the `*_WHEN` conditions above. The rest keep
/// whatever they were, so switching shape back and forth does not lose the
/// other variant's parameters. Hiding is display only: every field is
/// still stored, still serialised, still round-trips through a scene.
///
/// # Default
///
/// A unit sphere.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct Collider {
    /// Which geometry to use. One of the `SHAPE_*` constants.
    #[reflect(choices = SHAPE_CHOICES)]
    pub shape: u32,
    /// Sphere and capsule radius.
    #[reflect(shown_when = RADIUS_WHEN)]
    pub radius: f32,
    /// Cuboid half-extents.
    #[reflect(shown_when = HALF_EXTENTS_WHEN)]
    pub half_extents: Vec3,
    /// Half the length along Y, excluding a capsule's caps.
    #[reflect(shown_when = HALF_HEIGHT_WHEN)]
    pub half_height: f32,
    /// How far the rounded cylinder's rim is filleted.
    ///
    /// A sharp rim gives the solver one contact point to resolve, and a
    /// wheel or a barrel rolling over a box edge catches on it. The
    /// fillet costs nothing and is what stops the snag.
    #[reflect(shown_when = BORDER_RADIUS_WHEN)]
    pub border_radius: f32,
    /// Which way the half-space's solid side faces away from.
    ///
    /// Normalised when the shape is built, and defaulted to up when it
    /// has no direction to give — a plane with no side is one rapier
    /// cannot build and the author cannot see.
    #[reflect(shown_when = NORMAL_WHEN)]
    pub normal: Vec3,
    /// First corner of a segment or a triangle, in the shape's local
    /// space.
    #[reflect(shown_when = ENDPOINTS_WHEN)]
    pub point_a: Vec3,
    /// Second corner of a segment or a triangle.
    #[reflect(shown_when = ENDPOINTS_WHEN)]
    pub point_b: Vec3,
    /// Third corner of a triangle.
    #[reflect(shown_when = POINT_C_WHEN)]
    pub point_c: Vec3,
    /// The mesh a mesh-derived shape is built from.
    ///
    /// A hull, a decomposition or a trimesh cannot be typed in, so they
    /// name a mesh and something outside physics resolves it — see
    /// [`ColliderMeshCache`]. Usually the same mesh the entity draws, and
    /// deliberately not assumed to be: colliding against a simplified
    /// stand-in is the whole point of authoring it separately.
    #[reflect(shown_when = MESH_WHEN)]
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// Edge length of one voxel cell.
    ///
    /// The cost knob: halving it multiplies the cell count by eight, and
    /// the voxel shape only beats a trimesh while it stays coarse.
    #[reflect(shown_when = VOXEL_SIZE_WHEN)]
    pub voxel_size: f32,
    /// Fill the voxelised mesh's interior, not only its shell.
    ///
    /// A shell is what a hollow prop wants; a body dropped *inside* a
    /// shell passes straight out through the other side.
    #[reflect(shown_when = VOXEL_SOLID_WHEN)]
    pub voxel_solid: bool,
    /// Resistance to sliding. 0 is frictionless; 1 is about rubber on dry
    /// tarmac. Above 1 is legal and useful for gameplay.
    pub friction: f32,
    /// How this collider's friction combines with the other one's. One of
    /// the `COMBINE_*` constants.
    ///
    /// **The pushier claim wins.** Rapier resolves a pair by taking the
    /// higher of the two discriminants, so a collider on Average against
    /// one on Max gets Max. A rule is less "how my surface behaves" than
    /// "how I insist on being combined".
    #[reflect(choices = COMBINE_CHOICES)]
    pub friction_rule: u32,
    /// Bounce. 0 absorbs the impact; 1 returns it, so a ball comes back to
    /// roughly the height it fell from.
    pub restitution: f32,
    /// How this collider's bounce combines with the other one's. Same
    /// max-wins resolution as `friction_rule`.
    #[reflect(choices = COMBINE_CHOICES)]
    pub restitution_rule: u32,
    /// Report overlap and never push — a trigger volume.
    ///
    /// A sensor is not a collider that gets ignored: rapier computes no
    /// contact manifold for it at all, so its events carry no contact
    /// information. Checkpoints, damage zones, detection ranges.
    pub sensor: bool,
    /// Raise an event when this collider starts or stops touching
    /// something.
    ///
    /// Off by default, and that is the design rather than an oversight:
    /// events are opt-in per collider in rapier, so a scene pays only for
    /// what it listens to.
    pub collision_events: bool,
    /// Raise an event when contact force exceeds
    /// `contact_force_threshold`.
    ///
    /// This is what tells "brushed the wall" from "hit it hard enough to
    /// take damage" without inspecting contacts every frame.
    pub contact_force_events: bool,
    /// The force, in newtons, above which a contact is worth reporting.
    #[reflect(shown_when = CONTACT_FORCE_WHEN)]
    pub contact_force_threshold: f32,
    /// Which groups this collider belongs to.
    ///
    /// A pair is considered only when each side's memberships intersect the
    /// other's filter — **both** directions, so being in a group the other
    /// side looks for is not enough on its own.
    #[reflect(bits = GROUP_BITS)]
    pub collision_memberships: u32,
    /// Which groups this collider will collide with.
    #[reflect(bits = GROUP_BITS)]
    pub collision_filter: u32,
    /// Which groups this collider is *solved* against, out of those it
    /// collides with.
    ///
    /// The pair of masks is the point: a projectile that should detect a
    /// wall without being stopped by it shares the wall's collision groups
    /// and not its solver groups.
    #[reflect(bits = GROUP_BITS)]
    pub solver_memberships: u32,
    /// Which groups this collider will be pushed by.
    #[reflect(bits = GROUP_BITS)]
    pub solver_filter: u32,
    /// The shape's centre, in the entity's local space.
    ///
    /// Moves the geometry inside the body without moving the body. A
    /// model whose pivot is not at its centre of volume needs this: a
    /// character pivoted at the feet wants its capsule half a body up, and
    /// a door pivoted on the hinge wants its box beside it rather than
    /// around it.
    pub center: Vec3,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: SHAPE_SPHERE,
            radius: 0.5,
            half_extents: Vec3::splat(0.5),
            half_height: 0.5,
            border_radius: 0.05,
            normal: Vec3::Y,
            point_a: Vec3::ZERO,
            point_b: Vec3::Y,
            point_c: Vec3::X,
            mesh: None,
            voxel_size: 0.25,
            voxel_solid: true,
            friction: 0.5,
            friction_rule: COMBINE_AVERAGE,
            restitution: 0.0,
            restitution_rule: COMBINE_AVERAGE,
            sensor: false,
            collision_events: false,
            contact_force_events: false,
            contact_force_threshold: 0.0,
            collision_memberships: u32::MAX,
            collision_filter: u32::MAX,
            solver_memberships: u32::MAX,
            solver_filter: u32::MAX,
            center: Vec3::ZERO,
        }
    }
}

impl Component for Collider {}

/// Which state reads `contact_force_threshold`: only a collider that asked
/// for force events.
pub static CONTACT_FORCE_WHEN: FieldCondition = FieldCondition {
    field: "contact_force_events",
    values: &[1],
};

impl Collider {
    /// The surface this collider presents on contact.
    pub fn material(&self) -> SurfaceMaterial {
        SurfaceMaterial {
            friction: self.friction,
            friction_rule: combine_rule(self.friction_rule),
            restitution: self.restitution,
            restitution_rule: combine_rule(self.restitution_rule),
        }
        .sanitised()
    }

    /// How this collider participates: what it notices and what it
    /// reports.
    pub fn interaction(&self) -> ColliderInteraction {
        ColliderInteraction {
            collision_groups: InteractionMask {
                memberships: self.collision_memberships,
                filter: self.collision_filter,
            },
            solver_groups: InteractionMask {
                memberships: self.solver_memberships,
                filter: self.solver_filter,
            },
            sensor: self.sensor,
            collision_events: self.collision_events,
            contact_force_events: self.contact_force_events,
            contact_force_threshold: self.contact_force_threshold.max(0.0),
        }
    }

    /// The authored identity of this collider's geometry.
    ///
    /// POD and comparable, so the sync pass can decide "the shape
    /// changed" without resolving a mesh or hashing a point cloud.
    /// `meshes` supplies the epoch that makes a mesh *arriving* count as
    /// a change; `None` reads as "nothing has answered yet".
    pub fn shape_spec(&self, meshes: Option<&ColliderMeshCache>) -> ShapeSpec {
        ShapeSpec {
            shape: self.shape,
            radius: self.radius,
            half_extents: self.half_extents,
            half_height: self.half_height,
            border_radius: self.border_radius,
            normal: self.normal,
            point_a: self.point_a,
            point_b: self.point_b,
            point_c: self.point_c,
            voxel_size: self.voxel_size,
            voxel_solid: self.voxel_solid,
            mesh: self.mesh,
            mesh_epoch: match (self.mesh, meshes) {
                (Some(guid), Some(cache)) => cache.epoch(guid),
                _ => 0,
            },
        }
    }

    /// The geometry the backend takes, or `None` while a mesh-derived
    /// shape is still waiting for its mesh.
    pub fn collision_shape(&self, meshes: Option<&ColliderMeshCache>) -> Option<CollisionShape> {
        self.shape_spec(meshes).resolve(meshes)
    }
}
