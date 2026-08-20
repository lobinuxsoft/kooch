//! Everything that names `dlss_wgpu`. Compiled only with the feature.

use std::sync::Mutex;

use dlss_wgpu::super_resolution::{
    DlssSuperResolution, DlssSuperResolutionExposure, DlssSuperResolutionRenderParameters,
};
use dlss_wgpu::{DlssFeatureFlags, DlssPerfQualityMode};
use kooch_core::gpu::DlssRuntime;

use super::{PerfMode, perf_mode};

/// What DLSS writes into.
///
/// `Rgba16Float` because it is the one format in wgpu's core set that
/// is both storage-writable and filterable — DLSS writes it as a
/// storage image and the passes after it sample it. Everything else
/// costs a copy or a second target.
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl PerfMode {
    fn as_sdk(self) -> DlssPerfQualityMode {
        match self {
            Self::Dlaa => DlssPerfQualityMode::Dlaa,
            Self::Quality => DlssPerfQualityMode::Quality,
            Self::Balanced => DlssPerfQualityMode::Balanced,
            Self::Performance => DlssPerfQualityMode::Performance,
            Self::UltraPerformance => DlssPerfQualityMode::UltraPerformance,
        }
    }
}

/// The flags this engine's inputs are described by.
///
/// Every one of them is a statement about our own buffers rather than a
/// preference: the motion vectors are written at render resolution, the
/// projection is infinite reversed-Z, and the colour DLSS reads is the
/// HDR radiance before the tonemap curve.
///
/// ⚠️ `AutoExposure` is upstream's choice and, for now, ours. The
/// engine knows its exposure scalar and could hand over a 1x1 texture
/// instead; that is a change to be judged on a screen with an NVIDIA
/// card, not asserted here.
fn feature_flags() -> DlssFeatureFlags {
    DlssFeatureFlags::LowResolutionMotionVectors
        | DlssFeatureFlags::InvertedDepth
        | DlssFeatureFlags::HighDynamicRange
        | DlssFeatureFlags::AutoExposure
}

struct State {
    context: Option<DlssSuperResolution>,
    /// What `context` was built for. A change to either rebuilds it,
    /// which is expensive and therefore not done per frame.
    built_for: Option<((u32, u32), PerfMode)>,
    /// Whether the next frame must throw the history away.
    reset: bool,
    /// 🔴 Sticky. A DLSS call that fails fails for a reason that does
    /// not change between frames, and retrying turns one error into
    /// sixty log lines a second.
    failed: bool,
}

pub(super) struct Inner {
    state: Mutex<State>,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
}

impl Inner {
    pub(super) fn new(device: &wgpu::Device, output: (u32, u32)) -> Self {
        let (texture, view) = create_output(device, output);
        Self {
            state: Mutex::new(State {
                context: None,
                built_for: None,
                reset: true,
                failed: false,
            }),
            output: texture,
            output_view: view,
        }
    }

    pub(super) fn resize(&mut self, device: &wgpu::Device, output: (u32, u32)) {
        let (texture, view) = create_output(device, output);
        self.output = texture;
        self.output_view = view;
        let mut state = self.state.lock().expect("dlss state lock");
        state.context = None;
        state.built_for = None;
        state.reset = true;
    }

    pub(super) fn output_texture(&self) -> &wgpu::Texture {
        &self.output
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw<'a>(
        &'a self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        runtime: &DlssRuntime,
        inputs: &crate::meshlet::vbuf64_stage::sgsr2::UpscaleInputs<'_>,
        render: (u32, u32),
        output: (u32, u32),
    ) -> Option<(&'a wgpu::TextureView, wgpu::CommandBuffer)> {
        let sdk = runtime.sdk.as_ref()?;
        let mut state = self.state.lock().expect("dlss state lock");
        if state.failed {
            return None;
        }

        let mode = perf_mode(render, output);
        if state.built_for != Some((output, mode)) {
            match DlssSuperResolution::new(
                [output.0, output.1],
                mode.as_sdk(),
                feature_flags(),
                std::sync::Arc::clone(sdk),
                device,
                queue,
            ) {
                Ok(context) => {
                    tracing::info!(
                        ?mode,
                        optimal = ?context.render_resolution(),
                        asked = ?render,
                        "DLSS context created"
                    );
                    state.context = Some(context);
                    state.built_for = Some((output, mode));
                    state.reset = true;
                }
                Err(error) => {
                    tracing::error!("DLSS context creation failed, falling back: {error}");
                    state.failed = true;
                    return None;
                }
            }
        }

        // Read before the context borrows `state` mutably.
        let reset = state.reset;
        let context = state.context.as_mut()?;
        // 🔴 The engine picks the render size from `render_scale`, and
        // NGX rounds its own ladder differently. Rather than let the
        // stage's targets be dictated from here — which would mean
        // reallocating every one of them behind the settings' back —
        // the size we already rendered at is declared as the subrect,
        // which is what NGX's dynamic-resolution path is for.
        //
        // Outside the range it will not reconstruct, and a silently
        // cropped image is worse than the fallback.
        let range = context.render_resolution_range();
        let (min, max) = (range.start(), range.end());
        if render.0 < min[0] || render.1 < min[1] || render.0 > max[0] || render.1 > max[1] {
            tracing::error!(
                ?render,
                ?min,
                ?max,
                "render size outside what DLSS accepts, falling back"
            );
            state.failed = true;
            return None;
        }

        let parameters = DlssSuperResolutionRenderParameters {
            color: inputs.color,
            depth: inputs.depth,
            motion_vectors: inputs.motion,
            exposure: DlssSuperResolutionExposure::Automatic,
            bias: None,
            dlss_output: &self.output_view,
            reset,
            // 🔴 Negated, like upstream's. The engine jitters the
            // projection by `+offset`; NGX is told where the samples
            // landed relative to the pixel centre, which is the other
            // direction.
            jitter_offset: [-inputs.jitter.x, -inputs.jitter.y],
            partial_texture_size: Some([render.0, render.1]),
            // 🔴 Ours are UV offsets with `prev_uv = uv - motion`
            // (`motion_vectors.wgsl`); NGX wants render-space pixels
            // pointing the other way. One negation and one scale, which
            // is exactly what FSR 3.1 does on load in `load_motion`.
            motion_vector_scale: Some([-(render.0 as f32), -(render.1 as f32)]),
        };

        match context.render(parameters, encoder, &runtime.adapter) {
            Ok(commands) => {
                state.reset = false;
                Some((&self.output_view, commands))
            }
            Err(error) => {
                tracing::error!("DLSS render failed, falling back: {error}");
                state.failed = true;
                None
            }
        }
    }
}

fn create_output(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dlss_output"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OUTPUT_FORMAT,
        // `STORAGE_BINDING` is what DLSS writes through; the rest is
        // how the passes after it read the result, and how a test does.
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
