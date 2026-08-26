//! NVIDIA DLSS Super Resolution, through `dlss_wgpu` (#536).
//!
//! The odd one out among the techniques in this directory. SGSR 2 and
//! FSR 3.1 are transliterated shaders the engine owns; DLSS is a neural
//! network shipped as a binary blob, reached by handing NVIDIA's SDK
//! wgpu's Vulkan images. That difference decides everything below:
//!
//! - 🔴 **A build can lack it.** Without the `dlss` cargo feature
//!   nothing linked the SDK, so this type still exists and reports
//!   [`Self::ready`] false. The one shape keeps `cfg` out of the stage.
//! - 🔴 **It produces a command buffer of its own**, which has to be
//!   submitted immediately after the frame's encoder. That is why
//!   [`Self::draw`] returns one instead of only a view.
//! - ⚠️ NVIDIA adapters, Vulkan only. Everything else falls back.

#[cfg(feature = "dlss")]
mod inner;

use kooch_core::gpu::DlssRuntime;

/// The DLSS backend, present in every build and useful in some.
pub(super) struct Dlss {
    #[cfg(feature = "dlss")]
    inner: inner::Inner,
    render_size: (u32, u32),
    output_size: (u32, u32),
}

impl Dlss {
    #[cfg_attr(not(feature = "dlss"), allow(unused_variables))]
    pub(super) fn new(device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) -> Self {
        Self {
            #[cfg(feature = "dlss")]
            inner: inner::Inner::new(device, output),
            render_size: render,
            output_size: output,
        }
    }

    #[cfg_attr(not(feature = "dlss"), allow(unused_variables))]
    pub(super) fn resize(&mut self, device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) {
        #[cfg(feature = "dlss")]
        self.inner.resize(device, output);
        self.render_size = render;
        self.output_size = output;
    }

    /// What NGX insists this frame be rendered at, once it has said so.
    ///
    /// 🔴 The engine's `render_scale` arithmetic is NOT the authority
    /// for this technique. NGX's minimum render resolution **is** its
    /// optimal, so a size that rounds one pixel below is refused
    /// outright — which is what a 943-row window halved does.
    pub(super) fn wanted_render_size(&self, output: (u32, u32)) -> Option<(u32, u32)> {
        let _ = output;
        #[cfg(feature = "dlss")]
        return self.inner.wanted_render_size(output);
        #[cfg(not(feature = "dlss"))]
        None
    }

    /// Whether DLSS has given up for this session, so the frame must go
    /// back to being rendered at the output's own size.
    pub(super) fn unusable(&self) -> bool {
        #[cfg(feature = "dlss")]
        return self.inner.unusable();
        #[cfg(not(feature = "dlss"))]
        true
    }

    /// The most recent resolve, for a test to read back.
    pub(super) fn resolved_texture(&self) -> Option<&wgpu::Texture> {
        #[cfg(feature = "dlss")]
        return Some(self.inner.output_texture());
        #[cfg(not(feature = "dlss"))]
        None
    }

    /// Runs DLSS and returns the view the rest of the frame reads, plus
    /// the command buffer the caller must submit right after its own.
    ///
    /// `None` when DLSS is not usable, which the caller answers by
    /// falling back rather than by presenting nothing.
    #[cfg_attr(not(feature = "dlss"), allow(unused_variables))]
    pub(super) fn draw<'a>(
        &'a self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        runtime: Option<&DlssRuntime>,
        inputs: &super::sgsr2::UpscaleInputs<'_>,
    ) -> Option<(&'a wgpu::TextureView, wgpu::CommandBuffer)> {
        #[cfg(feature = "dlss")]
        return self.inner.draw(
            device,
            queue,
            encoder,
            runtime?,
            inputs,
            self.render_size,
            self.output_size,
        );
        #[cfg(not(feature = "dlss"))]
        None
    }
}

/// Which of NVIDIA's presets a render scale asks for.
///
/// Named here rather than in `inner` so a build without the feature
/// still compiles the mapping and still tests it: the ladder is the
/// engine's decision, and the SDK's enum is only how it is spelled.
// The ladder is the engine's, so it is compiled and tested even where
// nothing consumes it — see the doc comment above.
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PerfMode {
    /// Anti-aliasing at the output's own resolution, no reconstruction.
    Dlaa,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

/// The preset a render-to-output ratio asks for.
///
/// The engine's ladder and NVIDIA's are the same ladder — 100 / 67 / 59
/// / 50 are DLAA, Quality, Balanced and Performance — so this is a
/// lookup, not a fit. The bands around each rung exist because a
/// project can type any number into `render_scale`, not because the
/// values in between mean anything.
#[cfg_attr(not(feature = "dlss"), allow(dead_code))]
pub(super) fn perf_mode(render: (u32, u32), output: (u32, u32)) -> PerfMode {
    let scale = if output.0 == 0 {
        100
    } else {
        (render.0 * 100) / output.0
    };
    match scale {
        0..=40 => PerfMode::UltraPerformance,
        41..=54 => PerfMode::Performance,
        55..=62 => PerfMode::Balanced,
        63..=85 => PerfMode::Quality,
        _ => PerfMode::Dlaa,
    }
}

#[cfg(test)]
mod tests;
