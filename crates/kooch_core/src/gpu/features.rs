use wgpu::Adapter;

/// Hard-required engine features. Panics with a clear message if the
/// adapter does not expose them — none of these are optional and the
/// engine cannot start without each.
///
/// - `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`: needed for the sparse
///   SDF subgrid pool (issue #136 S6) which lives in a
///   `texture_storage_3d<r16float, write>` atlas. Without this flag
///   wgpu refuses storage access on `R16Float`. RDNA 2/4 (Steam Deck,
///   RX 9070 XT) supports it natively over Vulkan; DX12 / Metal also
///   expose it on contemporary hardware.
pub fn engine_features() -> wgpu::Features {
    // FLOAT32_FILTERABLE is required by PR-4 of epic #370: the GDF
    // cascade-0 storage texture is `R32Float` (no native R16Float
    // STORAGE_BINDING in wgpu 29 / WebGPU core), and the production
    // raymarch fragment shader samples it with a linear sampler so
    // sub-voxel ray-march steps see a smooth SDF instead of nearest-
    // neighbour stair-stepping. RX 9070 XT, the target dev HW, and
    // the Steam Deck APU all advertise this feature; raising it as a
    // hard-required surfaces unsupported HW at startup rather than a
    // crash mid-frame inside `create_bind_group`.
    //
    // SHADER_F16 is FSR 3.1's `FFX_HALF` path (#481). The accumulation
    // carries 25 colours per output pixel through YCoCg, a tonemap round
    // trip and a variance box; in half that is half the registers, and
    // on a 10 W part registers are occupancy and occupancy is latency
    // hiding. It is `VK_KHR_shader_float16_int8` — present on RADV
    // STRIX1 (the OneXFly's 890M) and on gfx1201, and on anything that
    // also carries the 64-bit texture atomics the meshlet path already
    // demands, which are far rarer.
    wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::FLOAT32_FILTERABLE
        | wgpu::Features::SHADER_F16
}

/// Everything a device must expose for the whole engine to run: the
/// hard-required set plus the meshlet path's atomic bundle.
///
/// 🔴 **The one list.** This existed in seven places — the engine, the
/// vbuf64 gate, and five test files that each spelled it out again —
/// and adding `SHADER_F16` meant remembering all seven. Forgetting one
/// does not fail to compile: it makes the device request come back
/// short, the test skip with "no adapter", and the reader conclude the
/// machine lacks the hardware rather than the list lacks a line.
pub fn all_required_features() -> wgpu::Features {
    engine_features() | vbuf64_features()
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

/// Requests optional features whose absence would silently degrade the engine
/// (pipeline cache, BVH-build telemetry timestamps, ...), falling back to
/// `empty()` when unsupported so cross-backend builds keep working.
///
/// `TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_PASSES` enable per-pass GPU
/// profiling in the removed LBVH builder; that builder
/// stays correct on adapters that don't expose them by skipping the
/// `timestamp_writes` calls (see #333 — without this opt-in, adapters that
/// support timestamps would never get the telemetry, even though the
/// builder is ready to record it).
pub(super) fn optional_features(adapter: &Adapter) -> wgpu::Features {
    let mut features = wgpu::Features::empty();
    if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        features |= wgpu::Features::PIPELINE_CACHE;
    }
    // #476 — the shadow pass wants `unclipped_depth` so a cascade's
    // depth range can hug the slice it covers. Without it the near plane
    // has to sit a cascade width further back to catch occluders outside
    // the view frustum, and that whole margin is precision the depth
    // comparison never gets. Bevy renders their shadow pass with it and
    // emulates it in the fragment shader where it is missing.
    if adapter
        .features()
        .contains(wgpu::Features::DEPTH_CLIP_CONTROL)
    {
        features |= wgpu::Features::DEPTH_CLIP_CONTROL;
    }
    // #952 — the virtual shadow pages' depth pass clips each triangle to
    // the one page it was paired with. Without this it does that in a
    // fragment shader with `discard`, which pays twice: the out-of-rect
    // fragments are rasterised before they are thrown away, and the
    // `discard` disables early-Z for the whole pass. With it the clipper
    // cuts the triangle before any fragment exists and the pass carries
    // no fragment shader at all. See `page_depth_clipped.wgsl`.
    if adapter.features().contains(wgpu::Features::CLIP_DISTANCES) {
        features |= wgpu::Features::CLIP_DISTANCES;
    }
    if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        && adapter
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    {
        features |= wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
    }
    // #463.4 — `encoder.write_timestamp` (called between passes by
    // MeshletGpuTimers in the meshlet render stage) requires this
    // separate feature in wgpu 29. Without it the encoder validates
    // and the queue submission fails with "Features
    // TIMESTAMP_QUERY_INSIDE_ENCODERS are required but not enabled".
    if adapter
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    {
        features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    }
    // #493 — Bevy-style atomic R64 visibility buffer requires three
    // interdependent features (you cannot atomicMax a u64 storage
    // texture without int64 in the shader, and you cannot store the
    // u64 atomic at all without TEXTURE_INT64_ATOMIC). Request the
    // bundle as all-or-nothing; the meshlet stage falls back to the
    // legacy R32Uint vbuf when any of the three is missing.
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
    // #454 — R32Uint atomic storage textures back the advanced debug
    // modes (TriangleDensity, Overdraw, reject overlays). This is
    // broader than the full vbuf64 bundle: many baseline adapters
    // (RDNA 2 without INT64 atomic, Adreno X1) expose TEXTURE_ATOMIC
    // standalone. Pick it up independently so the debug pipeline
    // lights up on every adapter that can actually run it, not just
    // those that also have the four-flag int64 atomic bundle.
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

/// Returns the feature bundle required for the Bevy-style atomic R64
/// visibility buffer (#493). All four flags must be present together;
/// any one missing forces the legacy `R32Uint` fallback path in the
/// meshlet render stage.
///
/// - `TEXTURE_ATOMIC` gates `StorageTextureAccess::Atomic` for any
///   format (the validation error message names
///   `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` but the actual gate is
///   `TEXTURE_ATOMIC` in wgpu 29).
/// - `TEXTURE_INT64_ATOMIC` adds the R64 format on top.
/// - `SHADER_INT64` enables `u64` in the shader.
/// - `SHADER_INT64_ATOMIC_MIN_MAX` enables `textureAtomicMax` on `u64`.
pub fn vbuf64_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
}
