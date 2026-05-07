//! GPU context initialization via wgpu.
//!
//! [`GpuContext`] is the central GPU infrastructure for the engine,
//! holding the wgpu Instance, Adapter, Device, Queue, and Surface.
//!
//! # Example
//! ```ignore
//! // GpuContext is created automatically by WindowPlugin during resumed().
//! // Access it as a resource in any system:
//! fn my_system(resources: &mut Resources) {
//!     if let Some(gpu) = resources.get::<GpuContext>() {
//!         tracing::info!("Using GPU: {:?}", gpu.adapter_info());
//!     }
//! }
//! ```

use wgpu::{
    Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, PipelineCache, Queue,
    RequestAdapterOptions, Surface, SurfaceConfiguration, SurfaceTarget, TextureFormat,
};

use crate::pipeline_cache;

/// Errors that can occur during GPU context initialization.
#[derive(Debug)]
pub enum GpuError {
    /// No suitable GPU adapter was found.
    NoAdapter(wgpu::RequestAdapterError),
    /// Failed to create the wgpu surface.
    Surface(wgpu::CreateSurfaceError),
    /// Failed to request a GPU device.
    Device(wgpu::RequestDeviceError),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "no suitable GPU adapter found: {e}"),
            Self::Surface(e) => write!(f, "failed to create surface: {e}"),
            Self::Device(e) => write!(f, "failed to request device: {e}"),
        }
    }
}

impl std::error::Error for GpuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoAdapter(e) => Some(e),
            Self::Surface(e) => Some(e),
            Self::Device(e) => Some(e),
        }
    }
}

impl From<wgpu::CreateSurfaceError> for GpuError {
    fn from(e: wgpu::CreateSurfaceError) -> Self {
        Self::Surface(e)
    }
}

impl From<wgpu::RequestDeviceError> for GpuError {
    fn from(e: wgpu::RequestDeviceError) -> Self {
        Self::Device(e)
    }
}

/// Central GPU context holding all wgpu state.
///
/// Created during window initialization and stored as a [`Resource`](crate::resource::Resources).
/// Provides access to the GPU device, queue, surface, and configuration needed
/// for rendering and compute operations.
pub struct GpuContext {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    pipeline_cache: Option<PipelineCache>,
}

impl GpuContext {
    /// Creates a new GPU context for the given surface target.
    ///
    /// Accepts any type that implements `Into<SurfaceTarget<'static>>`, such as
    /// `Arc<winit::Window>`. This keeps ome_core free of windowing dependencies.
    ///
    /// Uses `pollster::block_on` internally since the engine is synchronous.
    pub fn new(
        target: impl Into<SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        let instance = Instance::new(InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });

        let surface = instance.create_surface(target)?;

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(GpuError::NoAdapter)?;

        let info = adapter.get_info();
        tracing::info!(
            name = info.name,
            backend = ?info.backend,
            driver = info.driver,
            "GPU adapter selected"
        );

        let required_limits = elevated_compute_limits(&adapter);
        let required_features = required_engine_features(&adapter) | optional_features(&adapter);

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("ome_device"),
            required_features,
            required_limits,
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))?;

        let pipeline_cache = pipeline_cache::load(&device, &adapter);

        // Log uncaptured device errors (recovery deferred to post-v0.1).
        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            tracing::error!("wgpu device error: {error}");
        }));

        let surface_caps = surface.get_capabilities(&adapter);

        // Prefer non-sRGB format (e.g., Bgra8Unorm) since egui and most
        // renderers handle gamma correction in the shader. sRGB surfaces
        // double-apply gamma.
        let format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_config);

        tracing::info!(
            ?format,
            ?surface_config.present_mode,
            "Surface configured ({width}x{height})"
        );

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            pipeline_cache,
        })
    }

    /// Reconfigures the surface to match the new window dimensions.
    ///
    /// Ignores zero-sized dimensions (e.g. minimized windows).
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Returns a reference to the wgpu [`Instance`].
    #[inline]
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// Returns a reference to the GPU [`Adapter`].
    #[inline]
    pub fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    /// Returns adapter info (name, backend, driver).
    #[inline]
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Returns a reference to the GPU [`Device`].
    #[inline]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Returns a reference to the GPU [`Queue`].
    #[inline]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Returns a reference to the [`Surface`].
    #[inline]
    pub fn surface(&self) -> &Surface<'static> {
        &self.surface
    }

    /// Returns the current surface [`TextureFormat`].
    #[inline]
    pub fn format(&self) -> TextureFormat {
        self.surface_config.format
    }

    /// Returns the current surface dimensions as `(width, height)`.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    /// Returns the shared pipeline cache, if one was created for this adapter.
    ///
    /// `None` on adapters that lack [`wgpu::Features::PIPELINE_CACHE`] (Metal,
    /// GL, WebGPU). Callers passing this into pipeline descriptors should fall
    /// back to `cache: None` in that case — both paths are correct.
    #[inline]
    pub fn pipeline_cache(&self) -> Option<&PipelineCache> {
        self.pipeline_cache.as_ref()
    }
}

