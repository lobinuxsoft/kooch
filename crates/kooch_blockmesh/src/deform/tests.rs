use super::*;
use kooch_render::mesh::MeshVertex;
use kooch_render::meshlet::MeshletDescriptor;

/// A descriptor with every field at zero, so a test names only what it cares about.
fn blank() -> MeshletDescriptor {
    // `Pod`, so all-zero is a legal value of it.
    unsafe { std::mem::zeroed() }
}

/// A bar running 0..4 along Z, one metre wide, with its normals pointing out along +X.
fn bar() -> MeshletMesh {
    let vertices: Vec<MeshVertex> = (0..=4)
        .map(|step| MeshVertex {
            position: [1.0, 0.0, step as f32],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        })
        .collect();
    let mut aabb = Aabb::empty();
    for vertex in &vertices {
        aabb.expand(Vec3::from(vertex.position));
    }
    MeshletMesh {
        meshlet_vertices: (0..vertices.len() as u32).collect(),
        meshlet_triangles: Vec::new(),
        meshlets: vec![MeshletDescriptor {
            vertex_offset: 0,
            vertex_count: vertices.len() as u32,
            cone_cutoff: 0.5,
            ..blank()
        }],
        vertices,
        aabb,
    }
}

/// A straight run along Z, so a bend can be told from a move.
fn straight() -> Vec<Knot> {
    [Vec3::ZERO, Vec3::new(0.0, 0.0, 8.0)]
        .map(Knot::at)
        .to_vec()
}

/// A right-angle turn, which is what makes the frame do any work at all.
fn corner() -> Vec<Knot> {
    [
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 4.0),
        Vec3::new(4.0, 0.0, 4.0),
    ]
    .map(Knot::at)
    .to_vec()
}

fn settings() -> SplineDeform {
    SplineDeform {
        source: None,
        axis: AXIS_Z,
        taper: false,
    }
}

/// The mesh's extent on its chosen axis maps to the whole curve, so a bar half as long as the
/// curve still reaches the far end.
#[test]
fn a_bend_fills_the_curve() {
    let bent = deform(&bar(), &straight(), false, &settings()).expect("bent");
    let along: Vec<f32> = bent.vertices.iter().map(|v| v.position[2]).collect();
    assert!((along[0] - 0.0).abs() < 0.05, "{along:?}");
    assert!((along[4] - 8.0).abs() < 0.05, "{along:?}");
}

/// ⚠️ The one the eye does not catch. A mesh whose normals were left alone bends correctly and
/// lights as if it were still straight, which reads as a broken material rather than a broken
/// deform — so it is checked here and nowhere else would.
#[test]
fn a_bend_turns_the_normals() {
    let bent = deform(&bar(), &corner(), false, &settings()).expect("bent");
    let last = Vec3::from(bent.vertices[4].normal);
    let first = Vec3::from(bent.vertices[0].normal);
    assert!(
        first.distance(last) > 0.1,
        "every normal came out the same, so they were not transformed: {first} vs {last}",
    );
    assert!(
        bent.vertices
            .iter()
            .all(|v| (Vec3::from(v.normal).length() - 1.0).abs() < 1e-3),
        "a normal came out unnormalised",
    );
}

/// 🔴 A bent cluster's normals no longer share one cone, and a cone that is wrong by any amount
/// culls geometry the camera can see. `>= 1.0` is what the cull shader reads as "do not try".
#[test]
fn a_bend_disables_the_cone() {
    let bent = deform(&bar(), &corner(), false, &settings()).expect("bent");
    assert!(bent.meshlets[0].cone_cutoff >= 1.0);
}

/// Bounds follow the geometry, or the culler throws away a mesh that moved out of the box it
/// remembers — which looks like the mesh flickering rather than like a bounds fault.
#[test]
fn a_bend_moves_the_bounds() {
    let bent = deform(&bar(), &corner(), false, &settings()).expect("bent");
    assert!(bent.aabb.max.x > 1.5, "{:?}", bent.aabb);
    let meshlet = bent.meshlets[0];
    assert_eq!(meshlet.aabb_max, <[f32; 3]>::from(bent.aabb.max));
    assert!(meshlet.bounding_radius > 0.0);
}

