//! A plugin that draws, in the smallest form that still shows every moving part (#392).
//!
//! ```text
//! RUSTFLAGS="-C prefer-dynamic" cargo build -p example_post_process
//! ```
//!
//! 🔴 It edits no engine file. The pass is ordered **by name** against an engine system, so it runs
//! after the scene and before the present without either of them knowing it exists.

use kooch_plugin_api::prelude::*;
use kooch_plugin_render::{PassFrame, PassSetup, RenderEngine, RenderPass, TargetDesc};

/// Paints a gradient into a target it asks the pool for: a full-screen draw, which is the shape
/// every post-process has.
#[derive(Default)]
struct Gradient {
    pipeline: Option<wgpu::RenderPipeline>,
}

const SHADER: &str = r#"
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index & 1u) << 2u) - 1.0;
    let y = f32((index & 2u) << 1u) - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(position.x / 256.0, position.y / 256.0, 0.5, 1.0);
}
"#;

const SIDE: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl RenderPass for Gradient {
    fn name(&self) -> &str {
        "example_gradient"
    }

    fn init(&mut self, setup: PassSetup<'_>) {
        let module = setup
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("example_gradient"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        self.pipeline = Some(setup.device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("example_gradient"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    targets: &[Some(FORMAT.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            },
        ));
    }

    fn record(&mut self, frame: PassFrame<'_>) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        // Somewhere to draw, from the engine's pool: the texture stays the pool's, so this costs
        // nothing after the first frame.
        let target = frame.targets.acquire(
            "example_gradient",
            TargetDesc::attachment((SIDE, SIDE), FORMAT),
        );
        let Some(view) = frame.targets.view(target).cloned() else {
            return;
        };

        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("example_gradient"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(pipeline);
        pass.draw(0..3, 0..1);
        drop(pass);

        frame.targets.release(target);
    }
}

#[derive(Default)]
struct PostProcessPlugin;

impl KoochPlugin for PostProcessPlugin {
    fn name(&self) -> &str {
        "PostProcessPlugin"
    }

    fn build(&mut self, engine: &mut dyn Engine) {
        // Between the scene and the present, by name. Neither system is edited, and neither knows
        // this pass exists.
        let added = engine.add_pass(
            Stage::Render,
            Order::after("render_meshlets_system").and_before("present_frame_system"),
            Gradient::default(),
        );
        engine.log(match added {
            true => "PostProcessPlugin: pass registered",
            false => "PostProcessPlugin: the host refused the pass",
        });
    }
}

kooch_plugin_api::export_plugin!(PostProcessPlugin);
