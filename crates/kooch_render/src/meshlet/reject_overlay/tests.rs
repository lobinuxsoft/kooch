use super::*;

#[test]
fn overlay_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
        .expect("meshlet_reject_overlay.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("meshlet_reject_overlay.wgsl should validate");
}

#[test]
fn overlay_params_layout_is_pod() {
    // 64-byte mat4 + 8-byte vec2u + 4-byte selected + 4-byte
    // thickness = 80 B. Multiple of 16 — std140-friendly.
    assert_eq!(std::mem::size_of::<OverlayParams>(), 80);
}

#[test]
fn reject_reason_discriminants_match_shader() {
    // The cull shader's `REJECT_REASON_*` constants pin these
    // values. Reordering breaks the overlay's mode → reason
    // lookup silently — test fails first.
    assert_eq!(RejectReason::Skipped as u32, 0);
    assert_eq!(RejectReason::Passed as u32, 1);
    assert_eq!(RejectReason::Frustum as u32, 2);
    assert_eq!(RejectReason::Backface as u32, 3);
    assert_eq!(RejectReason::HiZ as u32, 4);
    assert_eq!(RejectReason::Lod as u32, 5);
}
