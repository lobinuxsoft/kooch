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
    Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, Queue, RequestAdapterOptions,
    Surface, SurfaceConfiguration, SurfaceTarget, TextureFormat,
};

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

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("ome_device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))?;

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
