//! Fitting six shadow faces to one point light (#778).
//!
//! A spot light *is* a frustum, so its shadow view is the cone itself. A
//! point light is not a frustum at all — it lights every direction — so
//! it gets six 90° frusta that tile the sphere, and the shading model
//! picks one by the direction to the fragment rather than by a matrix.
//!
//! That is what makes the record so small: there is no `view_proj` to
//! send. See [`GpuPointShadow`].

use glam::{Mat4, Vec3};

use kooch_ecs::entity::Entity;
use kooch_lighting::{GpuPointShadow, PointShadowSource};

/// Near plane for every cube face, in metres. Bevy's
/// `PointLight::DEFAULT_SHADOW_MAP_NEAR_Z`, and the same value the spots
/// use — a light's near plane is about how close geometry may get to the
/// bulb, which does not depend on the shape of the light.
pub const POINT_SHADOW_NEAR_Z: f32 = 0.1;

/// How many faces a cube map has. Named because `6` appears in the
/// layer arithmetic, the memory budget and the draw count, and only one
/// of those three is obviously the same six.
pub const CUBE_FACES: usize = 6;

/// The six directions, in the layer order a cube-array texture expects:
/// +X, −X, +Y, −Y, +Z, −Z.
///
/// 🔴 Ported verbatim from Bevy's `CUBE_MAP_FACES`
/// (`bevy_camera/src/primitives.rs:347`), including the two that look
/// wrong: face 4 is labelled +Z and targets `NEG_Z`, face 5 is labelled
/// −Z and targets `Z`. **Cube maps are left-handed and the rest of the
/// engine is not**, so the Z faces are swapped here and the sampling
/// direction is mirrored on Z at the other end (`flip_z` in the shader).
/// Fixing either half alone puts the shadow of everything in front of a
/// lamp behind it.
///
/// The `up` vectors are Bevy's too, not the D3D reference table's — they
/// are the ones that agree with the mirror above.
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
    /// Which light this is. Carried so the cache can tell "the same lamp
    /// has not moved" from "this slot now belongs to a different lamp",
    /// which look identical if only the position is compared and the two
    /// lamps happen to stand in the same place.
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
///
/// 🔴 This is the whole of #847. The cube cache used to key on a hash of
/// **every instance in the frame**, so a crate sliding in a sealed room
/// on the far side of the level redrew all four cubes — 24 faces,
/// measured at +2.0 ms, in any scene where anything moves at all.
///
/// Order matters and is the array's: two instances swapping places would
/// otherwise digest the same, and the array is rebuilt in ECS walk order
/// every frame — the same order the GPU upload uses.
///
/// ⚠️ The test is sphere against sphere, and the instance's sphere is
/// deliberately generous (see [`MeshBounds`](crate::meshlet::MeshBounds)).
/// A false positive costs one redrawn cube; a false negative is a shadow
/// frozen in place, which is silent and gets blamed on everything else
/// first.
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
///
/// 🔴 Deliberately coarse. `scene` is a hash of **every** instance in
/// the frame, so a crate moving on the far side of the level invalidates
/// a lamp that cannot see it. That is the conservative direction: a cube
/// redrawn for no reason costs a frame's work, and a cube NOT redrawn
/// when it should have been is a shadow frozen in place — silent, and
/// blamed on everything else first.
///
/// Making it finer means asking which instances a light's range reaches,
/// which is the clustering structure (#780) and not this issue.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CubeKey {
    entity: Entity,
    eye: [u32; 3],
    scene: u64,
}

/// Which casting point lights get one of the [`MAX_POINT_SHADOWS`] cubes
/// this frame.
///
/// [`MAX_POINT_SHADOWS`]: kooch_lighting::MAX_POINT_SHADOWS
///
/// `sources` arrives ranked by importance. Two things happen here:
/// whoever already held a cube is favoured (see below), and then the
/// limit is taken.
///
/// # 🔴 There is no camera frustum in here, and there used to be
///
/// This culled lamps whose `range` sphere fell outside the camera's
/// frustum, before applying the limit, because six faces is the most
/// expensive shadow in the engine and `cast_shadows` defaults to true —
/// a corridor of lamps behind the viewer would otherwise rasterise
/// twenty-four faces nobody can see.
///
/// The premise was wrong. **A cube map is drawn from the light**, so
/// what it contains cannot depend on where anyone is standing, and the
/// cube array, the cache and `holders` all belong to the render stage
/// while a frustum belongs to one view. The editor renders two views
/// through one stage — the View panel and the Game panel — so a lamp
/// outside the *gameplay* camera's frustum lost its cube for both, and
/// the panel that was looking straight at it drew no shadow. Whichever
/// view rendered last decided, every frame.
///
/// `two_views_on_one_stage` in `tests/point_shadow_dump.rs` is that
/// picture, and `two_views_where_game_also_sees_the_lamp` is the control
/// that pins it on the frustum and not on the shared cache.
///
/// The optimisation is worth having back, but it has to be asked of the
/// frame rather than of a view: the union of every active view's
/// frustum, or one selection computed once and reused. Neither is a
/// filter that lives in this function.
///
/// # 🔴 Why holding a cube is worth something
///
/// The ranking is continuous and the cut is not: two lights a hair apart
/// in importance are on opposite sides of a cliff, and the tiniest camera
/// movement swaps them. In `many_lights.scene` — a hundred lamps on a
/// two-metre grid — that is a shadow that **appears and disappears as the
/// viewer walks**, which reads as a broken shadow rather than as a budget
/// being enforced.
///
/// So a light that had a cube last frame keeps it unless a rival beats it
/// by [`CUBE_STICKINESS`]. A margin and not a lock: a genuinely more
/// important light still takes the cube, it just has to be clearly more
/// important rather than a rounding difference away.
///
/// `holders` is last frame's result. Empty on the first frame, which
/// makes this a plain ranked take — the right behaviour when there is no
/// history to preserve.
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
///
/// 25 % is chosen against the failure it exists to stop rather than
/// tuned: importance goes with the square of the angular radius, so a
/// quarter is about the difference a **12 % change in distance** makes.
/// Below that the swap is the camera breathing; above it, the viewer has
/// genuinely moved toward a different lamp.
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
///
/// The same infinite reverse-Z projection every other view in the engine
/// uses (ADR 0002), which is what lets the shader reconstruct the stored
/// depth as `near / major_axis_magnitude` instead of carrying the four
/// projection terms Bevy sends per light.
///
/// A cube face is 90° by construction — six of them tile the sphere
/// exactly — so the field of view is not a parameter and neither is the
/// aspect ratio.
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
///
/// A face spans 90°, so Bevy's `2 * tan(half_fov) / size` has
/// `tan(45°) = 1` and collapses to `2 / size`. Like the spot's, it is an
/// angle per texel and never involves `range` — the shader multiplies it
/// by the fragment's own distance. Baking the range in is what produced
/// #777's peter-panning, and a cube face would produce it six times.
///
/// The `SQRT_2` is Bevy's, for the worst-case diagonal offset.
fn face_texel_size(size: u32) -> f32 {
    2.0 / size.max(1) as f32 * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests;
