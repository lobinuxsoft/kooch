use super::*;
use kooch_ecs::entity::Entity;

/// A world point, and the face of a light at `eye` that sees it — found
/// the way the hardware finds it, by which projection puts it on screen.
fn face_seeing(eye: Vec3, world: Vec3) -> Option<(usize, f32)> {
    (0..CUBE_FACES).find_map(|face| {
        let clip = face_view_proj(eye, face, POINT_SHADOW_NEAR_Z) * world.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        (ndc.x.abs() <= 1.0 + 1e-4 && ndc.y.abs() <= 1.0 + 1e-4).then_some((face, ndc.z))
    })
}

fn source(position: Vec3, range: f32) -> PointShadowSource {
    PointShadowSource {
        entity: Entity::new(0, 0),
        position,
        range,
        intensity: 1.0,
        // Equal for every lamp a test builds this way, so the selection
        // sort leaves them in the order the test wrote them and these
        // tests keep asking what they asked before hysteresis existed.
        importance: 1.0,
    }
}

/// A lamp in front of the camera, identified and ranked.
fn lamp(id: u32, importance: f32) -> PointShadowSource {
    PointShadowSource {
        entity: Entity::new(id, 0),
        position: Vec3::new(0.0, 0.0, -20.0),
        range: 5.0,
        intensity: 1.0,
        importance,
    }
}

#[test]
fn the_six_faces_cover_every_direction() {
    let eye = Vec3::new(1.0, 2.0, -3.0);
    // Directions that are not axis-aligned, so none of them sits exactly
    // on a face boundary where either neighbour would be a fair answer.
    for (i, dir) in [
        Vec3::new(0.9, 0.2, 0.1),
        Vec3::new(-0.7, 0.3, -0.2),
        Vec3::new(0.1, 0.95, -0.3),
        Vec3::new(0.2, -0.8, 0.15),
        Vec3::new(-0.1, 0.25, 0.9),
        Vec3::new(0.3, -0.2, -0.85),
        Vec3::new(0.5, 0.5, 0.49),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            face_seeing(eye, eye + dir.normalize() * 4.0).is_some(),
            "direction {i} ({dir:?}) fell through all six faces",
        );
    }
}

/// 🔴 The test the whole record depends on.
///
/// `GpuPointShadow` carries a single `near` where Bevy carries four
/// projection terms, on the claim that with an infinite reverse-Z
/// projection the stored depth is exactly `near / major_axis_magnitude`.
/// If that is wrong, every comparison in the shader is off by a factor
/// that varies with distance — which reads as a bias that cannot be
/// tuned, not as a wrong formula.
#[test]
fn depth_is_near_over_the_major_axis() {
    let eye = Vec3::new(-2.0, 5.0, 1.5);
    for world in [
        eye + Vec3::new(3.0, 0.4, -0.7),
        eye + Vec3::new(-0.2, 8.0, 1.1),
        eye + Vec3::new(0.6, -1.3, -12.0),
        eye + Vec3::new(-4.5, -2.0, 0.9),
    ] {
        let (face, ndc_z) = face_seeing(eye, world).expect("some face sees it");
        let v = world - eye;
        let major = v.x.abs().max(v.y.abs()).max(v.z.abs());
        let expected = POINT_SHADOW_NEAR_Z / major;
        assert!(
            (ndc_z - expected).abs() < 1e-5,
            "face {face}: projection gave {ndc_z}, near/major says {expected}",
        );
    }
}

#[test]
fn the_texel_size_ignores_range() {
    // Two lights with the same face size and wildly different reach get
    // the same angular texel. #777 shipped the version that did not, and
    // it lifted every shadow off its object.
    let near = point_shadow(&source(Vec3::ZERO, 5.0), 512);
    let far = point_shadow(&source(Vec3::ZERO, 500.0), 512);
    assert_eq!(near.texel_world_size, far.texel_world_size);
}

#[test]
fn a_bigger_face_has_smaller_texels() {
    let small = point_shadow(&source(Vec3::ZERO, 10.0), 512);
    let big = point_shadow(&source(Vec3::ZERO, 10.0), 2048);
    assert!(big.texel_world_size < small.texel_world_size);
}

#[test]
fn a_degenerate_range_does_not_reach_the_near_plane() {
    // A range of zero is authorable, and a depth extent below the near
    // plane makes the penumbra estimate divide into nothing.
    let record = point_shadow(&source(Vec3::ZERO, 0.0), 512);
    assert!(record.depth_extent > POINT_SHADOW_NEAR_Z);
}

