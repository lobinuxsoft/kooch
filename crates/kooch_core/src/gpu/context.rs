use wgpu::{
    Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, PipelineCache, Queue,
    RequestAdapterOptions, Surface, SurfaceConfiguration, SurfaceTarget, TextureFormat,
};

use crate::pipeline_cache;

use super::dlss::{DlssRuntime, DlssSupport, Sdk};
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
    /// What DLSS this process can run (#536). Always false in a build
    /// without the `dlss` feature.
    dlss: DlssSupport,
    /// The application-wide DLSS object, created once here because NGX
    /// wants exactly one per process.
    dlss_sdk: Option<Sdk>,
}

impl GpuContext {
    /// Creates a new GPU context for the given surface target.
    ///
    /// Accepts any type that implements `Into<SurfaceTarget<'static>>`, such as
    /// `Arc<winit::Window>`. This keeps kooch_core free of windowing dependencies.
    ///
    /// Uses `pollster::block_on` internally since the engine is synchronous.
    pub fn new(
        target: impl Into<SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        // 🔴 Not `Instance::new` directly: with the `dlss` feature on,
        // NGX registers its Vulkan instance extensions from inside this
        // call and cannot be told about them afterwards (#536).
        let (instance, mut dlss) = super::dlss::instance(InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });

        let surface = instance.create_surface(target)?;

        let adapter = pick_adapter(&instance, &surface)?;

        let info = adapter.get_info();
        tracing::info!(
            name = info.name,
            backend = ?info.backend,
            driver = info.driver,
            "GPU adapter selected"
        );

        let required_limits = elevated_compute_limits(&adapter);
        let required_features = required_engine_features(&adapter) | optional_features(&adapter);

        // Same reason as the instance: the DLSS device extensions are
        // registered while the device is being opened, or never.
        let (device, queue) = super::dlss::device(
            &adapter,
            &DeviceDescriptor {
                label: Some("kooch_device"),
                required_features,
                required_limits: required_limits.clone(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            },
            &required_limits,
            &mut dlss,
        )?;
        let dlss_sdk = super::dlss::sdk(&device, &mut dlss);

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
            present_mode: present_mode(),
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: frame_latency(),
        };

        surface.configure(&device, &surface_config);

