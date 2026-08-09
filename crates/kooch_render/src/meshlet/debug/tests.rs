use super::*;

#[test]
fn off_is_zero() {
    assert_eq!(MeshletDebugMode::Off.as_u32(), 0);
    assert_eq!(MeshletDebugMode::default(), MeshletDebugMode::Off);
}

/// WGSL cannot import a Rust constant, so `inti_debug.wgsl` declares
/// the discriminants itself and this test is the only thing holding the
/// two ends together — renumber a variant and the dropdown silently
/// selects a different view from the one it names.
///
/// It reads the shader text rather than restating the numbers: a copy of
/// the literals here would go stale in exactly the same way as the copy
/// in the shader, and agree with nothing.
#[test]
fn discriminants_match_the_shader() {
    let source = kooch_lighting::inti_debug_shader();
    for (mode, name) in [
        (MeshletDebugMode::Normals, "INTI_DEBUG_NORMALS"),
        (
            MeshletDebugMode::ShadowCascades,
            "INTI_DEBUG_SHADOW_CASCADES",
        ),
        (
            MeshletDebugMode::ContactShadows,
            "INTI_DEBUG_CONTACT_SHADOWS",
        ),
        (MeshletDebugMode::SingleLight, "INTI_DEBUG_SINGLE_LIGHT"),
    ] {
        let declaration = format!("const {name}: u32 = {}u;", mode.as_u32());
        assert!(
            source.contains(&declaration),
            "inti_debug.wgsl should declare `{declaration}` for {mode:?}",
        );
    }
}

/// The shader dispatches on `mode >= INTI_DEBUG_FIRST`, so a mode below
/// that boundary never reaches `inti_debug_view` no matter what the
/// dropdown says.
#[test]
fn every_inti_view_is_above_the_dispatch_floor() {
    let floor = MeshletDebugMode::Normals.as_u32();
    for mode in [
        MeshletDebugMode::ShadowCascades,
        MeshletDebugMode::ContactShadows,
        MeshletDebugMode::SingleLight,
    ] {
        assert!(mode.as_u32() >= floor, "{mode:?} is below INTI_DEBUG_FIRST");
    }
}

#[test]
fn normals_is_selectable_on_every_device() {
    // It reads no atomic texture — it is the old production path.
    assert!(!MeshletDebugMode::Normals.needs_texture_atomic());
    assert!(MeshletDebugMode::all_implemented().contains(&MeshletDebugMode::Normals));
}

#[test]
fn needs_texture_atomic_covers_advanced_modes() {
    assert!(MeshletDebugMode::TriangleDensity.needs_texture_atomic());
    assert!(MeshletDebugMode::Overdraw.needs_texture_atomic());
    assert!(MeshletDebugMode::HiZRejected.needs_texture_atomic());
    assert!(MeshletDebugMode::BackfaceRejected.needs_texture_atomic());
    assert!(MeshletDebugMode::FrustumRejected.needs_texture_atomic());
    // Baseline-safe modes never lift the atomic feature gate.
    assert!(!MeshletDebugMode::Off.needs_texture_atomic());
    assert!(!MeshletDebugMode::MeshletIds.needs_texture_atomic());
    assert!(!MeshletDebugMode::InstanceIds.needs_texture_atomic());
    assert!(!MeshletDebugMode::CullPassthrough.needs_texture_atomic());
    assert!(!MeshletDebugMode::OnlyLod0.needs_texture_atomic());
    assert!(!MeshletDebugMode::OnlyRoots.needs_texture_atomic());
}

#[test]
fn all_available_with_caps_filters_atomic_modes() {
    // Conservative caps (texture_atomic missing): only the
    // baseline-safe subset of `all_implemented()` survives.
    let no_atomic = MeshletDebugCaps::from_flags(false);
    let filtered = MeshletDebugMode::all_available_with_caps(&no_atomic);
    for mode in &filtered {
        assert!(
            !mode.needs_texture_atomic(),
            "{mode:?} leaked through the filter without atomic support",
        );
    }
    // With atomic support, the filter is identity over `all_implemented`.
    let with_atomic = MeshletDebugCaps::from_flags(true);
    let unfiltered = MeshletDebugMode::all_available_with_caps(&with_atomic);
    assert_eq!(unfiltered.len(), MeshletDebugMode::all_implemented().len());
}