#[test]
fn every_face_is_distinct() {
    let eye = Vec3::ZERO;
    let mut seen: Vec<usize> = Vec::new();
    for (target, _) in FACE_DIRECTIONS {
        let (face, _) = face_seeing(eye, target * 3.0).expect("a face sees its own axis");
        seen.push(face);
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), CUBE_FACES, "two faces answered for one axis");
}

/// A camera at the origin looking down −Z, which is what
/// `extract_frustum_planes` is fed everywhere else.
fn frustum() -> [[f32; 4]; 6] {
    let camera = crate::view_camera::ViewCamera::looking_at(Vec3::ZERO, Vec3::NEG_Z);
    crate::meshlet::extract_frustum_planes(camera.view_proj(1.0))
}

#[test]
fn a_lamp_behind_the_camera_gets_no_cube() {
    let f = frustum();
    let behind = source(Vec3::new(0.0, 0.0, 50.0), 5.0);
    assert!(select_point_casters(&[behind], &f, 4, &[]).is_empty());
}

#[test]
fn a_lamp_off_screen_still_casts_onto_it() {
    // Its centre is outside the frustum and its reach is not. A point
    // test would drop it and the shadow it throws across the visible
    // floor would vanish as the camera turned.
    let f = frustum();
    let edge = source(Vec3::new(0.0, 60.0, -30.0), 50.0);
    assert_eq!(select_point_casters(&[edge], &f, 4, &[]).len(), 1);
}

/// 🔴 The ordering test: cull, then limit.
///
/// Limiting first would spend all four cubes on the nearest lights even
/// when they are behind the camera, and the visible lamp — the only one
/// whose shadow anybody can see — would get nothing.
#[test]
fn culling_happens_before_the_limit() {
    let f = frustum();
    let mut ranked: Vec<_> = (0..4)
        .map(|i| source(Vec3::new(0.0, 0.0, 10.0 + i as f32), 5.0))
        .collect();
    let visible = source(Vec3::new(0.0, 0.0, -20.0), 5.0);
    ranked.push(visible);

    let chosen = select_point_casters(&ranked, &f, 4, &[]);
    assert_eq!(chosen.len(), 1, "only the visible lamp should get a cube");
    assert_eq!(chosen[0].position, visible.position);
}

#[test]
fn the_limit_still_applies_to_visible_lamps() {
    let f = frustum();
    let ranked: Vec<_> = (0..6)
        .map(|i| source(Vec3::new(0.0, 0.0, -10.0 - i as f32), 5.0))
        .collect();
    assert_eq!(select_point_casters(&ranked, &f, 4, &[]).len(), 4);
}

/// 🔴 The failure this exists to stop: a hundred lamps on a grid, the
/// ranking continuous and the cut not, so a step in any direction swaps
/// which four hold cubes and the shadow blinks.
#[test]
fn a_holder_keeps_its_cube_against_a_marginal_rival() {
    let f = frustum();
    let holder = lamp(1, 1.0);
    let rival = lamp(2, 1.2);
    let chosen = select_point_casters(&[rival, holder], &f, 1, &[holder.entity]);
    assert_eq!(chosen.len(), 1);
    assert_eq!(
        chosen[0].entity, holder.entity,
        "a rival 20% ahead is inside the margin and must not take the cube",
    );
}

/// The control. Without it the test above would pass just as well with
/// the bonus set to infinity, which would freeze the cubes on whatever
/// four lamps the first frame happened to pick.
#[test]
fn a_clearly_better_rival_takes_the_cube() {
    let f = frustum();
    let holder = lamp(1, 1.0);
    let rival = lamp(2, 1.5);
    let chosen = select_point_casters(&[holder, rival], &f, 1, &[holder.entity]);
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].entity, rival.entity);
}

/// Holding is worth something; being out of view is worth nothing. A
/// lamp behind the camera keeps no cube, bonus or not — otherwise the
/// four faces it costs would be spent rasterising for a viewer who
/// turned around.
#[test]
fn a_holder_that_left_the_view_loses_its_cube() {
    let f = frustum();
    let gone = source(Vec3::new(0.0, 0.0, 50.0), 5.0);
    assert!(select_point_casters(&[gone], &f, 4, &[gone.entity]).is_empty());
}