/// `radius` scales the cross-section against the default, so a curve nobody tapered carries the
/// mesh at the size it was modelled.
#[test]
fn a_taper_scales_across() {
    let mut points = straight();
    points[0].radius = DEFAULT_RADIUS;
    points[1].radius = DEFAULT_RADIUS * 3.0;
    let of = SplineDeform {
        taper: true,
        ..settings()
    };
    let bent = deform(&bar(), &points, false, &of).expect("bent");
    let across = |index: usize| Vec3::from(bent.vertices[index].position).x.abs();
    assert!((across(0) - 1.0).abs() < 0.05, "{}", across(0));
    assert!((across(4) - 3.0).abs() < 0.2, "{}", across(4));
}

/// Off by default, or every existing curve would silently resize whatever it carries.
#[test]
fn no_taper_keeps_the_size() {
    let mut points = straight();
    points[1].radius = DEFAULT_RADIUS * 4.0;
    let bent = deform(&bar(), &points, false, &settings()).expect("bent");
    let across = Vec3::from(bent.vertices[4].position).x.abs();
    assert!((across - 1.0).abs() < 0.05, "{across}");
}

/// 🔴 `None`, not an empty mesh. These are states an author passes through while building, and
/// replacing the geometry with nothing makes the entity blink out mid-edit.
#[test]
fn a_degenerate_bend_is_refused() {
    let flat = MeshletMesh {
        aabb: Aabb {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 1.0, 0.0),
        },
        ..bar()
    };
    assert!(deform(&bar(), &[], false, &settings()).is_none());
    assert!(deform(&bar(), &straight()[..1], false, &settings()).is_none());
    assert!(deform(&flat, &straight(), false, &settings()).is_none());
    let stalled = vec![Knot::at(Vec3::ZERO); 3];
    assert!(deform(&bar(), &stalled, false, &settings()).is_none());
}

/// Nothing may reach the GPU as NaN: it draws as nothing, with no error anywhere to say why.
#[test]
fn a_bend_is_always_finite() {
    let mut points = corner();
    points[1].radius = f32::NAN;
    let of = SplineDeform {
        taper: true,
        ..settings()
    };
    let Some(bent) = deform(&bar(), &points, false, &of) else {
        return;
    };
    assert!(
        bent.vertices
            .iter()
            .all(|v| Vec3::from(v.position).is_finite() && Vec3::from(v.normal).is_finite()),
        "a NaN radius reached the vertices",
    );
}

/// 🔴 Every axis keeps the mesh's chirality. A mirrored result is the classic sign that the
/// cross-section was split without keeping the triple right-handed.
#[test]
fn every_axis_keeps_its_hand() {
    for axis in [AXIS_X, AXIS_Y, AXIS_Z] {
        let of = SplineDeform { axis, ..settings() };
        let (along, up, side) = split(Vec3::new(1.0, 2.0, 3.0), axis);
        // The split is cyclic, so rebuilding from it returns the vector it came from.
        let back = match axis {
            AXIS_X => Vec3::new(along, up, side),
            AXIS_Y => Vec3::new(side, along, up),
            _ => Vec3::new(up, side, along),
        };
        assert_eq!(back, Vec3::new(1.0, 2.0, 3.0), "axis {axis} is not cyclic");
        assert!(deform(&bar(), &straight(), false, &of).is_some() || axis != AXIS_Z);
    }
}

/// A field added to the component and not hashed is a control that edits nothing the author can
/// see — the trap `SplineSweep` fell into with `wall`.
#[test]
fn every_setting_changes_the_revision() {
    let base = revision(&settings());
    assert_ne!(
        base,
        revision(&SplineDeform {
            axis: AXIS_X,
            ..settings()
        })
    );
    assert_ne!(
        base,
        revision(&SplineDeform {
            taper: true,
            ..settings()
        })
    );
    assert_ne!(
        base,
        revision(&SplineDeform {
            source: Some(kooch_core::Guid::from_bytes([7; 16])),
            ..settings()
        })
    );
}

/// 🔴 The strongest thing this can promise, and the one that caught a real bug: a straight curve
/// running along the chosen axis leaves the mesh **where it was modelled**. The first version
/// mapped the two cross-section axes the wrong way round and quietly rotated every mesh by 90°,
/// which no bend test notices because a bent mesh is supposed to move.
#[test]
fn a_straight_curve_is_identity() {
    let bent = deform(&bar(), &straight(), false, &settings()).expect("bent");
    for (index, vertex) in bent.vertices.iter().enumerate() {
        let was = Vec3::new(1.0, 0.0, index as f32 * 2.0);
        let now = Vec3::from(vertex.position);
        assert!(
            now.distance(was) < 0.05,
            "vertex {index}: {was} became {now}"
        );
        let normal = Vec3::from(vertex.normal);
        assert!(normal.distance(Vec3::X) < 0.05, "normal {index}: {normal}");
    }
}
