use super::*;
use glam::{Mat4, Quat};
use kooch_core::resource::Resources;
use kooch_gizmos::{GizmoBatch, MeshBatch};

/// Draws one collider and returns the line segments produced.
fn draw(collider: &Collider, matrix: Mat4) -> Vec<(Vec3, Vec3)> {
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        ColliderVisualizer.draw(collider, &GlobalTransform { matrix }, &mut gizmos);
    }
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}

/// The furthest any drawn point sits from `centre`, per axis.
fn extent(lines: &[(Vec3, Vec3)], centre: Vec3) -> Vec3 {
    lines
        .iter()
        .flat_map(|(a, b)| [*a, *b])
        .fold(Vec3::ZERO, |acc, p| acc.max((p - centre).abs()))
}

#[test]
fn a_collider_draws_something() {
    for shape in [SHAPE_CUBOID, SHAPE_CAPSULE, u32::MAX] {
        let collider = Collider {
            shape,
            ..Default::default()
        };
        assert!(
            !draw(&collider, Mat4::IDENTITY).is_empty(),
            "shape {shape} drew nothing"
        );
    }
}

/// The outline follows the *effective* shape, scale included. This is
/// the assertion that matters: an outline drawn from the authored
/// numbers would lie precisely where a collider is most likely wrong.
#[test]
fn the_outline_grows_with_the_transform_scale() {
    let collider = Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::splat(0.5),
        ..Default::default()
    };
    let unscaled = extent(&draw(&collider, Mat4::IDENTITY), Vec3::ZERO);
    let scaled = extent(
        &draw(
            &collider,
            Mat4::from_scale_rotation_translation(
                Vec3::new(3.0, 1.0, 5.0),
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
        ),
        Vec3::ZERO,
    );

    assert!(unscaled.abs_diff_eq(Vec3::splat(0.5), 1e-4), "{unscaled:?}");
    assert!(
        scaled.abs_diff_eq(Vec3::new(1.5, 0.5, 2.5), 1e-4),
        "the outline ignored the transform scale: {scaled:?}"
    );
}

/// A sphere takes the largest axis, exactly as the solver does.
#[test]
fn a_scaled_sphere_outline_takes_the_largest_axis() {
    let extent = extent(
        &draw(
            &Collider::default(),
            Mat4::from_scale_rotation_translation(
                Vec3::new(1.0, 4.0, 2.0),
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
        ),
        Vec3::ZERO,
    );
    // 0.5 * 4.0 = 2.0 on every axis: a sphere, not an ellipsoid.
    assert!(extent.min_element() > 1.9, "got {extent:?}");
    assert!(extent.max_element() < 2.1, "got {extent:?}");
}

/// A capsule scaled on Y gets taller, not fatter.
#[test]
fn a_scaled_capsule_outline_grows_along_its_own_axis() {
    let collider = Collider {
        shape: SHAPE_CAPSULE,
        radius: 0.5,
        half_height: 1.0,
        ..Default::default()
    };
    let extent = extent(
        &draw(
            &collider,
            Mat4::from_scale_rotation_translation(
                Vec3::new(1.0, 3.0, 1.0),
                Quat::IDENTITY,
                Vec3::ZERO,
            ),
        ),
        Vec3::ZERO,
    );
    // half_height 3.0 plus radius 0.5 = 3.5 tall, still 0.5 wide.
    assert!((extent.y - 3.5).abs() < 0.05, "height wrong: {extent:?}");
    assert!((extent.x - 0.5).abs() < 0.05, "it got fatter: {extent:?}");
}

/// The outline is drawn where the entity is, not at the origin.
#[test]
fn the_outline_follows_the_entity() {
    let centre = Vec3::new(10.0, -4.0, 7.0);
    let lines = draw(&Collider::default(), Mat4::from_translation(centre));
    let extent = extent(&lines, centre);
    assert!(
        extent.max_element() < 0.6,
        "the outline is not centred on the entity: {extent:?}"
    );
}

/// A rotated entity's outline rotates with it. Drawing it
/// axis-aligned would be worse than drawing nothing — it would look
/// like the collider had not rotated.
#[test]
fn the_outline_rotates_with_the_entity() {
    let collider = Collider {
        shape: SHAPE_CUBOID,
        half_extents: Vec3::new(2.0, 0.1, 0.1),
        ..Default::default()
    };
    let flat = extent(&draw(&collider, Mat4::IDENTITY), Vec3::ZERO);
    assert!(flat.x > 1.9 && flat.z < 0.2, "{flat:?}");

    // A quarter turn about Y swaps the long axis from X to Z.
    let turned = extent(
        &draw(
            &collider,
            Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2),
        ),
        Vec3::ZERO,
    );
    assert!(
        turned.z > 1.9 && turned.x < 0.2,
        "the outline did not rotate: {turned:?}"
    );
}

