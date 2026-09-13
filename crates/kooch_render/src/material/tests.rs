use super::*;

#[test]
fn material_params_layout_is_pod_64_bytes() {
    // 16 B base_color + 16 B (metallic, rough, emissive, pad)
    // + 16 B (albedo, normal, metal_rough, pad)
    // + 16 B (uv scale, uv offset) = 64 B.
    assert_eq!(std::mem::size_of::<MaterialParams>(), 64);
    assert_eq!(std::mem::align_of::<MaterialParams>(), 4);
}

#[test]
fn default_material_is_white_diffuse_mid_roughness() {
    let m = MaterialParams::default();
    assert_eq!(m.base_color, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(m.metallic(), 0.0);
    assert_eq!(m.roughness(), 0.5);
    assert_eq!(m.emissive(), 0.0);
    // No maps by default — every channel carries the sentinel.
    assert_eq!(m.albedo_index(), NO_TEXTURE);
    assert_eq!(m.normal_index(), NO_TEXTURE);
    assert_eq!(m.metal_roughness_index(), NO_TEXTURE);
}

#[test]
fn new_packs_scalars_correctly() {
    let m = MaterialParams::new([0.2, 0.4, 0.8, 1.0], 0.7, 0.3, 1.5);
    assert_eq!(m.base_color(), [0.2, 0.4, 0.8, 1.0]);
    assert_eq!(m.metallic(), 0.7);
    assert_eq!(m.roughness(), 0.3);
    assert_eq!(m.emissive(), 1.5);
}

/// 🔴 The layout is declared four times and checked nowhere.
#[test]
fn every_shader_agrees_on_the_material_layout() {
    const SHADERS: [&str; 3] = [
        include_str!("../../shaders/material_pbr_compute.wgsl"),
        include_str!("../../shaders/material_pbr_default.wgsl"),
        include_str!("../../shaders/meshlet_deferred.wgsl"),
    ];
    // Rust's own, in declaration order.
    let expected = [
        "base_color",
        "metallic_roughness_emissive_pad",
        "texture_indices",
        "uv_scale_offset",
    ];

    for (index, source) in SHADERS.iter().enumerate() {
        let body = source
            .split("struct MaterialParams {")
            .nth(1)
            .unwrap_or_else(|| panic!("shader {index} declares no MaterialParams"))
            .split('}')
            .next()
            .expect("unterminated struct");
        let fields: Vec<&str> = body
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with("//") || line.is_empty() {
                    return None;
                }
                line.split(':').next().map(str::trim)
            })
            .collect();
        assert_eq!(
            fields, expected,
            "shader {index} declares MaterialParams as {fields:?}, which is not what \
             Rust writes into the buffer — every material after the first mismatched \
             field reads the next one's bytes",
        );
    }

    // Four vec4s. The assertion is on the number rather than on the
    // expression so a field added as a bare `f32` — which pads to 16
    // anyway and would keep the count right — still fails here.
    assert_eq!(std::mem::size_of::<MaterialParams>(), 64);
}

/// The texture transform reaches the GPU struct in the order the shader
/// unpacks it: `xy` scale, `zw` offset. Swapping the pairs is a silent
/// change of meaning — a tiled texture would slide instead.
#[test]
fn the_uv_transform_packs_scale_then_offset() {
    let params = MaterialParams::default().with_uv([4.0, 2.0], [0.25, 0.5]);
    assert_eq!(params.uv_scale_offset, [4.0, 2.0, 0.25, 0.5]);
}

/// And a material that says nothing tiles exactly once.
#[test]
fn the_default_transform_is_the_identity() {
    assert_eq!(
        crate::material::Material::default()
            .to_params()
            .uv_scale_offset,
        [1.0, 1.0, 0.0, 0.0],
    );
}

/// 🔴 Whatever tiles the coordinate must tile its derivatives too.
#[test]
fn tiling_scales_the_derivatives_too() {
    for (name, source) in [
        (
            "material_pbr_compute",
            include_str!("../../shaders/material_pbr_compute.wgsl"),
        ),
        (
            "material_pbr_default",
            include_str!("../../shaders/material_pbr_default.wgsl"),
        ),
    ] {
        let line = |prefix: &str| -> String {
            source
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with(prefix))
                .unwrap_or_else(|| panic!("{name} has no line starting `{prefix}`"))
                .to_owned()
        };

        let factor = line("let derivative_scale =");
        assert!(
            factor.contains("uv_scale_offset.xy"),
            "{name} computes `{factor}` — the coordinate is tiled and the derivatives \
             are not, so the mip is selected for a texture that is not the one being \
             sampled",
        );
        assert!(
            factor.contains("mip_bias_scale"),
            "{name} computes `{factor}` — without the bias the derivatives choose the \
             level the resolution suggests, not the one the upscaler can resolve",
        );

        for derivative in ["ddx_uv", "ddy_uv"] {
            let assignment = line(&format!("let {derivative} ="));
            assert!(
                assignment.contains("derivative_scale"),
                "{name} computes `{assignment}` — that derivative is unscaled while the \
                 other one is, so the two disagree about what a pixel covers",
            );
        }
    }
}
