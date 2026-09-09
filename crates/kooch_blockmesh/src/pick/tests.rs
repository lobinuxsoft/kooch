use glam::{Mat4, Vec2, Vec3};

use super::{Screen, edge_at, face_at, vertex_at};
use crate::{Adjacency, BlockMesh};

/// A unit cube centred on the origin: faces at ±0.5 on each axis.
fn cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

/// The face whose outward normal points along `axis`.
fn facing(mesh: &BlockMesh, axis: Vec3) -> u32 {
    (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().dot(axis) > 0.99)
        .expect("a cube faces every axis") as u32
}

#[test]
fn a_ray_hits_the_face_it_points_at() {
    let mesh = cube();
    for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        // Stand off along the axis and look back at the centre.
        let hit = face_at(&mesh, axis * 3.0, -axis).expect("the cube is in the way");
        assert_eq!(hit.element, facing(&mesh, axis), "looking down {axis}");
    }
}

#[test]
fn the_distance_reaches_the_surface() {
    // From 3 units out, the near face of a half-extent-0.5 cube is 2.5.
    let hit = face_at(&cube(), Vec3::Z * 3.0, -Vec3::Z).unwrap();
    assert!((hit.distance - 2.5).abs() < 1e-4, "{}", hit.distance);
}

#[test]
fn the_near_face_wins() {
    // The ray crosses two faces; the one it reaches first is the answer.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z).unwrap();
    assert_eq!(hit.element, facing(&mesh, Vec3::Z));
    assert_ne!(hit.element, facing(&mesh, -Vec3::Z));
}

#[test]
fn a_ray_that_misses_answers_none() {
    assert!(face_at(&cube(), Vec3::new(5.0, 5.0, 3.0), -Vec3::Z).is_none());
}

#[test]
fn nothing_behind_the_eye_is_hit() {
    // Standing in front and looking away.
    assert!(face_at(&cube(), Vec3::Z * 3.0, Vec3::Z).is_none());
}

#[test]
fn a_ray_from_inside_hits_a_wall() {
    // 🔴 Two-sided on purpose: a block being edited is routinely seen
    // from inside, and culling backfaces makes the wall in front of you
    // unselectable exactly then.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::ZERO, Vec3::Z).expect("the far wall is still a face");
    assert_eq!(hit.element, facing(&mesh, Vec3::Z));
    assert!((hit.distance - 0.5).abs() < 1e-4);
}

#[test]
fn a_grazing_ray_still_lands() {
    // Straight at a corner of the +Z face. An epsilon too tight drops
    // it, and a face becomes unclickable along its own edges.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::new(0.5, 0.5, 3.0), -Vec3::Z);
    assert_eq!(hit.map(|h| h.element), Some(facing(&mesh, Vec3::Z)));
}

#[test]
fn an_unnormalised_ray_scales_its_answer() {
    // `distance` is in units of `direction`, so a doubled direction
    // halves it. What a caller comparing two `t` on one ray wants.
    let mesh = cube();
    let slow = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z).unwrap();
    let fast = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z * 2.0).unwrap();
    assert!((slow.distance - fast.distance * 2.0).abs() < 1e-4);
}

#[test]
fn an_empty_mesh_is_never_hit() {
    assert!(face_at(&BlockMesh::default(), Vec3::Z * 3.0, -Vec3::Z).is_none());
}

/// A camera 4 m down +Z looking back at the origin, 800x600.
///
/// Reverse-Z infinite, the same projection the viewport builds, so the
/// `w` these tests rank by is the one the editor ranks by.
fn screen() -> Screen {
    let eye = Vec3::new(0.0, 0.0, 4.0);
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    let proj = kooch_render::projection::perspective_infinite_rh_reverse_z(
        60f32.to_radians(),
        800.0 / 600.0,
        0.1,
    );
    Screen {
        clip: proj * view,
        size: Vec2::new(800.0, 600.0),
    }
}

/// Where a mesh-space point lands, in pixels.
fn at(screen: Screen, point: Vec3) -> Vec2 {
    let clip = screen.clip * point.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    Vec2::new(
        (ndc.x * 0.5 + 0.5) * screen.size.x,
        (0.5 - ndc.y * 0.5) * screen.size.y,
    )
}

#[test]
fn a_cursor_on_a_corner_picks_it() {
    let mesh = cube();
    let screen = screen();
    let corner = Vec3::new(0.5, 0.5, 0.5);
    let hit = vertex_at(&mesh, screen, at(screen, corner), 8.0).expect("a corner is under it");
    assert_eq!(mesh.positions()[hit.element as usize], corner);
}

#[test]
fn a_cursor_between_corners_misses() {
    let mesh = cube();
    let screen = screen();
    // The middle of the front face: no corner within eight pixels.
    assert!(vertex_at(&mesh, screen, at(screen, Vec3::ZERO), 8.0).is_none());
}

#[test]
fn a_tie_goes_to_the_nearer() {
    // A cube seen down an axis projects its front and back corners onto
    // the same pixel. Ranking by pixels alone would leave which one you
    // grabbed to iteration order.
    assert!(super::closer((10.0, 1.0), (10.0, 5.0)));
    assert!(!super::closer((10.0, 5.0), (10.0, 1.0)));
}

#[test]
fn a_far_pixel_loses_to_a_near_one() {
    // And the depth tie-break must not reach past the tie: something
    // twenty pixels off is not what you clicked, however close it is.
    assert!(!super::closer((30.0, 0.1), (10.0, 90.0)));
    assert!(super::closer((10.0, 90.0), (30.0, 0.1)));
}

#[test]
fn a_cursor_on_an_edge_picks_it() {
    let mesh = cube();
    let adjacency = Adjacency::of(&mesh);
    let screen = screen();
    // The middle of the cube's top-front edge, which no corner is near.
    let middle = Vec3::new(0.0, 0.5, 0.5);
    let hit =
        edge_at(&mesh, &adjacency, screen, at(screen, middle), 8.0).expect("an edge is there");
    let [from, to] = adjacency.edge_corners(hit.element).expect("a real edge");
    let ends = [
        mesh.positions()[from as usize],
        mesh.positions()[to as usize],
    ];
    assert!(ends.iter().all(|end| end.y > 0.0 && end.z > 0.0));
}

#[test]
fn a_cursor_inside_a_face_misses_every_edge() {
    let mesh = cube();
    let adjacency = Adjacency::of(&mesh);
    let screen = screen();
    assert!(edge_at(&mesh, &adjacency, screen, at(screen, Vec3::ZERO), 8.0).is_none());
}

#[test]
fn a_mirrored_ghost_is_not_hit() {
    // 🔴 The eye sits inside the cube, so four corners are behind it.
    // Dividing by their negative `w` mirrors them through the centre —
    // corner (0.5, 0.5, -0.5) lands at pixel (1166, 1166), 700 px from
    // the nearest real one. A cursor there must find nothing.
    let mesh = cube();
    let screen = Screen {
        clip: kooch_render::projection::perspective_infinite_rh_reverse_z(
            60f32.to_radians(),
            1.0,
            0.1,
        ) * Mat4::look_at_rh(Vec3::new(0.0, 0.0, -0.2), Vec3::ZERO, Vec3::Y),
        size: Vec2::splat(600.0),
    };
    assert!(vertex_at(&mesh, screen, Vec2::splat(1166.0254), 20.0).is_none());
}
