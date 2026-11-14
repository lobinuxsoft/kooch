//! The engine's sky: a `kind: sky` graph drawn with its own defaults when a `SkyRenderer` names no
//! material, or one that cannot draw.

use super::graph::{SkyGraph, SkyMaterials, SkyView};
use crate::material::shader::MAX_PARAM_TEXTURES;
use crate::material::{
    Material, MaterialParams, MaterialPool, MaterialTexturePool, PackedParams, Shader, TextureRef,
};

/// The engine's sky graph, open in the Shader Graph panel like any other.
pub const DEFAULT_SKY_SHADER: &str = include_str!("../../shaders/sky_space.shgraph");

/// Its pipeline never changes once built.
const STAMP: u64 = 0;

pub(super) struct DefaultSky {
    /// Its own: sharing a material's cache would rebuild both every frame a material falls back.
    graph: SkyGraph,
    params_wgsl: String,
    source: String,
    /// One slot holding the graph's parameter defaults.
    pool: MaterialPool,
    values: PackedParams,
    seeded: bool,
    /// Said once: an engine shader that does not compile is a bug, not a frame-rate log line.
    broken: bool,
}

impl DefaultSky {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Option<Self> {
        let shader = match Shader::parse(DEFAULT_SKY_SHADER) {
            Ok(shader) => shader,
            Err(e) => {
                tracing::error!(target: "kooch_render::sky", "the engine's sky does not parse: {e}");
                return None;
            }
        };
        Some(Self {
            graph: SkyGraph::new(device, format),
            params_wgsl: shader.params_wgsl(),
            values: PackedParams::for_shader(&Material::default(), &shader.params),
            source: shader.source,
            pool: MaterialPool::new(device, &[MaterialParams::default()]),
            seeded: false,
            broken: false,
        })
    }

    /// Records the engine's sky into `pass`. `false` leaves the clear, black.
    pub(super) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        textures: &MaterialTexturePool,
        view: &SkyView,
    ) -> bool {
        if let Err(why) = self
            .graph
            .ensure(device, STAMP, &self.params_wgsl, &self.source)
        {
            if !self.broken {
                self.broken = true;
                tracing::error!(target: "kooch_render::sky", "the engine's sky does not compile: {why}");
            }
            return false;
        }
        if !self.seeded {
            self.seeded = true;
            self.pool.write_values(queue, 0, &self.values.values);
        }
        let refs: &[TextureRef; MAX_PARAM_TEXTURES as usize] = &self.values.textures;
        self.graph.record(
            device,
            queue,
            pass,
            SkyMaterials {
                pool: &self.pool,
                textures,
                refs,
                slot: 0,
            },
            view,
        )
    }
}
