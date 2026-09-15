use super::*;
use crate::material::Shader;

fn toon() -> Shader {
    Shader::parse(
        "struct SurfaceParams {
            tint: vec4<f32>,  // @color
            strength: f32,    // @range(0, 4)
            offset: vec2<f32>,
        }
        const SURFACE_DEFAULTS = SurfaceParams(vec4(1.0, 0.5, 0.2, 1.0), 2.0, vec2(0.0));
        var detail: texture_2d<f32>;  // @default(normal)",
    )
    .unwrap()
}

/// A material that says nothing packs every default at its declared offset.
#[test]
fn defaults_fill_the_block() {
    let packed = PackedParams::for_shader(&Material::default(), &toon().params);
    assert_eq!(&packed.values[..7], &[1.0, 0.5, 0.2, 1.0, 2.0, 0.0, 0.0]);
    assert_eq!(packed.textures[0].fallback, TextureDefault::Normal);
    assert_eq!(packed.textures[0].guid, None);
}

#[test]
fn a_stored_value_wins() {
    let mut material = Material::default();
    let map = Guid::new_v4();
    material
        .values
        .insert("strength".into(), ParamValue::Number([3.5, 0.0, 0.0, 0.0]));
    material
        .values
        .insert("detail".into(), ParamValue::Texture(Some(map)));
    let packed = PackedParams::for_shader(&material, &toon().params);
    assert_eq!(packed.values[4], 3.5);
    assert_eq!(packed.textures[0].guid, Some(map));
}

/// A value of the wrong kind reads as absent rather than as garbage.
#[test]
fn a_mismatched_value_is_default() {
    let mut material = Material::default();
    material
        .values
        .insert("tint".into(), ParamValue::Texture(None));
    let packed = PackedParams::for_shader(&material, &toon().params);
    assert_eq!(&packed.values[..4], &[1.0, 0.5, 0.2, 1.0]);
}

/// Switching shader keeps what both declare and drops the rest.
#[test]
fn switching_keeps_shared_values() {
    let mut values = ParamValues::new();
    values.insert("tint".into(), ParamValue::Number([0.0; 4]));
    values.insert("gone".into(), ParamValue::Number([1.0; 4]));
    values.insert("detail".into(), ParamValue::Number([1.0; 4]));
    retain_declared(&mut values, &toon().params);
    assert_eq!(values.keys().collect::<Vec<_>>(), vec!["tint"]);
}

#[test]
fn the_default_surface_uses_the_maps() {
    let albedo = Guid::new_v4();
    let packed = PackedParams::default_surface(&Material::default().with_albedo(albedo));
    assert_eq!(packed.textures[0].guid, Some(albedo));
    assert_eq!(packed.textures[1].fallback, TextureDefault::Normal);
}
