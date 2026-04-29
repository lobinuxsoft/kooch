//! CPU/naga checks for the lookup WGSL — parse + validate the body
//! against both layout shapes consumers will use, plus a defensive
//! grep guarding the host-side mirrors of the WGSL constants. No GPU
//! required.

use super::super::{
    LOOKUP_BODY_WGSL, LOOKUP_DEFAULT_GROUP, LOOKUP_DEFAULT_MASK_BINDING,
    LOOKUP_DEFAULT_POOL_BINDINGS, LOOKUP_DEFAULT_ROOT_BINDING,
    LOOKUP_DEFAULT_SAMPLER_BINDING, LOOKUP_DEFAULT_UNIFORM_BINDING, lookup_wgsl,
};
use super::harness::PROBE_HARNESS_WGSL;
use crate::sparse::{LOD_COUNT, ROOT_DIM};

#[test]
fn lookup_body_with_default_layout_parses_and_validates() {
    let combined = format!(
        "{}{}",
        lookup_wgsl(
            LOOKUP_DEFAULT_GROUP,
            LOOKUP_DEFAULT_ROOT_BINDING,
            LOOKUP_DEFAULT_POOL_BINDINGS,
            LOOKUP_DEFAULT_UNIFORM_BINDING,
            LOOKUP_DEFAULT_SAMPLER_BINDING,
            LOOKUP_DEFAULT_MASK_BINDING,
        ),
        PROBE_HARNESS_WGSL,
    );
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("default lookup layout should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("default lookup layout should validate");
}

#[test]
fn lookup_body_with_alternative_layout_validates() {
    // Raymarcher-style override — single bind group with the lookup
    // globals slotted in 8/9/10/11/12/13/14/15 alongside other resources.
    // Assemble a stand-alone shim that exercises the lookup body.
    let shim = r#"
@compute @workgroup_size(1)
fn shim_main() {
    _ = sparse_sdf_lookup(vec3<f32>(0.0, 0.0, 0.0), 1.0);
}
"#;
    let combined = format!("{}{}", lookup_wgsl(0, 8, [9, 10, 11, 12], 13, 14, 15), shim);
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("alternative lookup layout should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("alternative lookup layout should validate");
}

#[test]
fn lookup_wgsl_constants_match_host() {
    assert!(
        LOOKUP_BODY_WGSL.contains(&format!("LOOKUP_LOD_COUNT: u32 = {LOD_COUNT}u")),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains(&format!("LOOKUP_ROOT_DIM: u32 = {ROOT_DIM}u")),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains("LOOKUP_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu"),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains("LOOKUP_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu"),
    );
}
