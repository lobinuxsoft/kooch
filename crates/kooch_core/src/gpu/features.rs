use wgpu::Adapter;

/// Hard-required engine features. Panics with a clear message if the adapter does not expose them —
/// none of these are optional and the engine cannot start without each.
pub fn engine_features() -> wgpu::Features {
    // FLOAT32_FILTERABLE is required by PR-4 of epic #370.
    wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::FLOAT32_FILTERABLE
        | wgpu::Features::SHADER_F16
}

/// Everything a device must expose for the whole engine to run: the hard-required set plus the
/// meshlet path's atomic bundle.
pub fn all_required_features() -> wgpu::Features {
    engine_features() | vbuf64_features()
}

/// Whether this adapter carries everything the engine hard-requires.
pub(super) fn suits_engine(adapter: &Adapter) -> bool {
    (engine_features() - adapter.features()).is_empty()
}

/// Asserts the adapter carries [`engine_features`], with a message that
/// names why each one is there.
pub(super) fn required_engine_features(adapter: &Adapter) -> wgpu::Features {
    let required = engine_features();
    let missing = required - adapter.features();
    assert!(
        missing.is_empty(),
        "GPU adapter is missing required features for kooch: {missing:?}. \
         #136 S6 — sparse SDF storage needs R16Float storage textures, \
         which requires TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES on the adapter. \
         PR-4 of epic #370 — GDF cascade fetch needs FLOAT32_FILTERABLE for \
         linear-sampled R32Float textures. \
         #481 — FSR 3.1's half-precision path needs SHADER_F16."
    );
    required
}

/// Requests optional features whose absence would silently degrade the engine (pipeline cache,
/// BVH-build telemetry timestamps, ...), falling back to `empty()` when unsupported so
/// cross-backend builds keep working.
pub(super) fn optional_features(adapter: &Adapter) -> wgpu::Features {
    let mut features = wgpu::Features::empty();
    if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        features |= wgpu::Features::PIPELINE_CACHE;
    }
    // depth range can hug the slice it covers. Without it the near plane has to sit a cascade width
    // further back to catch occluders outside the view frustum, and that whole margin is precision
    // the depth comparison never gets.
    if adapter
        .features()
        .contains(wgpu::Features::DEPTH_CLIP_CONTROL)
    {
        features |= wgpu::Features::DEPTH_CLIP_CONTROL;
    }
    // the one page it was paired with. Without this it does that in a fragment shader with
    // `discard`, which pays twice: the out-of-rect fragments are rasterised before they are thrown
    // away, and the `discard` disables early-Z for the whole pass.
    if adapter.features().contains(wgpu::Features::CLIP_DISTANCES) {
        features |= wgpu::Features::CLIP_DISTANCES;
    }
    // 🔴 Without it wgpu's indirect validation drops every draw whose `first_instance` is not 0, in
    // silence: the transparent tail drew only its first material (#452).
    if adapter
        .features()
        .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
    {
        features |= wgpu::Features::INDIRECT_FIRST_INSTANCE;
    }
    if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        && adapter
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    {
        features |= wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
    }
    // MeshletGpuTimers in the meshlet render stage) requires this separate feature in wgpu 29.
    // Without it the encoder validates and the queue submission fails with "Features
    // TIMESTAMP_QUERY_INSIDE_ENCODERS are required but not enabled".
    if adapter
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    {
        features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    }
    // interdependent features (you cannot atomicMax a u64 storage texture without int64 in the
    // shader, and you cannot store the u64 atomic at all without TEXTURE_INT64_ATOMIC).
    let vbuf64 = vbuf64_features();
    if adapter.features().contains(vbuf64) {
        features |= vbuf64;
        tracing::info!(
            "vbuf64 features available — atomic R64 visibility buffer path enabled \
             (TEXTURE_INT64_ATOMIC + SHADER_INT64 + SHADER_INT64_ATOMIC_MIN_MAX)"
        );
    } else {
        let missing = vbuf64 - adapter.features();
        tracing::info!(
            ?missing,
            "vbuf64 features unavailable — meshlet visibility buffer will use R32Uint fallback \
             (coplanar meshlets may z-fight)"
        );
    }
    // #452 — the transparent layers insert with a 64-bit atomic whose result they read, which the
    // min/max feature does not allow. Without it, transparency falls back to the sorted pass.
    if features.contains(vbuf64)
        && adapter
            .features()
            .contains(wgpu::Features::SHADER_INT64_ATOMIC_ALL_OPS)
    {
        features |= wgpu::Features::SHADER_INT64_ATOMIC_ALL_OPS;
    }
    // modes (TriangleDensity, Overdraw, reject overlays). This is broader than the full vbuf64
    // bundle: many baseline adapters (RDNA 2 without INT64 atomic, Adreno X1) expose TEXTURE_ATOMIC
    // standalone.
    if adapter.features().contains(wgpu::Features::TEXTURE_ATOMIC)
        && !features.contains(wgpu::Features::TEXTURE_ATOMIC)
    {
        features |= wgpu::Features::TEXTURE_ATOMIC;
        tracing::info!(
            "TEXTURE_ATOMIC available standalone — advanced debug modes enabled (R32Uint atomic)"
        );
    }
    features
}

/// Returns the feature bundle required for the Bevy-style atomic R64 visibility buffer (#493). All
/// four flags must be present together; any one missing forces the legacy `R32Uint` fallback path
/// in the meshlet render stage.
pub fn vbuf64_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
}