        tracing::info!(
            ?format,
            ?surface_config.present_mode,
            latency = surface_config.desired_maximum_frame_latency,
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
            dlss,
            dlss_sdk,
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

    /// Whether the surface is presenting with vsync.
    #[inline]
    pub fn vsync(&self) -> bool {
        self.surface_config.present_mode == mode_for(true)
    }

    /// Switches vsync on or off, reconfiguring the surface when the mode
    /// actually changes. Returns whether it reconfigured.
    ///
    /// 🔴 Guarded on the current mode rather than called unconditionally.
    /// `configure` rebuilds the swapchain — it drops every image the
    /// surface holds — so doing it once a frame because a resource says
    /// the same thing it said last frame would rebuild it sixty times a
    /// second.
    ///
    /// Safe to call between frames, which is where its only caller runs.
    /// The same is true of [`Self::resize`], which does the same thing
    /// for a different field.
    pub fn set_vsync(&mut self, vsync: bool) -> bool {
        let wanted = mode_for(vsync);
        if self.surface_config.present_mode == wanted {
            return false;
        }
        self.surface_config.present_mode = wanted;
        self.surface.configure(&self.device, &self.surface_config);
        tracing::info!(?wanted, "present mode changed");
        true
    }

    /// Which DLSS features this process can run (#536).
    #[inline]
    pub fn dlss(&self) -> DlssSupport {
        self.dlss
    }

    /// The application-wide DLSS object, or `None` when this build has
    /// no DLSS or this adapter cannot run it.
    #[inline]
    pub fn dlss_sdk(&self) -> Option<&Sdk> {
        self.dlss_sdk.as_ref()
    }

    /// The DLSS handles a render pass needs, in a form it can keep for
    /// the frame this context is removed from `Resources`.
    #[inline]
    pub fn dlss_runtime(&self) -> DlssRuntime {
        DlssRuntime {
            adapter: self.adapter.clone(),
            sdk: self.dlss_sdk.clone(),
            support: self.dlss,
        }
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

/// How frames are presented. Vsync unless `KOOCH_PRESENT_MODE=novsync`.
///
/// # Why this needs to be switchable
///
/// With vsync on, a frame takes 16.67 ms at 60 Hz whatever the engine
/// does, because most of that number is waiting for the vblank. That
/// makes the frame-time readout useless for optimisation work: removing
/// 7.5 ms of real CPU work changed the HUD by nothing, since the time
/// simply moved from working to waiting (#691).
///
/// Vsync stays the default — it is what anyone editing or playing wants,
/// and an uncapped editor burns a GPU to draw frames nobody sees. The
/// variable exists so that measuring is possible at all.
///
/// `AutoNoVsync` rather than `Immediate`: it falls back to whatever the
/// surface actually supports instead of failing on a driver that has no
/// immediate mode.
/// How many frames the swapchain may have in flight.
/// `KOOCH_FRAME_LATENCY` overrides it, clamped to 1..=3.
///
/// # The measurement this existed for, and what it came back with
///
/// A 1165-frame capture on the OneXFly (#814) has the same GPU work
/// produce two different frames: 167 frames turned 33.5 ms of GPU into a
/// 34.7 ms frame, and 80 turned 34.7 ms into a 69.4 ms frame. Identical
/// load, and the bad outcome **exactly double**.
///
/// The explanation written here used to be a swapchain of two images:
/// under FIFO the compositor holds one while the GPU draws into the
/// other, so `get_current_texture` waits out the compositor's whole turn
/// instead of overlapping with it.
///
/// ❌ **Measured on the device, and it is wrong.** Three 30-second
/// captures of one binary, one variable each:
///
/// | | latency 2 | latency 3 | `novsync` |
/// |---|---|---|---|
/// | frame/GPU p80 | 1.98 | 1.99 | 1.94 |
/// | frame/GPU p90 | 2.49 | 2.50 | 2.21 |
///
/// A third image does not move the ratio by a hundredth, and neither
/// does leaving FIFO. Two things this leaves behind:
///
/// - **An acquire of ~35 ms against a GPU of ~35 ms is not a defect.**
///   Being GPU-bound means the CPU waits somewhere, and it waits here.
///   What is unexplained is only the tail — frames where the wait grows
///   by 50 ms while our GPU work grows by 2.
/// - **A present mode is close to decorative when a compositor owns the
///   display.** These captures run under gamescope, which composites on
///   the same GPU on its own schedule and is invisible to our scopes:
///   they time *our* passes. Whatever is left lives outside this
///   process, and no environment variable on this side will find it.
///
/// # Why 2 is still the default
///
/// Because a third image costs a frame of input lag — at 34 ms per frame
/// that is 34 ms of extra lag on a handheld, which is not a rounding
/// error — and it now has a measurement saying it buys nothing. Holding
/// the default while the mechanism was only plausible turned out to be
/// the right call: the lag would have been paid for no return.
fn frame_latency() -> u32 {
    let raw = std::env::var("KOOCH_FRAME_LATENCY").ok();
    let latency = latency_from(raw.as_deref());
    if raw.is_some() {
        tracing::info!("KOOCH_FRAME_LATENCY={latency}: swapchain frames in flight");
    }
    latency
}

/// Reads the variable, so the rule is testable without an environment.
///
/// Anything unparseable keeps the default: a typo in a measurement run
/// must not silently change what is being measured, and a panic here
/// would take the window with it.
pub(super) fn latency_from(raw: Option<&str>) -> u32 {
    raw.and_then(|v| v.trim().parse::<u32>().ok())
        .map(|latency| latency.clamp(1, 3))
        .unwrap_or(2)
}

fn present_mode() -> wgpu::PresentMode {
    mode_for(vsync_override().unwrap_or(true))
}

/// The present mode a surface gets for `vsync`.
///
/// `AutoNoVsync` rather than `Immediate`, and `AutoVsync` rather than
/// `Fifo`: the `Auto` pair falls back to whatever the surface actually
/// supports instead of failing on a driver that lacks the exact mode.
fn mode_for(vsync: bool) -> wgpu::PresentMode {
    match vsync {
        true => wgpu::PresentMode::AutoVsync,
        false => wgpu::PresentMode::AutoNoVsync,
    }
}

/// `KOOCH_PRESENT_MODE`, read once. `None` means the variable said
/// nothing, which is what lets the project's own setting stand.
///
/// 🔴 **`vsync` is a recognised value and not a no-op**, which it would
/// have been while this returned a mode rather than an opinion. Once
/// `.rendersettings` can turn vsync off, a run that needs it back on has
/// to be able to say so — and "unset" cannot mean that, because unset is
/// also what every ordinary launch looks like.
pub fn vsync_override() -> Option<bool> {
    static VSYNC: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *VSYNC.get_or_init(|| {
        let vsync = vsync_from(std::env::var("KOOCH_PRESENT_MODE").ok().as_deref());
        if vsync == Some(false) {
            tracing::info!(
                "KOOCH_PRESENT_MODE=novsync: frame times will show work rather than \
                 the wait for the vblank"
            );
        }
        vsync
    })
}

/// Reads the variable, so the rule is testable without an environment.
///
/// Anything unrecognised is `None` rather than a guess: a typo during a
/// measurement run must not silently decide how frames are presented,
/// and must not silently override the author's choice either.
pub(super) fn vsync_from(raw: Option<&str>) -> Option<bool> {
    match raw.map(str::trim) {
        Some("novsync") => Some(false),
        Some("vsync") => Some(true),
        _ => None,
    }
}

/// The adapter the engine can actually run on.
///
/// # 🔴 Why asking for "the best one" is not enough
///
/// `request_adapter` answers with the most powerful adapter, not the
/// most capable one, and those differ. Under Proton the same Radeon
/// 890M is visible **twice**: once through Vulkan and once through DX12,
/// which is vkd3d translating to Vulkan underneath. DX12 wins the
/// preference and does not expose `SHADER_F16`, so the engine's feature
/// assert killed the game 571 ms after launch — on a machine whose GPU
/// supports every feature it needs, through the other door (#963).
///
/// So: take the preferred adapter when it suits, and otherwise look for
/// one that does before giving up. The order is deliberate — a desktop
/// with an integrated and a discrete GPU keeps getting the discrete one,
/// because the preferred adapter is tried first and almost always fits.
fn pick_adapter(
    instance: &Instance,
    surface: &wgpu::Surface<'static>,
) -> Result<Adapter, GpuError> {
    let preferred = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(surface),
        force_fallback_adapter: false,
    }))
    .map_err(GpuError::NoAdapter)?;

    if super::features::suits_engine(&preferred) {
        return Ok(preferred);
    }

    // Said out loud, because "the engine picked a different GPU than you
    // expected" is otherwise invisible, and because the reason names the
    // exact features that ruled the first one out.
    let info = preferred.get_info();
    let missing = super::features::engine_features() - preferred.features();
    tracing::warn!(
        name = info.name,
        backend = ?info.backend,
        ?missing,
        "the preferred adapter cannot run the engine; looking for one that can",
    );

    let usable = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .into_iter()
        .find(|adapter| {
            adapter.is_surface_supported(surface) && super::features::suits_engine(adapter)
        });

    match usable {
        Some(adapter) => {
            let info = adapter.get_info();
            tracing::info!(
                name = info.name,
                backend = ?info.backend,
                "using this adapter instead",
            );
            Ok(adapter)
        }
        // Nothing on the machine suits. Hand back the preferred one so
        // the feature assert reports what is missing, rather than
        // failing here with a vaguer message about no adapter at all.
        None => Ok(preferred),
    }
}
