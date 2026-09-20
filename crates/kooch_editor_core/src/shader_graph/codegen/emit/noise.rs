//! The noises' emission: which samples a fractal noise takes and which Voronoi pass runs depend on
//! what is wired, so an unused output costs nothing (#1159).

use egui_snarl::{InPinId, NodeId};

use super::Body;
use crate::shader_graph::{
    NOISE_BASES, NOISE_COLOUR, NOISE_DISTORTION, NOISE_FRACTALS, NOISE_PHASE, NOISE_TILING,
    VORONOI_BORDER, VORONOI_METRICS, VORONOI_TILING, WHITE_TILING,
};

/// Where the colour's green and blue are sampled: far enough from the value to look unrelated.
const GREEN_OFFSET: &str = "vec2<f32>(19.1, 7.3)";
const BLUE_OFFSET: &str = "vec2<f32>(-5.7, 31.4)";

impl Body<'_> {
    fn wired_in(&self, node: NodeId, input: usize) -> bool {
        self.wires.contains_key(&InPinId { node, input })
    }

    fn wired_out(&self, node: NodeId, output: usize) -> bool {
        self.wires
            .values()
            .any(|from| from.node == node && from.output == output)
    }

    /// The period the lattice wraps against, and the coordinate that reads it (#1237). A wired
    /// tiling sets the cells across the uv square itself, so the noise repeats exactly: a scale that
    /// disagreed with the period would seam however round the period was.
    fn tiling(
        &mut self,
        id: NodeId,
        input: usize,
        uv: &str,
        scale: &str,
    ) -> Result<(String, String), String> {
        if !self.wired_in(id, input) {
            let at = self.local(&format!("{uv}.xy * {scale}.x"));
            return Ok((at, "vec2<f32>(0.0)".to_owned()));
        }
        let tiling = self.input_or(id, input, "vec4<f32>(0.0)")?;
        let period = self.local(&format!("round(max({tiling}.xy, vec2<f32>(0.0)))"));
        let at = self.local(&format!(
            "{uv}.xy * select(vec2<f32>({scale}.x), {period}, {period} > vec2<f32>(0.5))"
        ));
        Ok((at, period))
    }

    /// Value in `x`; the colour in `yzw` only when that output is wired.
    pub(super) fn fractal_noise(
        &mut self,
        id: NodeId,
        basis: &str,
        fractal: &str,
    ) -> Result<String, String> {
        let basis = NOISE_BASES
            .iter()
            .find(|b| **b == basis)
            .unwrap_or(&"value");
        let mode = NOISE_FRACTALS
            .iter()
            .position(|f| *f == fractal)
            .unwrap_or(0);
        let uv = self.input_or(id, 0, "vec4<f32>(0.0)")?;
        let scale = self.input_or(id, 1, "vec4<f32>(1.0)")?;
        let octaves = self.input_or(id, 2, "vec4<f32>(1.0)")?;
        let roughness = self.input_or(id, 3, "vec4<f32>(0.5)")?;
        let lacunarity = self.input_or(id, 4, "vec4<f32>(2.0)")?;
        if *basis == "simplex" && self.wired_in(id, NOISE_TILING) {
            return Err(
                "a simplex noise cannot tile: its lattice is skewed, so a period repeats in skewed \
                 space and not in uv. Use value or gradient for a seamless noise"
                    .to_owned(),
            );
        }
        let (mut at, period) = self.tiling(id, NOISE_TILING, &uv, &scale)?;
        if self.wired_in(id, NOISE_DISTORTION) {
            let amount = self.input_or(id, NOISE_DISTORTION, "vec4<f32>(0.0)")?;
            at = self.local(&format!("graph_{basis}_warp({at}, {amount}.x, {period})"));
        }
        let phase = if self.wired_in(id, NOISE_PHASE) {
            Some(self.input_or(id, NOISE_PHASE, "vec4<f32>(0.0)")?)
        } else {
            None
        };
        let controls = format!("{octaves}.x, {roughness}.x, {lacunarity}.x, {mode}.0, {period}");
        let sample = |point: &str| match &phase {
            Some(phase) => {
                format!("graph_{basis}_fractal3(vec3<f32>({point}, {phase}.x), {controls})")
            }
            None => format!("graph_{basis}_fractal({point}, {controls})"),
        };
        let value = self.local(&sample(&at));
        if !self.wired_out(id, NOISE_COLOUR) {
            return Ok(format!("vec4<f32>({value})"));
        }
        let green = sample(&format!("{at} + {GREEN_OFFSET}"));
        let blue = sample(&format!("{at} + {BLUE_OFFSET}"));
        Ok(format!("vec4<f32>({value}, {value}, {green}, {blue})"))
    }

    /// A `GraphVoronoi`; the border's wider pass runs only when that output is wired.
    pub(super) fn voronoi(&mut self, id: NodeId, metric: &str) -> Result<String, String> {
        let metric = VORONOI_METRICS
            .iter()
            .position(|m| *m == metric)
            .unwrap_or(0);
        let uv = self.input_or(id, 0, "vec4<f32>(0.0)")?;
        let scale = self.input_or(id, 1, "vec4<f32>(1.0)")?;
        let randomness = self.input_or(id, 2, "vec4<f32>(1.0)")?;
        let phase = self.input_or(id, 3, "vec4<f32>(0.0)")?;
        let smoothness = self.input_or(id, 4, "vec4<f32>(0.0)")?;
        let edges = self.wired_out(id, VORONOI_BORDER);
        let (at, period) = self.tiling(id, VORONOI_TILING, &uv, &scale)?;
        Ok(format!(
            "graph_voronoi({at}, clamp({randomness}.x, 0.0, 1.0), {phase}.x, \
             max({smoothness}.x, 0.0), {metric}, {edges}, {period})"
        ))
    }

    /// One hash per cell, the cell wrapped when a period is wired.
    pub(super) fn white_noise(&mut self, id: NodeId) -> Result<String, String> {
        let uv = self.input_or(id, 0, "vec4<f32>(0.0)")?;
        let scale = self.input_or(id, 1, "vec4<f32>(1.0)")?;
        let (at, period) = self.tiling(id, WHITE_TILING, &uv, &scale)?;
        Ok(format!(
            "vec4<f32>(graph_hash(graph_wrap(floor({at}), {period})))"
        ))
    }
}
