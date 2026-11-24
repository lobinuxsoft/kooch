//! What an author puts on an entity to make it a planet.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::FieldRange;

/// A planet: a sphere of `radius` that its generator moves by up to `relief` above and `depth`
/// below, drawn from voxel blocks of the pool every planet shares.
///
/// Its centre is the entity's position; it turns with the entity, and its blocks never regenerate
/// for that. Lengths are in the entity's local units, so a scale of 1 keeps them in metres.
#[derive(Debug, Clone, PartialEq, Reflect)]
#[reflect(category = "World")]
pub struct Planet {
    /// Sea level: where an unwired generator puts the ground.
    #[reflect(range = LENGTH_RANGE)]
    pub radius: f32,
    /// The highest the generator may raise the ground above `radius`. Higher costs blocks.
    #[reflect(range = LENGTH_RANGE)]
    pub relief: f32,
    /// The deepest a cave may reach below `radius`; under it the core is solid.
    #[reflect(range = LENGTH_RANGE)]
    pub depth: f32,
    /// The edge of a voxel next to the camera: the finest detail the ground can have.
    #[reflect(range = VOXEL_RANGE)]
    pub voxel: f32,
    /// How far from the camera the finest voxels reach; each coarser level reaches twice as far.
    /// Higher keeps detail further out and costs blocks.
    #[reflect(range = LENGTH_RANGE)]
    pub lod_distance: f32,
    /// A `planet` shader — usually a planet graph — that gives the ground's signed distance.
    /// None is the bare sphere.
    #[reflect(asset = "kooch_render::material::shader::Shader")]
    pub generator: Option<Guid>,
    /// What the ground is drawn with.
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
}

const LENGTH_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0e7,
    step: 0.5,
};

const VOXEL_RANGE: FieldRange = FieldRange {
    min: 0.01,
    max: 100.0,
    step: 0.05,
};

impl Default for Planet {
    fn default() -> Self {
        Self {
            radius: 50.0,
            relief: 10.0,
            depth: 10.0,
            voxel: 0.5,
            lod_distance: 32.0,
            generator: None,
            material: None,
        }
    }
}

impl Component for Planet {}
