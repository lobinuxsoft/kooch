//! [`Collider`]: the shape an entity presents and how its surface behaves. `shape` is a
//! discriminant ([`PhysicsBody`](super::PhysicsBody)); choices live in [`shapes`], surface and
//! filtering in [`groups`].

mod groups;
mod shapes;
mod spec;

pub use groups::{
    COMBINE_AVERAGE, COMBINE_CHOICES, COMBINE_CLAMPED_SUM, COMBINE_MAX, COMBINE_MIN,
    COMBINE_MULTIPLY, GROUP_BITS,
};
use shapes::is_own_mesh;
pub use shapes::{
    BORDER_RADIUS_WHEN, ENDPOINTS_WHEN, HALF_EXTENTS_WHEN, HALF_HEIGHT_WHEN, MESH_DERIVED,
    MESH_WHEN, NORMAL_WHEN, POINT_C_WHEN, RADIUS_WHEN, SHAPE_CAPSULE, SHAPE_CHOICES, SHAPE_CONE,
    SHAPE_CONVEX_DECOMPOSITION, SHAPE_CONVEX_HULL, SHAPE_CUBOID, SHAPE_CYLINDER, SHAPE_HALF_SPACE,
    SHAPE_OWN_MESH, SHAPE_POLYLINE, SHAPE_ROUND_CYLINDER, SHAPE_SEGMENT, SHAPE_SPHERE,
    SHAPE_TRIANGLE, SHAPE_TRIMESH, SHAPE_VOXELIZED_MESH, SHAPE_VOXELS, VOXEL_SIZE_WHEN,
    VOXEL_SOLID_WHEN, is_mesh_derived,
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

/// Collision geometry plus material and filtering (#137); [`CollisionShape`] stays pure geometry.
/// Only the selected shape's fields are read and shown; the rest keep their values. Default: a unit
/// sphere.
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
    /// Rim fillet of the rounded cylinder — a sharp rim catches on box edges.
    #[reflect(shown_when = BORDER_RADIUS_WHEN)]
    pub border_radius: f32,
    /// Direction the half-space's solid side faces away from; normalised at build, up when zero.
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
    /// The mesh a hull, decomposition or trimesh is built from, resolved outside physics
    /// ([`ColliderMeshCache`]) — often a simplified stand-in, not the drawn mesh.
    #[reflect(shown_when = MESH_WHEN)]
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// Voxel edge length: halving it is 8× the cells, and voxels only beat a trimesh while coarse.
    #[reflect(shown_when = VOXEL_SIZE_WHEN)]
    pub voxel_size: f32,
    /// Fill the interior, not only the shell — a body inside a shell passes straight out.
    #[reflect(shown_when = VOXEL_SOLID_WHEN)]
    pub voxel_solid: bool,
    /// Resistance to sliding. 0 is frictionless; 1 is about rubber on dry
    /// tarmac. Above 1 is legal and useful for gameplay.
    pub friction: f32,
    /// How friction combines, a `COMBINE_*` constant. **The pushier claim wins**: rapier takes the
    /// higher discriminant.
    #[reflect(choices = COMBINE_CHOICES)]
    pub friction_rule: u32,
    /// Bounce. 0 absorbs the impact; 1 returns it, so a ball comes back to
    /// roughly the height it fell from.
    pub restitution: f32,
    /// How this collider's bounce combines with the other one's. Same
    /// max-wins resolution as `friction_rule`.
    #[reflect(choices = COMBINE_CHOICES)]
    pub restitution_rule: u32,
    /// Report overlap, never push — checkpoints, damage zones. No manifold, so no contact data.
    pub sensor: bool,
    /// Event on touch start and stop; off, since rapier's events are opt-in and a scene pays for
    /// what it hears.
    pub collision_events: bool,
    /// Event above `contact_force_threshold` — "hit hard enough" without per-frame contact
    /// inspection.
    pub contact_force_events: bool,
    /// The force, in newtons, above which a contact is worth reporting.
    #[reflect(shown_when = CONTACT_FORCE_WHEN)]
    pub contact_force_threshold: f32,
    /// Groups this collider belongs to; a pair needs **both** sides' memberships to meet the
    /// other's filter.
    #[reflect(bits = GROUP_BITS)]
    pub collision_memberships: u32,
    /// Which groups this collider will collide with.
    #[reflect(bits = GROUP_BITS)]
    pub collision_filter: u32,
    /// Groups it is solved against, of those it collides with — detect a wall without being stopped
    /// by it.
    #[reflect(bits = GROUP_BITS)]
    pub solver_memberships: u32,
    /// Which groups this collider will be pushed by.
    #[reflect(bits = GROUP_BITS)]
    pub solver_filter: u32,
    /// Shape centre in local space, moving geometry without the body — a feet-pivoted character's
    /// capsule sits half a body up.
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

    /// This collider's POD spec on `entity`, comparable without resolving a mesh; `meshes` supplies
    /// the epoch that makes an arriving mesh a change. Takes the entity, since generated meshes are
    /// addressed by it ([`MeshKey`](crate::backend::MeshKey)).
    pub fn shape_spec(
        &self,
        entity: kooch_ecs::entity::Entity,
        meshes: Option<&ColliderMeshCache>,
    ) -> ShapeSpec {
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
            mesh: self.mesh_key(entity),
            mesh_epoch: match (self.mesh_key(entity), meshes) {
                (Some(key), Some(cache)) => cache.epoch(key),
                _ => 0,
            },
        }
    }

    /// How the geometry is addressed; own-mesh shapes ignore `mesh`, which would chase a
    /// nonexistent file.
    fn mesh_key(&self, entity: kooch_ecs::entity::Entity) -> Option<crate::backend::MeshKey> {
        match is_own_mesh(self.shape) {
            true => Some(crate::backend::MeshKey::Owned(entity)),
            false => self.mesh.map(crate::backend::MeshKey::Asset),
        }
    }

    /// The geometry the backend takes, or `None` while a mesh-derived
    /// shape is still waiting for its mesh.
    pub fn collision_shape(
        &self,
        entity: kooch_ecs::entity::Entity,
        meshes: Option<&ColliderMeshCache>,
    ) -> Option<CollisionShape> {
        self.shape_spec(entity, meshes).resolve(meshes)
    }
}
