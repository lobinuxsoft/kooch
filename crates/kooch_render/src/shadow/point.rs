//! Fitting six shadow faces to one point light (#778).

use glam::{Mat4, Vec3};

use kooch_ecs::entity::Entity;
use kooch_lighting::{GpuPointShadow, PointShadowSource};

/// Near plane for every cube face, in metres. Bevy's `PointLight::DEFAULT_SHADOW_MAP_NEAR_Z`, and
/// the same value the spots use — a light's near plane is about how close geometry may get to the
/// bulb, which does not depend on the shape of the light.
pub const POINT_SHADOW_NEAR_Z: f32 = 0.1;

/// How many faces a cube map has. Named because `6` appears in the
/// layer arithmetic, the memory budget and the draw count, and only one
/// of those three is obviously the same six.
pub const CUBE_FACES: usize = 6;

/// The six directions, in the layer order a cube-array texture expects: +X, −X, +Y, −Y, +Z, −Z.
pub const FACE_DIRECTIONS: [(Vec3, Vec3); CUBE_FACES] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

/// One point light's shadow, as the pass needs it: where the light is,
/// and the six matrices its faces draw with.
#[derive(Copy, Clone, Debug)]
pub struct PointShadowDraw {
    /// Which light this is. Carried so the cache can tell "the same lamp has not moved" from "this
    /// slot now belongs to a different lamp", which look identical if only the position is compared
    /// and the two lamps happen to stand in the same place.
    pub entity: Entity,
    /// The light's position — a real eye, six times over.
    pub eye: Vec3,
    /// Clip-from-world per face, in cube-array layer order.
    pub faces: [Mat4; CUBE_FACES],
    /// How far this light reaches, so the cache can ask which instances
    /// are inside it (#847).
    pub range: f32,
}

impl PointShadowDraw {
    pub fn new(source: &PointShadowSource) -> Self {
        let position = source.position;
        Self {
            entity: source.entity,
            eye: position,
            faces: std::array::from_fn(|face| face_view_proj(position, face, POINT_SHADOW_NEAR_Z)),
            range: source.range,
        }
    }

    /// What has to be equal for last frame's six faces to still be true.
    pub fn key(&self, scene: u64) -> CubeKey {
        CubeKey {
            entity: self.entity,
            // Bit patterns, not floats: this is an identity test and it
            // needs `Eq`. A lamp that moves and moves back is genuinely
            // unchanged.
            eye: [
                self.eye.x.to_bits(),
                self.eye.y.to_bits(),
                self.eye.z.to_bits(),
            ],
            scene,
        }
    }
}

/// One instance as the cube cache sees it: where it is, how far it
/// reaches, and a digest of everything about it that could move a
/// shadow (#847).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct InstanceBounds {
    pub center: Vec3,
    pub radius: f32,
    pub hash: u64,
}

/// A digest of the instances a light's own range can reach.
pub fn light_scene_hash(instances: &[InstanceBounds], position: Vec3, range: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for instance in instances {
        let reach = range + instance.radius;
        if instance.center.distance_squared(position) <= reach * reach {
            instance.hash.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Everything a cached cube depends on.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CubeKey {
    entity: Entity,
    eye: [u32; 3],
    scene: u64,
}

/// Which casting point lights get one of the
/// [`MAX_POINT_SHADOWS`](kooch_lighting::MAX_POINT_SHADOWS) cubes this frame.
pub fn select_point_casters(
    sources: &[PointShadowSource],
    limit: usize,
    holders: &[Entity],
) -> Vec<PointShadowSource> {
    let mut visible: Vec<PointShadowSource> = sources.to_vec();
    // Stable, so lights the bonus leaves tied keep the order importance
    // gave them rather than swapping on an implementation detail.
    visible.sort_by(|a, b| {
        let a_score = a.importance * stickiness(holders, a.entity);
        let b_score = b.importance * stickiness(holders, b.entity);
        b_score.total_cmp(&a_score)
    });
    visible.truncate(limit);
    visible
}

/// What a light's importance is multiplied by while it holds a cube.
pub const CUBE_STICKINESS: f32 = 1.25;

fn stickiness(holders: &[Entity], entity: Entity) -> f32 {
    if holders.contains(&entity) {
        CUBE_STICKINESS
    } else {
        1.0
    }
}

/// The record the shading model reads for one point light.
///
/// `size` is the side of one face in texels.
pub fn point_shadow(source: &PointShadowSource, size: u32) -> GpuPointShadow {
    GpuPointShadow {
        near: POINT_SHADOW_NEAR_Z,
        texel_world_size: face_texel_size(size),
        depth_extent: source.range.max(POINT_SHADOW_NEAR_Z * 2.0),
        _pad0: 0.0,
    }
}

/// Clip-from-world for one face of one light.
pub fn face_view_proj(position: Vec3, face: usize, near: f32) -> Mat4 {
    let (target, up) = FACE_DIRECTIONS[face.min(CUBE_FACES - 1)];
    let projection = crate::projection::perspective_infinite_rh_reverse_z(
        std::f32::consts::FRAC_PI_2,
        1.0,
        near.max(1e-4),
    );
    projection * Mat4::look_to_rh(position, target, up)
}

/// Shadow-texel size **per metre of distance from the light**.
fn face_texel_size(size: u32) -> f32 {
    2.0 / size.max(1) as f32 * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests;
