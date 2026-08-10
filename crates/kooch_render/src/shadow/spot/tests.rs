use super::*;

use glam::Vec4Swizzles;
use kooch_ecs::entity::Entity;

fn source(position: Vec3, direction: Vec3, outer_angle: f32) -> SpotShadowSource {
    SpotShadowSource {
        entity: Entity::new(0, 0),
        position,
        direction,
        outer_angle,
        range: 20.0,
    }
}

/// Projecting a point through the record has to put the light's own
/// position at the centre of the map. Getting the view's handedness or
/// its up vector wrong still produces a plausible matrix — and a shadow
/// that is somewhere else in the room.
#[test]
fn a_point_down_the_cone_lands_in_the_middle() {
    let src = source(Vec3::new(3.0, 8.0, -2.0), Vec3::NEG_Y, 0.6);
    let record = spot_shadow(&src, 4, 2048);
    let view_proj = Mat4::from_cols_array_2d(&record.view_proj);

    let ahead = src.position + src.direction * 5.0;
    let clip = view_proj * ahead.extend(1.0);
    let ndc = clip.xyz() / clip.w;

    assert!(ndc.x.abs() < 1e-4, "x off centre: {}", ndc.x);
    assert!(ndc.y.abs() < 1e-4, "y off centre: {}", ndc.y);
}

/// Reversed-Z with an infinite far plane: near maps to 1 and distance
/// approaches 0. A cascade's orthographic slice is linear either way, so
/// this is the one place in the shadow code where getting the convention
/// backwards is silent — everything renders, inverted.
#[test]
fn nearer_the_light_is_greater_depth() {
    let src = source(Vec3::ZERO, Vec3::NEG_Z, 0.5);
    let view_proj = Mat4::from_cols_array_2d(&spot_shadow(&src, 4, 1024).view_proj);

    let depth_at = |metres: f32| {
        let clip = view_proj * (Vec3::NEG_Z * metres).extend(1.0);
        clip.z / clip.w
    };
    assert!(
        depth_at(1.0) > depth_at(10.0),
        "reversed-Z means closer is greater: {} vs {}",
        depth_at(1.0),
        depth_at(10.0),
    );
}

/// A light pointing straight down is the most ordinary way to author a
/// spot and the one that breaks a fixed world-up basis.
#[test]
fn a_light_pointing_straight_down_has_a_basis() {
    for direction in [Vec3::NEG_Y, Vec3::Y] {
        let record = spot_shadow(&source(Vec3::new(0.0, 5.0, 0.0), direction, 0.7), 4, 1024);
        let view_proj = Mat4::from_cols_array_2d(&record.view_proj);
        assert!(
            view_proj.to_cols_array().iter().all(|v| v.is_finite()),
            "degenerate basis for {direction:?}",
        );
    }
}

/// The cone edge is where the light stops. Fitting the frustum to the
/// half-angle instead of the full one clips the lit pool into a square.
#[test]
fn the_frustum_covers_the_whole_cone() {
    let half_angle: f32 = 0.6;
    let src = source(Vec3::ZERO, Vec3::NEG_Z, half_angle);
    let view_proj = Mat4::from_cols_array_2d(&spot_shadow(&src, 4, 1024).view_proj);

    // A point on the cone's rim, five metres out.
    let distance = 5.0;
    let rim = Vec3::new(distance * half_angle.tan(), 0.0, -distance);
    let clip = view_proj * rim.extend(1.0);
    let ndc_x = clip.x / clip.w;

    assert!(
        ndc_x.abs() <= 1.0 + 1e-3,
        "the cone's rim projects outside the map at ndc.x = {ndc_x}",
    );
    assert!(
        ndc_x.abs() > 0.9,
        "the rim should reach the map's edge, got {ndc_x} — the frustum is \
         far wider than the cone and most of the map is wasted",
    );
}

