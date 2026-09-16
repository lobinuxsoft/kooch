use super::*;

/// 🔴 The engine's meshes carry no tangent, and **Unpack Normal** means nothing without one — a
/// normal map would preview as noise, which is worse than not previewing it at all.
#[test]
fn every_tangent_is_square() {
    let mesh = Primitive::CANONICAL[default_primitive()].1.build();

    let vertices = with_tangents(&mesh);

    assert_eq!(vertices.len(), mesh.vertices.len(), "a vertex was dropped");
    for vertex in &vertices {
        let normal = Vec3::from(vertex.normal);
        let tangent = Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]);
        assert!(
            tangent.is_finite() && tangent.length() > 0.5,
            "a vertex came out with no tangent: {tangent:?}",
        );
        assert!(
            normal.dot(tangent).abs() < 0.001,
            "the tangent leans out of the surface: {}",
            normal.dot(tangent),
        );
    }
}

/// A preview opens on the shape a material is judged on.
#[test]
fn a_preview_starts_round() {
    assert_eq!(Primitive::CANONICAL[default_primitive()].0, "sphere");
}

/// Every shape the menu offers has to survive the tangent pass — a quad's uv is degenerate in one
/// direction, and a cone has a vertex every triangle disagrees about.
#[test]
fn every_primitive_uploads() {
    for (name, primitive) in Primitive::CANONICAL {
        let mesh = primitive.build();
        let vertices = with_tangents(&mesh);
        assert!(!vertices.is_empty(), "{name} built nothing");
        assert!(
            vertices
                .iter()
                .all(|v| v.tangent.iter().all(|c| c.is_finite())),
            "{name} produced a tangent that is not a number",
        );
    }
}