/// The outline follows `Collider.center`, so what you see is where the
/// solver collides.
#[test]
fn the_outline_sits_at_the_shape_centre() {
    let collider = Collider {
        center: Vec3::new(0.0, 2.0, 0.0),
        ..Default::default()
    };
    let lines = draw(&collider, Mat4::IDENTITY);

    // Tight around the offset centre, and nothing near the origin.
    assert!(
        extent(&lines, Vec3::new(0.0, 2.0, 0.0)).max_element() < 0.6,
        "the outline is not centred on the shape"
    );
    let lowest = lines
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .fold(f32::INFINITY, f32::min);
    assert!(
        lowest > 1.4,
        "the outline reaches down to the entity origin: lowest y = {lowest}"
    );
}

/// The offset rotates with the entity, and scales with it.
#[test]
fn the_shape_centre_follows_the_transform() {
    let collider = Collider {
        center: Vec3::new(0.0, 1.0, 0.0),
        ..Default::default()
    };

    // A half turn about X sends a +Y offset to -Y.
    let turned = draw(&collider, Mat4::from_rotation_x(std::f32::consts::PI));
    assert!(
        extent(&turned, Vec3::new(0.0, -1.0, 0.0)).max_element() < 0.6,
        "the offset did not rotate with the entity"
    );

    // Scale multiplies the offset along with the dimensions.
    let scaled = draw(
        &collider,
        Mat4::from_scale_rotation_translation(Vec3::splat(3.0), Quat::IDENTITY, Vec3::ZERO),
    );
    assert!(
        extent(&scaled, Vec3::new(0.0, 3.0, 0.0)).max_element() < 1.7,
        "the offset did not scale with the entity"
    );
}

/// Multi-selection draws one outline per entity, into the same batch.
///
/// The dispatch loop relies on visualizers *appending* rather than
/// replacing, so two selected bodies produce two outlines at two
/// places. A visualizer that overwrote the batch would show only the
/// last entity, which is what suppressing multi-selection used to hide.
#[test]
fn two_entities_produce_two_outlines_in_one_batch() {
    let collider = Collider::default();
    let (a, b) = (Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));

    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        for centre in [a, b] {
            ColliderVisualizer.draw(
                &collider,
                &GlobalTransform {
                    matrix: Mat4::from_translation(centre),
                },
                &mut gizmos,
            );
        }
    }
    let segments: Vec<(Vec3, Vec3)> = lines.lines.iter().map(|s| (s.start, s.end)).collect();

    let single = draw(&collider, Mat4::from_translation(a)).len();
    assert_eq!(
        segments.len(),
        single * 2,
        "the second entity did not add its own outline"
    );
    // One cluster around each entity, nothing stranded between them.
    for centre in [a, b] {
        let near = segments
            .iter()
            .filter(|(p, _)| (*p - centre).length() < 1.0)
            .count();
        assert_eq!(near, single, "no outline at {centre:?}");
    }
}

/// The shapes #137 added draw their own outline. Falling back to a
/// sphere would show a shape the solver is not using, in the one tool
/// that exists to tell the truth about that.
#[test]
fn every_analytic_shape_draws_itself() {
    for shape in [
        SHAPE_CYLINDER,
        SHAPE_ROUND_CYLINDER,
        SHAPE_CONE,
        SHAPE_HALF_SPACE,
        SHAPE_SEGMENT,
        SHAPE_TRIANGLE,
    ] {
        let collider = Collider {
            shape,
            ..Default::default()
        };
        assert!(
            !draw(&collider, Mat4::IDENTITY).is_empty(),
            "shape {shape} drew nothing"
        );
    }
}

/// A cone is not a sphere: its base is at `-half_height` and its apex at
/// `+half_height`, so the outline has to be taller than it is wide.
#[test]
fn a_cone_is_drawn_pointing_up() {
    let collider = Collider {
        shape: SHAPE_CONE,
        radius: 0.5,
        half_height: 2.0,
        ..Default::default()
    };
    let lines = draw(&collider, Mat4::IDENTITY);
    let highest = lines
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((highest - 2.0).abs() < 0.05, "apex at {highest}, not 2.0");
    assert!(extent(&lines, Vec3::ZERO).x < 0.6, "it is not a cone");
}