/// 🔴 The stress scene in miniature: a hundred lamps on a two-metre
/// grid, all in front of the camera, all casting.
///
/// Written because a device capture showed `Device::create_bind_group`
/// at 53 calls a frame — the number a frame with NO cube draws reports —
/// while the scene had `cast_shadows: true` on all hundred lights.
#[test]
fn a_grid_of_lamps_fills_every_cube() {
    let f = frustum();
    let eye = Vec3::ZERO;
    let mut ranked: Vec<PointShadowSource> = (0..100u32)
        .map(|i| {
            // 10x10, two metres apart, centred ahead of the camera.
            let position = Vec3::new(
                (i % 10) as f32 * 2.0 - 9.0,
                1.0,
                -((i / 10) as f32 * 2.0) - 5.0,
            );
            PointShadowSource {
                entity: Entity::new(i, 0),
                position,
                range: 4.0,
                intensity: 1000.0,
                importance: kooch_lighting::point_shadow_importance(position, 4.0, 1000.0, eye),
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.importance.total_cmp(&a.importance));

    let chosen = select_point_casters(&ranked, &f, kooch_lighting::MAX_POINT_SHADOWS, &[]);
    assert_eq!(
        chosen.len(),
        kooch_lighting::MAX_POINT_SHADOWS,
        "every cube should be spoken for; got {} of {}",
        chosen.len(),
        kooch_lighting::MAX_POINT_SHADOWS,
    );
}

fn instance(center: Vec3, radius: f32, hash: u64) -> InstanceBounds {
    InstanceBounds {
        center,
        radius,
        hash,
    }
}

/// 🔴 The defect #847 exists for: a crate moving on the far side of the
/// level used to redraw every cube in the frame.
#[test]
fn a_distant_instance_does_not_touch_the_key() {
    let lamp = Vec3::ZERO;
    let near = instance(Vec3::new(1.0, 0.0, 0.0), 0.5, 111);
    let far = instance(Vec3::new(80.0, 0.0, 0.0), 0.5, 222);
    let moved = instance(Vec3::new(80.0, 0.0, 0.0), 0.5, 999);

    let before = light_scene_hash(&[near, far], lamp, 4.0);
    let after = light_scene_hash(&[near, moved], lamp, 4.0);
    assert_eq!(
        before, after,
        "a lamp cannot see 80 m away with a 4 m range"
    );
}

/// The control. Without it the test above passes just as well with a
/// hash that ignores every instance — which would freeze every cube in
/// the scene, the silent failure this cache is written to avoid.
#[test]
fn an_instance_in_range_does_change_it() {
    let lamp = Vec3::ZERO;
    let near = instance(Vec3::new(1.0, 0.0, 0.0), 0.5, 111);
    let moved = instance(Vec3::new(1.0, 0.0, 0.0), 0.5, 112);
    assert_ne!(
        light_scene_hash(&[near], lamp, 4.0),
        light_scene_hash(&[moved], lamp, 4.0),
    );
}

/// The floor case, and the reason the test is sphere against sphere
/// rather than point against sphere: a 20 m floor slab is centred far
/// from a lamp standing on it and is the very surface its shadow lands
/// on. A point test would drop it and freeze the shadow.
#[test]
fn a_big_slab_counts_from_its_edge() {
    let lamp = Vec3::new(0.0, 1.0, 0.0);
    let floor = instance(Vec3::new(0.0, -0.25, 0.0), 14.0, 7);
    let moved = instance(Vec3::new(0.0, -0.25, 0.0), 14.0, 8);
    assert_ne!(
        light_scene_hash(&[floor], lamp, 4.0),
        light_scene_hash(&[moved], lamp, 4.0),
        "the slab's own radius has to be added to the light's range",
    );
}

/// Order is part of the digest: two instances swapping places is a
/// change, and the array is rebuilt in walk order every frame.
#[test]
fn order_is_part_of_the_digest() {
    let lamp = Vec3::ZERO;
    let a = instance(Vec3::new(1.0, 0.0, 0.0), 0.5, 1);
    let b = instance(Vec3::new(-1.0, 0.0, 0.0), 0.5, 2);
    assert_ne!(
        light_scene_hash(&[a, b], lamp, 4.0),
        light_scene_hash(&[b, a], lamp, 4.0),
    );
}
