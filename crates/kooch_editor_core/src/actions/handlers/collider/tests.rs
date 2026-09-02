use super::*;

use glam::Vec3;

/// A shape with volume and more faces than a hull needs, so a budget has
/// something to remove.
fn sphere_points() -> Vec<Vec3> {
    let mut points = Vec::new();
    for i in 0..24 {
        for j in 0..24 {
            let u = i as f32 / 24.0 * std::f32::consts::TAU;
            let v = j as f32 / 24.0 * std::f32::consts::PI;
            points.push(Vec3::new(v.sin() * u.cos(), v.cos(), v.sin() * u.sin()));
        }
    }
    points
}

#[test]
fn a_cube_hulls_to_its_corners() {
    let corners: Vec<Vec3> = (0..8)
        .map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32))
        .collect();
    let mesh = hull_mesh(&corners, 0).expect("a cube has volume");
    assert_eq!(mesh.vertices.len(), 8);
    assert_eq!(mesh.indices.len(), 12 * 3, "a cube is twelve triangles");
}

/// The exact hull of an organic mesh is a few hundred planes and the
/// narrowphase pays for every one.
#[test]
fn a_budget_removes_faces() {
    let points = sphere_points();
    let exact = hull_mesh(&points, 0).expect("a sphere has volume");
    let capped = hull_mesh(&points, 32).expect("still has volume");
    assert!(
        capped.indices.len() < exact.indices.len(),
        "budget {} did not reduce {}",
        capped.indices.len() / 3,
        exact.indices.len() / 3,
    );
}

/// A budget above the exact hull is a no-op, not a rebuild that drifts.
#[test]
fn a_generous_budget_changes_nothing() {
    let points = sphere_points();
    let exact = hull_mesh(&points, 0).expect("has volume");
    let same = hull_mesh(&points, 100_000).expect("has volume");
    assert_eq!(exact.indices.len(), same.indices.len());
}

/// Collinear points have no volume, and a hull of them would be a
/// collider that cannot be hit.
#[test]
fn a_flat_cloud_makes_no_mesh() {
    let flat = vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
    assert!(hull_mesh(&flat, 0).is_none());
}

#[test]
fn the_hash_follows_the_bytes() {
    assert_eq!(hash_of(b"suzanne"), hash_of(b"suzanne"));
    assert_ne!(hash_of(b"suzanne"), hash_of(b"suzanne "));
}

/// Re-baking must leave every collider that already points at the file
/// still pointing at it.
#[test]
fn rebaking_keeps_the_guid() {
    let dir = std::env::temp_dir().join(format!("kooch_bake_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join("suzanne_hull.glb");
    std::fs::write(&out, b"not really a glb").expect("write");

    let source = Guid::new_v4();
    write_sidecar(&out, source, "hull", 1);
    let first = asset_meta::read_meta(&out).expect("sidecar").guid;

    write_sidecar(&out, source, "hull", 2);
    let meta = asset_meta::read_meta(&out).expect("sidecar");
    assert_eq!(meta.guid, first, "a re-bake orphaned every collider");

    let import = meta.import.expect("import table");
    assert_eq!(import.get(KEY_KIND).and_then(|v| v.as_str()), Some("hull"));
    assert_eq!(
        import.get(KEY_SOURCE).and_then(|v| v.as_str()),
        Some(source.to_string().as_str()),
        "without the source there is no way to know the bake is stale",
    );
    assert_eq!(
        import.get(KEY_HASH).and_then(|v| v.as_str()),
        Some("0000000000000002"),
        "the hash has to follow the source it was baked from",
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A decimated mesh is the one bake that keeps the source's topology
/// rather than replacing it with a hull, so it has to actually shrink.
#[test]
fn decimation_removes_triangles() {
    let points = sphere_points();
    let (hull, faces) = hull_of(&points).expect("has volume");
    let mesh = Mesh::from_triangles(&hull, &faces);

    let smaller = decimate(&mesh, 64, Guid::new_v4());
    assert!(
        smaller.indices.len() < mesh.indices.len(),
        "{} faces did not come down from {}",
        smaller.indices.len() / 3,
        mesh.indices.len() / 3,
    );
    assert!(!smaller.indices.is_empty(), "it decimated to nothing");
}

/// Unlike a hull, this bake can move the surface — so the caller is told
/// how far, and a target it cannot reach is the mesh unchanged rather
/// than an error.
#[test]
fn an_unreachable_target_returns_the_mesh() {
    let corners: Vec<Vec3> = (0..8)
        .map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32))
        .collect();
    let mesh = hull_mesh(&corners, 0).expect("a cube has volume");
    let same = decimate(&mesh, 100_000, Guid::new_v4());
    assert_eq!(same.indices.len(), mesh.indices.len());
}
