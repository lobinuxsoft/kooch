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
    /// What NGX said it wants the frame rendered at, and for which
    /// output.
    ///
    /// 🔴 The whole reason this is stored rather than checked. NGX's
    /// minimum render resolution **is** its optimal — it will not
    /// reconstruct from fewer pixels than the mode asks for — so the
    /// engine's own `render_scale` arithmetic cannot be the authority.
    /// A window of 943 rows halved is 471 by flooring and 472 by NGX's
    /// rounding, and one pixel short is refused outright.
    wanted: Option<((u32, u32), (u32, u32))>,
    /// The size the mismatch was last reported for, so a frame waiting
    /// for the resize does not log once per frame.
    reported: Option<(u32, u32)>,
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
                wanted: None,
                reported: None,
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
        state.wanted = None;
        state.reported = None;
        state.reset = true;
    }

    /// What NGX wants `output` rendered at, once it has said so.
    pub(super) fn wanted_render_size(&self, output: (u32, u32)) -> Option<(u32, u32)> {
        let state = self.state.lock().expect("dlss state lock");
        state
            .wanted
            .and_then(|(seen, render)| (seen == output).then_some(render))
    }

    pub(super) fn unusable(&self) -> bool {
        self.state.lock().expect("dlss state lock").failed
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
                    let optimal = context.render_resolution();
                    tracing::info!(?mode, ?optimal, asked = ?render, "DLSS context created");
                    // Every resource NGX is about to be handed, as NGX
                    // will see it: `texture_to_ngx` reads the real
                    // width, height and usage off each texture, so an
                    // input that is not the size this context was told
                    // to expect is invisible from up here.
                    tracing::info!(
                        color = ?described(inputs.color),
                        depth = ?described(inputs.depth),
                        motion = ?described(inputs.motion),
                        dlss_output = ?described(&self.output_view),
                        subrect = ?render,
                        range = ?(*context.render_resolution_range().start(),
                                  *context.render_resolution_range().end()),
                        "DLSS inputs"
                    );
                    // What the stage's targets have to become. Read here
                    // because this is the only moment NGX tells us.
                    state.wanted = Some((output, (optimal[0], optimal[1])));
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

        // 🔴 The range is read through a SHARED borrow and the whole
        // check runs before anything takes a mutable one. The engine
        // picks the render size from `render_scale`, and NGX rounds its
        // own ladder differently — but its MINIMUM is its optimal, so a
        // size a pixel below is refused rather than reconstructed from.
        let range = state.context.as_ref()?.render_resolution_range();
        let (min, max) = (*range.start(), *range.end());
        if render.0 < min[0] || render.1 < min[1] || render.0 > max[0] || render.1 > max[1] {
            // NOT sticky, and not an error. The stage reads
            // `wanted_render_size` and reallocates, so this is the one
            // frame in between — going sticky here is what turned a
            // one-pixel rounding difference into a permanently disabled
            // upscaler and a frame drawn into the corner of the window.
            if state.reported != Some(render) {
                state.reported = Some(render);
                tracing::warn!(
                    ?render,
                    ?min,
                    ?max,
                    "render size is not one DLSS accepts; resizing to its own and \
                     resolving with TAA for this frame"
                );
            }
            return None;
        }
        state.reported = None;

        // Read before the context borrows `state` mutably.
        let reset = state.reset;
        let context = state.context.as_mut()?;

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

/// What NGX reads off a view: the size, the format and whether it
/// counts as writable.
fn described(view: &wgpu::TextureView) -> (u32, u32, wgpu::TextureFormat, bool) {
    let texture = view.texture();
    (
        texture.width(),
        texture.height(),
        texture.format(),
        texture
            .usage()
            .contains(wgpu::TextureUsages::STORAGE_BINDING),
    )
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
