use wgpu::{
    Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, PipelineCache, Queue,
    RequestAdapterOptions, Surface, SurfaceConfiguration, SurfaceTarget, TextureFormat,
};

use crate::pipeline_cache;

use super::error::GpuError;
use super::features::{optional_features, required_engine_features};
use super::limits::elevated_compute_limits;

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
