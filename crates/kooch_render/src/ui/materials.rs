//! `kind: ui` shaders (#1690): a pipeline per shader, built from the text the draw list carries,
//! and each material's values packed where `surface_params` reads them.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use kooch_core::Guid;
use kooch_ecs::{UiDrawList, UiShader};

use crate::material::{MAX_PARAM_SCALARS, Shader, ShaderKind};
use crate::meshlet::validate_body;

const UI: &str = include_str!("../../shaders/ui.wgsl");
const PRELUDE: &str = include_str!("../../shaders/ui_material.wgsl");
const FRAME: &str = include_str!("../../shaders/ui_material_frame.wgsl");

fn compose(params: &str, body: &str) -> String {
    [UI, PRELUDE, params, body, FRAME].join("\n")
}

/// Checks a `kind: ui` shader before a pipeline is built from it. Lines are the file's own.
pub fn validate_ui(shader: &Shader) -> Result<(), String> {
    if shader.kind != ShaderKind::Ui {
        return Err(format!("its shader is {:?}, not ui", shader.kind));
    }
    let params = shader.params_wgsl();
    validate_body(|body| compose(&params, body), &shader.source)
}

/// A shader as last built. `built` is `None` when it was refused: its quads draw as plain panels.
struct Entry {
    text: Arc<str>,
    built: Option<(Shader, wgpu::RenderPipeline)>,
}

/// A contiguous stretch of quads one pipeline draws.
pub(super) struct Run {
    pub shader: Guid,
    pub quads: Range<u32>,
}

#[derive(Default)]
pub(super) struct UiMaterials {
    entries: HashMap<Guid, Entry>,
    refused: HashSet<Guid>,
    /// [`MAX_PARAM_SCALARS`] per material, in list order.
    pub values: Vec<f32>,
    pub runs: Vec<Run>,
}

impl UiMaterials {
    /// Builds what changed and lays out this frame's values and runs.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        list: &UiDrawList,
    ) {
        for shader in &list.shaders {
            self.ensure(device, layout, format, shader);
        }
        self.values.clear();
        self.runs.clear();
        for material in &list.materials {
            let guid = list.shaders[material.shader as usize].guid;
            let [first, count] = material.params.map(|n| n as usize);
            let block = self.values.len();
            self.values.resize(block + MAX_PARAM_SCALARS as usize, 0.0);
            let Some((shader, _)) = self.entries.get(&guid).and_then(|e| e.built.as_ref()) else {
                continue;
            };
            for param in &shader.params {
                let written = list.params[first..first + count]
                    .iter()
                    .find(|own| own.name.eq_ignore_ascii_case(&param.name));
                let value = written.map_or(param.default, |own| own.value);
                let at = block + param.offset as usize;
                let width = param.kind.width() as usize;
                self.values[at..at + width].copy_from_slice(&value[..width]);
            }
            match self.runs.last_mut() {
                Some(run) if run.shader == guid && run.quads.end == material.quad => {
                    run.quads.end += 1;
                }
                _ => self.runs.push(Run {
                    shader: guid,
                    quads: material.quad..material.quad + 1,
                }),
            }
        }
    }

    pub fn pipeline(&self, guid: Guid) -> Option<&wgpu::RenderPipeline> {
        self.entries
            .get(&guid)?
            .built
            .as_ref()
            .map(|(_, pipeline)| pipeline)
    }

    fn ensure(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        shader: &UiShader,
    ) {
        if self
            .entries
            .get(&shader.guid)
            .is_some_and(|entry| entry.text == shader.source)
        {
            return;
        }
        let parsed = Shader::parse(&shader.source).map_err(|e| e.to_string());
        let built = parsed.and_then(|parsed| {
            validate_ui(&parsed)?;
            let pipeline = build(device, layout, format, &parsed);
            Ok((parsed, pipeline))
        });
        let built = match built {
            Ok(built) => {
                self.refused.remove(&shader.guid);
                Some(built)
            }
            Err(why) => {
                // 🔴 Refused, not built: a broken edit reads as a message, never as a wgpu panic.
                if self.refused.insert(shader.guid) {
                    tracing::error!(
                        target: "kooch_render::material::shader",
                        "UI shader {} does not compile — {why}",
                        shader.guid,
                    );
                }
                None
            }
        };
        self.entries.insert(
            shader.guid,
            Entry {
                text: shader.source.clone(),
                built,
            },
        );
    }
}

fn build(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    shader: &Shader,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ui_material"),
        source: wgpu::ShaderSource::Wgsl(compose(&shader.params_wgsl(), &shader.source).into()),
    });
    super::pipeline(device, layout, format, &module, "fs_material")
}
