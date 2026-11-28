//! What an author puts on an entity to give it voxel ground.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::FieldRange;

use crate::stamp::Stamp;

/// Voxel ground: a base shape that its generator moves by up to `relief` above and `depth` below,
/// drawn from voxel blocks of the pool every terrain shares (#2035). The generator's base node
/// picks the shape: a sphere of `radius`, or a plane `extent` either side (#1950).
///
/// Its centre is the entity's position; it turns with the entity, and its blocks never regenerate
/// for that. Lengths are in the entity's local units, so a scale of 1 keeps them in metres.
#[derive(Debug, Clone, PartialEq, Reflect)]
// Was `kooch_planet::component::Planet` until #2037 and in `kooch_planet` until #2040; saved
// scenes still name it so.
#[reflect(
    category = "World",
    alias = "kooch_planet::component::Planet, kooch_planet::component::Terrain"
)]
pub struct Terrain {
    /// Sea level of a sphere base: where an unwired generator puts the ground.
    #[reflect(range = LENGTH_RANGE)]
    pub radius: f32,
    /// Half the side of a plane base (its generator's Plane Base node): the ground spans this far
    /// either side of the centre on X and Z. Unused by a sphere.
    #[reflect(range = LENGTH_RANGE)]
    pub extent: f32,
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
    /// Shapes applied over the generator in list order, first to last (#1954). Moving one
    /// regenerates only the blocks it reaches.
    pub stamps: Vec<Stamp>,
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

impl Default for Terrain {
    fn default() -> Self {
        Self {
            radius: 50.0,
            extent: 128.0,
            relief: 10.0,
            depth: 10.0,
            voxel: 0.5,
            lod_distance: 32.0,
            generator: None,
            material: None,
            stamps: Vec::new(),
        }
    }
}

impl Component for Terrain {}