/// An absurd cone must not produce an infinite frustum. `tan` runs away
/// approaching 90° and the matrix fills with infinities, which then
/// spread into every depth the pass writes.
#[test]
fn an_absurd_cone_is_clamped_rather_than_infinite() {
    let record = spot_shadow(&source(Vec3::ZERO, Vec3::NEG_Z, 3.0), 4, 1024);
    assert!(
        record.view_proj.iter().flatten().all(|v| v.is_finite()),
        "a 172° cone produced a non-finite projection",
    );
    assert!(record.texel_world_size.is_finite() && record.texel_world_size > 0.0);
}

/// The layer is what tells the shading model which map to read, and it
/// is the one field nothing else can derive.
#[test]
fn the_record_carries_the_layer_it_was_given() {
    assert_eq!(
        spot_shadow(&source(Vec3::ZERO, Vec3::NEG_Z, 0.5), 6, 1024).layer,
        6
    );
}

/// 🔴 The regression the first smoke found. `texel_world_size` on a spot
/// is an angle per texel, NOT metres: the shader multiplies it by each
/// fragment's own distance to the light.
///
/// Baking `range` in instead biases every fragment as though it sat at
/// the far end of the cone. For a 100 m spot over objects five metres
/// away that was a 17 cm offset along the normal — the shadow shrank and
/// lifted off the object, and nothing about that reads as a unit error.
#[test]
fn the_texel_size_does_not_depend_on_range() {
    let near = SpotShadowSource {
        range: 10.0,
        ..source(Vec3::ZERO, Vec3::NEG_Z, 0.5)
    };
    let far = SpotShadowSource {
        range: 1000.0,
        ..near
    };
    assert_eq!(
        spot_shadow(&near, 4, 2048).texel_world_size,
        spot_shadow(&far, 4, 2048).texel_world_size,
        "range must not reach the bias — it is what made the first smoke \
         peter-pan",
    );
}

/// And it does scale the way a texel does: a wider cone spreads the same
/// texels over more of the world.
#[test]
fn a_wider_cone_has_bigger_texels() {
    let narrow = spot_shadow(&source(Vec3::ZERO, Vec3::NEG_Z, 0.2), 4, 2048).texel_world_size;
    let wide = spot_shadow(&source(Vec3::ZERO, Vec3::NEG_Z, 1.0), 4, 2048).texel_world_size;
    assert!(wide > narrow, "{wide} should exceed {narrow}");
}

/// 🔴 The smoke's second finding: rotating the light moved the lit pool
/// and did NOT move the shadow.
///
/// Two lights at the same place pointing different ways must not share a
/// shadow matrix. If this passes, the fitting is right and the fault is
/// downstream — the cull, the raster or the sampling. If it fails, it is
/// here. Those are different files and looking at the frame cannot tell
/// them apart.
#[test]
fn rotating_the_light_moves_its_shadow() {
    let position = Vec3::new(0.0, 9.0, 0.0);
    let down = spot_shadow(&source(position, Vec3::NEG_Y, 0.78), 4, 2048);
    let tilted = spot_shadow(
        &source(position, Vec3::new(0.0, -1.0, -1.0).normalize(), 0.78),
        4,
        2048,
    );

    assert_ne!(
        down.view_proj, tilted.view_proj,
        "the same matrix for two directions — the shadow cannot follow the light",
    );

    // And concretely: a point on the floor ahead of the tilted light is
    // inside its map and outside the one pointing straight down.
    let ahead = Vec3::new(0.0, 0.0, -9.0);
    let inside = |m: &[[f32; 4]; 4]| {
        let clip = Mat4::from_cols_array_2d(m) * ahead.extend(1.0);
        let ndc = clip.xyz() / clip.w;
        ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && clip.w > 0.0
    };
    assert!(
        inside(&tilted.view_proj),
        "a point the tilted light shines at falls outside its own shadow map",
    );
    assert!(
        !inside(&down.view_proj),
        "a point nine metres to the side is inside the map of a light \
         pointing straight down — the view is ignoring the direction",
    );
}