impl Drop for GpuContext {
    fn drop(&mut self) {
        if let Some(cache) = &self.pipeline_cache
            && let Err(e) = pipeline_cache::save(cache, &self.adapter)
        {
            tracing::warn!(error = %e, "failed to persist pipeline cache on shutdown");
        }
    }
}

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
fn required_engine_features(adapter: &Adapter) -> wgpu::Features {
    // FLOAT32_FILTERABLE is required by PR-4 of epic #370: the GDF
    // cascade-0 storage texture is `R32Float` (no native R16Float
    // STORAGE_BINDING in wgpu 29 / WebGPU core), and the production
    // raymarch fragment shader samples it with a linear sampler so
    // sub-voxel ray-march steps see a smooth SDF instead of nearest-
    // neighbour stair-stepping. RX 9070 XT, the target dev HW, and
    // the Steam Deck APU all advertise this feature; raising it as a
    // hard-required surfaces unsupported HW at startup rather than a
    // crash mid-frame inside `create_bind_group`.
    let required = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::FLOAT32_FILTERABLE;
    let missing = required - adapter.features();
    assert!(
        missing.is_empty(),
        "GPU adapter is missing required features for oh_my_engine: {missing:?}. \
         #136 S6 — sparse SDF storage needs R16Float storage textures, \
         which requires TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES on the adapter. \
         PR-4 of epic #370 — GDF cascade fetch needs FLOAT32_FILTERABLE for \
         linear-sampled R32Float textures."
    );
    required
}

/// Requests optional features whose absence would silently degrade the engine
/// (pipeline cache, BVH-build telemetry timestamps, ...), falling back to
/// `empty()` when unsupported so cross-backend builds keep working.
///
/// `TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_PASSES` enable per-pass GPU
/// profiling in the LBVH builder ([`ome_bvh::BvhGpuBuilder`]); the builder
/// stays correct on adapters that don't expose them by skipping the
/// `timestamp_writes` calls (see #333 — without this opt-in, adapters that
/// support timestamps would never get the telemetry, even though the
/// builder is ready to record it).
fn optional_features(adapter: &Adapter) -> wgpu::Features {
    let mut features = wgpu::Features::empty();
    if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        features |= wgpu::Features::PIPELINE_CACHE;
    }
    if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        && adapter
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    {
        features |= wgpu::Features::TIMESTAMP_QUERY
            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
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

/// Targets from wgpu capabilities audit §C.1: SSAO tiles (8×8×16 = 1024 invocations)
/// and bloom downsample (~32 KiB LDS) need more than wgpu defaults of 256 / 16 KiB.
/// Clamped against adapter-reported limits so older GPUs fall back gracefully.
const TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP: u32 = 1024;
const TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE: u32 = 32_768;
/// Hi-Z SPD (#486) binds 12 storage texture slots in one bind
/// group. wgpu's default `max_storage_textures_per_shader_stage` is
/// 4, so the SPD pipeline-layout creation rejects without raising
/// it. Most desktop GPUs (RX 9070 XT included) advertise ≥ 16.
const TARGET_MAX_STORAGE_TEXTURES_PER_STAGE: u32 = 16;
/// The atomic R64 visibility-buffer raster pipeline (#493) uses bind
/// groups 0..4 (camera, meshlet pool, visible_meshlets, instances,
/// vbuf64). wgpu's default `max_bind_groups` is 4 (group indices 0..3),
/// so the BGL creation rejects without raising it. RDNA 2+ desktop /
/// handheld + DX12 + Metal all advertise ≥ 8.
const TARGET_MAX_BIND_GROUPS: u32 = 5;

fn elevated_compute_limits(adapter: &Adapter) -> wgpu::Limits {
    let adapter_limits = adapter.limits();

    let invocations = TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP
        .min(adapter_limits.max_compute_invocations_per_workgroup);
    let storage = TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE
        .min(adapter_limits.max_compute_workgroup_storage_size);
    let storage_textures = TARGET_MAX_STORAGE_TEXTURES_PER_STAGE
        .min(adapter_limits.max_storage_textures_per_shader_stage);
    let bind_groups = TARGET_MAX_BIND_GROUPS.min(adapter_limits.max_bind_groups);

    if invocations < TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP {
        tracing::warn!(
            requested = TARGET_MAX_COMPUTE_INVOCATIONS_PER_WORKGROUP,
            granted = invocations,
            "adapter clamped max_compute_invocations_per_workgroup; compute-heavy passes (SSAO, bloom) may run degraded"
        );
    }
    if storage < TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE {
        tracing::warn!(
            requested = TARGET_MAX_COMPUTE_WORKGROUP_STORAGE_SIZE,
            granted = storage,
            "adapter clamped max_compute_workgroup_storage_size; tile-based compute passes may need smaller tiles"
        );
    }
    if storage_textures < TARGET_MAX_STORAGE_TEXTURES_PER_STAGE {
        tracing::warn!(
            requested = TARGET_MAX_STORAGE_TEXTURES_PER_STAGE,
            granted = storage_textures,
            "adapter clamped max_storage_textures_per_shader_stage; Hi-Z SPD pyramid build (#486) requires ≥ 12"
        );
    }
    if bind_groups < TARGET_MAX_BIND_GROUPS {
        tracing::warn!(
            requested = TARGET_MAX_BIND_GROUPS,
            granted = bind_groups,
            "adapter clamped max_bind_groups; atomic R64 vbuf raster (#493) requires ≥ 5"
        );
    }

    wgpu::Limits {
        max_compute_invocations_per_workgroup: invocations,
        max_compute_workgroup_storage_size: storage,
        max_storage_textures_per_shader_stage: storage_textures,
        max_bind_groups: bind_groups,
        ..wgpu::Limits::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Display/Debug/Error::source impls for `GpuError` are exhaustive
    // match arms over wgpu-typed payloads, so their correctness is
    // proven by compilation. Synthetic-instance tests removed when wgpu
    // 29 made `RequestAdapterError` non-constructible from user code
    // (issue #218). The headless smoke test below still exercises the
    // happy path with a real adapter.

    #[test]
    #[ignore] // Requires GPU hardware.
    fn create_adapter_headless() {
        let instance = Instance::new(InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }));

        assert!(adapter.is_ok(), "expected a GPU adapter to be available");

        let info = adapter.unwrap().get_info();
        println!("Adapter: {} ({:?})", info.name, info.backend);
    }
}
