//! Export tests. The one that matters is the round-trip: a file this
//! writes has to come back through the engine's own importer, because
//! that importer is the only consumer whose opinion is binding.

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, Primitive, parse_mesh_bytes};

use super::{ExportError, SimplifyTarget, simplify, to_glb};

/// Exports and re-imports through the engine's glTF loader.
fn round_trip(mesh: &Mesh, name: &str) -> Mesh {
    let bytes = to_glb(mesh, name).expect("export failed");
    parse_mesh_bytes(&bytes).expect("the engine could not read back its own export")
}

/// The acceptance criterion: same counts, same bounds, back through the
/// importer. Not a byte comparison — the JSON key order is serde's
/// business, and asserting on it would break on a dependency bump while
/// telling us nothing about whether the geometry survived.
#[test]
fn every_primitive_round_trips_through_the_importer() {
    for (name, primitive) in Primitive::CANONICAL {
        let original = primitive.build();
        let reloaded = round_trip(&original, name);

        assert_eq!(
            reloaded.vertex_count(),
            original.vertex_count(),
            "{name}: vertex count changed"
        );
        assert_eq!(
            reloaded.index_count(),
            original.index_count(),
            "{name}: index count changed"
        );
        assert!(
            reloaded.aabb.min.abs_diff_eq(original.aabb.min, 1e-4)
                && reloaded.aabb.max.abs_diff_eq(original.aabb.max, 1e-4),
            "{name}: bounds changed, {:?}..{:?} became {:?}..{:?}",
            original.aabb.min,
            original.aabb.max,
            reloaded.aabb.min,
            reloaded.aabb.max
        );
    }
}

/// Positions, normals and UVs all survive, not just the counts. The
/// interleaved layout means one wrong accessor byte offset silently reads
/// normals as positions — with the right count and plausible-looking data.
#[test]
fn round_trip_preserves_the_vertex_attributes() {
    let original = Primitive::Cube {
        half_extents: Vec3::new(1.0, 2.0, 3.0),
    }
    .build();
    let reloaded = round_trip(&original, "cube");

    for (i, (a, b)) in original
        .vertices
        .iter()
        .zip(reloaded.vertices.iter())
        .enumerate()
    {
        assert!(
            Vec3::from_array(a.position).abs_diff_eq(Vec3::from_array(b.position), 1e-5),
            "vertex {i} position: {:?} became {:?}",
            a.position,
            b.position
        );
        assert!(
            Vec3::from_array(a.normal).abs_diff_eq(Vec3::from_array(b.normal), 1e-5),
            "vertex {i} normal: {:?} became {:?} — check the accessor byte offsets",
            a.normal,
            b.normal
        );
        assert!(
            Vec2::from_array(a.uv).abs_diff_eq(Vec2::from_array(b.uv), 1e-5),
            "vertex {i} uv: {:?} became {:?}",
            a.uv,
            b.uv
        );
    }
    assert_eq!(original.indices, reloaded.indices, "triangle order changed");
}

/// GLB chunks must be 4-byte aligned or a conformant reader rejects the
/// file — including readers that are not this engine.
#[test]
fn the_container_is_four_byte_aligned() {
    let bytes = to_glb(&Primitive::CANONICAL[0].1.build(), "cube").unwrap();
    assert_eq!(bytes.len() % 4, 0, "total length is not aligned");
    assert_eq!(&bytes[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
        bytes.len(),
        "the header length disagrees with the file"
    );
}

/// A mesh that cannot be a valid asset is refused here, where the
/// generator that produced it is still in the picture — not at import
/// time in whatever opens the file next.
#[test]
fn invalid_meshes_are_refused_rather_than_written() {
    assert!(matches!(
        to_glb(&Mesh::empty(), "empty"),
        Err(ExportError::Empty)
    ));

    let mut partial = Primitive::CANONICAL[0].1.build();
    partial.indices.pop();
    assert!(matches!(
        to_glb(&partial, "partial"),
        Err(ExportError::PartialTriangle(_))
    ));

    let mut bad = Primitive::CANONICAL[0].1.build();
    let count = bad.vertex_count();
    bad.indices[0] = count + 10;
    assert!(matches!(
        to_glb(&bad, "bad"),
        Err(ExportError::IndexOutOfRange { .. })
    ));
}

// ---------------------------------------------------------------------------
// Simplification
// ---------------------------------------------------------------------------

/// The collision-proxy path: decimate, and the result is still a mesh
/// that exports and re-imports.
#[test]
fn simplifying_a_sphere_reduces_it_and_still_round_trips() {
    let original = Primitive::Sphere {
        radius: 1.0,
        rings: 32,
        sectors: 48,
    }
    .build();
    let before = original.index_count() / 3;

    let (proxy, error) = simplify(&original, SimplifyTarget::Ratio(0.25));
    let after = proxy.index_count() / 3;

    assert!(after < before, "no reduction: {before} → {after}");
    assert!(error.is_finite() && error >= 0.0, "nonsense error: {error}");
    // The silhouette has to survive, or the collider is the wrong shape.
    assert!(
        proxy.aabb.max.abs_diff_eq(original.aabb.max, 0.2),
        "bounds drifted: {:?} vs {:?}",
        original.aabb.max,
        proxy.aabb.max
    );

    let reloaded = round_trip(&proxy, "sphere_collision");
    assert_eq!(reloaded.index_count(), proxy.index_count());
}

/// Orphaned vertices are dropped. `meshopt` returns indices into the
/// original array, so without compaction the proxy carries every collapsed
/// vertex — a file whose vertex count says nothing about its complexity.
#[test]
fn simplification_drops_orphaned_vertices() {
    let original = Primitive::Sphere {
        radius: 1.0,
        rings: 32,
        sectors: 48,
    }
    .build();
    let (proxy, _) = simplify(&original, SimplifyTarget::Ratio(0.1));

    assert!(
        proxy.vertex_count() < original.vertex_count(),
        "vertices were kept: {} vs {}",
        original.vertex_count(),
        proxy.vertex_count()
    );
    for &i in &proxy.indices {
        assert!(
            i < proxy.vertex_count(),
            "index {i} survived compaction stale"
        );
    }
}

/// Asking for more than there is, or for an impossible reduction, returns
/// the mesh — "unchanged" is the answer, not an error the caller has to
/// special-case.
#[test]
fn a_target_that_cannot_reduce_returns_the_mesh() {
    let original = Primitive::CANONICAL[0].1.build();
    let triangles = original.index_count() / 3;

    for target in [
        SimplifyTarget::Ratio(1.0),
        SimplifyTarget::Ratio(5.0),
        SimplifyTarget::Triangles(triangles * 10),
    ] {
        let (out, error) = simplify(&original, target);
        assert_eq!(out.index_count(), original.index_count(), "{target:?}");
        assert_eq!(error, 0.0);
    }
}

/// A degenerate target never produces an empty mesh: a collider with no
/// triangles collides with nothing, silently.
#[test]
fn a_zero_target_still_leaves_geometry() {
    let original = Primitive::Sphere {
        radius: 1.0,
        rings: 16,
        sectors: 24,
    }
    .build();
    for target in [SimplifyTarget::Ratio(0.0), SimplifyTarget::Triangles(0)] {
        let (out, _) = simplify(&original, target);
        assert!(out.index_count() >= 3, "{target:?} produced no triangles");
        assert_eq!(out.index_count() % 3, 0);
    }
}
