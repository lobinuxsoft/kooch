use super::*;
use std::path::Path;

#[test]
fn extension_is_ron() {
    assert_eq!(MaterialLoader.extensions(), &["ron"]);
}

#[test]
fn default_matches_legacy_white_diffuse() {
    let m = Material::default();
    assert_eq!(m.base_color, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(m.metallic, 0.0);
    assert_eq!(m.roughness, 0.5);
    assert_eq!(m.emissive, 0.0);
    // Textures are opt-in: a fresh material references none.
    assert_eq!(m.albedo, None);
    assert_eq!(m.normal, None);
    assert_eq!(m.metal_roughness, None);
}

#[test]
fn builders_attach_texture_guids() {
    let (a, n, mr) = (Guid::new_v4(), Guid::new_v4(), Guid::new_v4());
    let m = Material::default()
        .with_albedo(a)
        .with_normal(n)
        .with_metal_roughness(mr);
    assert_eq!(m.albedo, Some(a));
    assert_eq!(m.normal, Some(n));
    assert_eq!(m.metal_roughness, Some(mr));
}

#[test]
fn to_params_round_trips_scalars() {
    let m = Material::new([0.2, 0.3, 0.4, 1.0], 0.6, 0.7, 1.5);
    let p = m.to_params();
    assert_eq!(p.base_color, [0.2, 0.3, 0.4, 1.0]);
    assert_eq!(p.metallic(), 0.6);
    assert_eq!(p.roughness(), 0.7);
    assert_eq!(p.emissive(), 1.5);
}

#[test]
fn ron_minimal_uses_defaults() {
    // Empty struct literal — every field falls through to its
    // serde default. Exercises the back-compat contract: future
    // schemas must preserve this property.
    let mut ctx = LoadContext::new(Path::new("empty.kooch_material.ron"));
    let m = MaterialLoader
        .load(b"()", &mut ctx)
        .expect("empty struct parses");
    assert_eq!(m, Material::default());
}

#[test]
fn ron_full_round_trip() {
    let original = Material::new([0.9, 0.1, 0.05, 1.0], 0.0, 0.4, 0.0)
        .with_albedo(Guid::new_v4())
        .with_normal(Guid::new_v4())
        .with_metal_roughness(Guid::new_v4());
    let text = ron::ser::to_string_pretty(&original, ron::ser::PrettyConfig::default())
        .expect("serialize");
    let mut ctx = LoadContext::new(Path::new("red.kooch_material.ron"));
    let parsed = MaterialLoader
        .load(text.as_bytes(), &mut ctx)
        .expect("parse");
    assert_eq!(parsed, original);
}

#[test]
fn ron_parses_texture_guid_literals() {
    use std::str::FromStr;
    let text = r#"(
    base_color: (0.8, 0.8, 0.8, 1.0),
    albedo: Some("550e8400-e29b-41d4-a716-446655440000"),
    metal_roughness: Some("00000000-0000-0000-0000-000000000001"),
)"#;
    let mut ctx = LoadContext::new(Path::new("textured.kooch_material.ron"));
    let m = MaterialLoader
        .load(text.as_bytes(), &mut ctx)
        .expect("parse");
    assert_eq!(
        m.albedo,
        Some(Guid::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap())
    );
    assert_eq!(
        m.metal_roughness,
        Some(Guid::from_str("00000000-0000-0000-0000-000000000001").unwrap())
    );
    // Normal elided → stays None; scalars fall back to defaults.
    assert_eq!(m.normal, None);
    assert_eq!(m.roughness, 0.5);
}

#[test]
fn ron_partial_fills_remaining_with_defaults() {
    let text = r#"(
    base_color: (0.1, 0.2, 0.3, 1.0),
    emissive: 2.0,
)"#;
    let mut ctx = LoadContext::new(Path::new("partial.kooch_material.ron"));
    let m = MaterialLoader
        .load(text.as_bytes(), &mut ctx)
        .expect("parse");
    assert_eq!(m.base_color, [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(m.emissive, 2.0);
    // Missing fields fell back to defaults.
    assert_eq!(m.metallic, 0.0);
    assert_eq!(m.roughness, 0.5);
}

#[test]
fn invalid_bytes_return_loader_error() {
    let mut ctx = LoadContext::new(Path::new("garbage.kooch_material.ron"));
    let err = MaterialLoader
        .load(b"not RON at all =", &mut ctx)
        .expect_err("garbage rejected");
    assert!(matches!(err, AssetError::Loader(_)));
}
