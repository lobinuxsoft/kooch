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
fn texture_indices_round_trip_with_sentinel() {
    let m = MaterialParams::default().with_texture_indices(3, NO_TEXTURE, 7);
    assert_eq!(m.albedo_index(), 3);
    assert_eq!(m.normal_index(), NO_TEXTURE);
    assert_eq!(m.metal_roughness_index(), 7);
    assert_eq!(m.texture_indices[3], 0, "pad slot stays zero");
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
///
/// `MaterialParams` exists in Rust and in three WGSL files, all reading
/// the same storage buffer. Adding a field to two of the three compiles
/// perfectly: the shader that missed it reads every material at the
/// wrong stride, so material 3 gets material 2's bytes and the picture
/// shows the wrong material rather than anything that looks like a
/// layout bug.
///
/// Compares the field NAMES in order, which is what a stride mismatch
/// comes from, and the byte size against Rust's.
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
///
/// The default has to be the identity: every `.ron` in every project
/// written before this field existed elides it, and those materials must
/// look the way they looked.
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
///
/// `textureSampleGrad` picks the mip from how fast the uv moves between
/// neighbouring pixels. Tiling twenty times makes it move twenty times
/// faster, so handing it the untiled derivatives selects a level about
/// four steps too sharp — the aliasing the mip chain exists to remove,
/// on exactly the surfaces that asked for tiling.
///
/// Both shaders build one `derivative_scale` and multiply both
/// derivatives by it, so the test follows that: the factor has to carry
/// the material's tiling AND the mip bias (#881), and both derivatives
/// have to use it. A derivative left on the raw `surf.` value is the
/// bug.
///
/// ⚠️ This reads the shader as TEXT, which is a weak test and is here
/// anyway: nothing else in the suite fails when a multiply is dropped,
/// and the way it gets dropped is a copy-paste that keeps the
/// coordinate and forgets the lines under it. A GPU test that measured
/// the chosen mip would be better — `texture_mip_selection` is now that
/// test for the bias half.
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
