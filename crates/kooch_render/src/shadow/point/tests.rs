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
    assert!(select_point_casters(&[behind], &f, 4).is_empty());
}

#[test]
fn a_lamp_off_screen_still_casts_onto_it() {
    // Its centre is outside the frustum and its reach is not. A point
    // test would drop it and the shadow it throws across the visible
    // floor would vanish as the camera turned.
    let f = frustum();
    let edge = source(Vec3::new(0.0, 60.0, -30.0), 50.0);
    assert_eq!(select_point_casters(&[edge], &f, 4).len(), 1);
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

    let chosen = select_point_casters(&ranked, &f, 4);
    assert_eq!(chosen.len(), 1, "only the visible lamp should get a cube");
    assert_eq!(chosen[0].position, visible.position);
}

#[test]
fn the_limit_still_applies_to_visible_lamps() {
    let f = frustum();
    let ranked: Vec<_> = (0..6)
        .map(|i| source(Vec3::new(0.0, 0.0, -10.0 - i as f32), 5.0))
        .collect();
    assert_eq!(select_point_casters(&ranked, &f, 4).len(), 4);
}