/// The same predicate decides two things that look unrelated: whether
/// the cull shader records reject reasons, and whether the HUD has
/// per-stage survivor counts to show at all (#703).
///
/// A mode added later that measures the counters but returns `None`
/// here would read them and then hide them. One that returns `Some`
/// without the shader writing reasons would show four zeros. Both
/// failures are silent, which is why this is pinned rather than left
/// to the two call sites to agree.
#[test]
fn only_the_modes_that_measure_the_counters_report_them() {
    let measures = |mode: MeshletDebugMode| mode.reject_reason_code().is_some();

    // These three ask the cull shader to record, so the HUD gets rows.
    assert!(measures(MeshletDebugMode::FrustumRejected));
    assert!(measures(MeshletDebugMode::BackfaceRejected));
    assert!(measures(MeshletDebugMode::HiZRejected));

    // These do not — and the rows have to be absent rather than
    // showing whatever the last measuring frame happened to read.
    assert!(!measures(MeshletDebugMode::Off));
    assert!(!measures(MeshletDebugMode::TriangleDensity));
    assert!(!measures(MeshletDebugMode::Overdraw));
}

#[test]
fn reject_reason_code_tracks_cull_shader_constants() {
    // `REJECT_REASON_*` in meshlet_cull/atomic.wgsl pin these.
    // Reordering or renumbering breaks the overlay's match.
    assert_eq!(
        MeshletDebugMode::FrustumRejected.reject_reason_code(),
        Some(2)
    );
    assert_eq!(
        MeshletDebugMode::BackfaceRejected.reject_reason_code(),
        Some(3)
    );
    assert_eq!(MeshletDebugMode::HiZRejected.reject_reason_code(), Some(4));
    // Non-reject modes never write into reject_reasons[] — the
    // orchestrator must NOT lift `debug_active` for them.
    assert!(MeshletDebugMode::Off.reject_reason_code().is_none());
    assert!(
        MeshletDebugMode::TriangleDensity
            .reject_reason_code()
            .is_none()
    );
    assert!(MeshletDebugMode::Overdraw.reject_reason_code().is_none());
    assert!(
        MeshletDebugMode::CullPassthrough
            .reject_reason_code()
            .is_none()
    );
    assert!(MeshletDebugMode::OnlyLod0.reject_reason_code().is_none());
    assert!(MeshletDebugMode::OnlyRoots.reject_reason_code().is_none());
    assert!(MeshletDebugMode::MeshletIds.reject_reason_code().is_none());
    assert!(MeshletDebugMode::InstanceIds.reject_reason_code().is_none());
}

#[test]
fn discriminants_are_stable() {
    // GPU shader assumes these exact values. Reordering breaks
    // every active debug mode silently — flip this test first.
    assert_eq!(MeshletDebugMode::Off.as_u32(), 0);
    assert_eq!(MeshletDebugMode::MeshletIds.as_u32(), 1);
    assert_eq!(MeshletDebugMode::InstanceIds.as_u32(), 2);
    assert_eq!(MeshletDebugMode::TriangleDensity.as_u32(), 3);
    assert_eq!(MeshletDebugMode::Overdraw.as_u32(), 4);
    assert_eq!(MeshletDebugMode::HiZRejected.as_u32(), 5);
    assert_eq!(MeshletDebugMode::BackfaceRejected.as_u32(), 6);
    assert_eq!(MeshletDebugMode::CullPassthrough.as_u32(), 7);
    assert_eq!(MeshletDebugMode::OnlyLod0.as_u32(), 8);
    assert_eq!(MeshletDebugMode::OnlyRoots.as_u32(), 9);
    assert_eq!(MeshletDebugMode::FrustumRejected.as_u32(), 10);
    assert_eq!(MeshletDebugMode::Normals.as_u32(), 11);
    assert_eq!(MeshletDebugMode::ShadowCascades.as_u32(), 12);
    assert_eq!(MeshletDebugMode::ContactShadows.as_u32(), 13);
    assert_eq!(MeshletDebugMode::SingleLight.as_u32(), 14);
}