/// A mesh-derived shape has nothing to draw from `draw` alone — its
/// points are in the cache, which only `draw_with` can reach. A sphere
/// here would be a shape the solver is not using.
#[test]
fn a_mesh_shape_draws_nothing_without_the_cache() {
    let collider = Collider {
        shape: kooch_physics::components::SHAPE_CONVEX_HULL,
        ..Default::default()
    };
    assert!(draw(&collider, Mat4::IDENTITY).is_empty());
}

/// Draws one collider through the path that can reach the cache.
fn draw_with(collider: &Collider, resources: &Resources, matrix: Mat4) -> Vec<(Vec3, Vec3)> {
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        ColliderVisualizer.draw_with(
            collider,
            &GlobalTransform { matrix },
            resources,
            &mut gizmos,
        );
    }
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}

/// A tetrahedron, with the faces that say it is already a hull.
fn cached_hull() -> (Resources, kooch_core::Guid) {
    use kooch_physics::{ColliderMesh, ColliderMeshCache, ConvexPart};

    let points = vec![
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let faces = vec![[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];

    let guid = kooch_core::Guid::new_v4();
    let mut cache = ColliderMeshCache::new();
    cache.insert(
        guid,
        ColliderMesh {
            vertices: points.clone(),
            hull: ConvexPart { points, faces },
            ..Default::default()
        },
    );
    let mut resources = Resources::new();
    resources.insert(cache);
    (resources, guid)
}

/// The whole point of #574: a hull authored as a mesh gets an outline of
/// that mesh's hull, not of a sphere and not of nothing.
#[test]
fn a_hull_outlines_its_cached_points() {
    let (resources, guid) = cached_hull();
    let collider = Collider {
        shape: kooch_physics::components::SHAPE_CONVEX_HULL,
        mesh: Some(guid),
        ..Default::default()
    };
    let lines = draw_with(&collider, &resources, Mat4::IDENTITY);
    assert!(!lines.is_empty(), "the hull drew nothing");
    // A tetrahedron has six edges, and `wire_triangles` deduplicates the
    // ones its four faces share.
    assert_eq!(lines.len(), 6, "shared edges were drawn twice");
}

/// The outline follows the solver's scale folding, the same as every
/// other shape here — an outline at the authored size would lie exactly
/// where a scaled collider is most likely to be wrong.
#[test]
fn a_scaled_hull_outline_grows() {
    let (resources, guid) = cached_hull();
    let collider = Collider {
        shape: kooch_physics::components::SHAPE_CONVEX_HULL,
        mesh: Some(guid),
        ..Default::default()
    };
    let plain = extent(
        &draw_with(&collider, &resources, Mat4::IDENTITY),
        Vec3::ZERO,
    );
    let scaled = extent(
        &draw_with(
            &collider,
            &resources,
            Mat4::from_scale_rotation_translation(Vec3::splat(3.0), Quat::IDENTITY, Vec3::ZERO),
        ),
        Vec3::ZERO,
    );
    assert!(
        (scaled.max_element() - plain.max_element() * 3.0).abs() < 1e-4,
        "{plain:?} -> {scaled:?}"
    );
}

/// A mesh that has not arrived draws nothing — the body was not built
/// either, so an outline would be the only thing claiming there is a
/// collider here.
#[test]
fn an_unresolved_hull_draws_nothing() {
    let resources = Resources::new();
    let collider = Collider {
        shape: kooch_physics::components::SHAPE_CONVEX_HULL,
        mesh: Some(kooch_core::Guid::new_v4()),
        ..Default::default()
    };
    assert!(draw_with(&collider, &resources, Mat4::IDENTITY).is_empty());
}

/// A triangle mesh IS the render mesh, edge for edge. Outlining it draws
/// a second copy of what is already on screen.
#[test]
fn a_trimesh_draws_nothing() {
    let (resources, guid) = cached_hull();
    let collider = Collider {
        shape: kooch_physics::components::SHAPE_TRIMESH,
        mesh: Some(guid),
        ..Default::default()
    };
    assert!(draw_with(&collider, &resources, Mat4::IDENTITY).is_empty());
}
