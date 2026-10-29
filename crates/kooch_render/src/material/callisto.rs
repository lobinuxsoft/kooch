//! The Callisto BRDF's artist controls (#1657): SIGGRAPH 2023 Advances, "The Rendering of The
//! Callisto Protocol", slides 85, 90 and 95. Every default reproduces Lambert + GGX, by design.

use serde::{Deserialize, Serialize};

/// The light model's controls on a material. Applied by Inti per light, whatever the surface shader.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Callisto {
    /// Diffuse Fresnel, ρf: scales the diffuse where light grazes. [0, 256].
    pub fresnel: f32,
    pub fresnel_tint: [f32; 3],
    /// nf, [0, 1]: higher spreads it wider — the exponent is `5·2(1 − nf)`, 0 at 1.
    pub fresnel_falloff: f32,
    /// mf, [0, 1].
    pub fresnel_tangent: f32,
    /// Retroreflection, ρr: scales the diffuse where light comes from behind the viewer. [0, 256].
    pub retro: f32,
    pub retro_tint: [f32; 3],
    /// nr, [0, 1].
    pub retro_falloff: f32,
    /// mr, [0, 1].
    pub retro_tangent: f32,
    /// Smooth terminator, o, [−1, 1]: softens the light-to-shadow edge, outside the Fresnel and
    /// retroreflection areas.
    pub terminator: f32,
    pub terminator_tint: [f32; 3],
    /// p, [0, 1]: how wide the softened edge is.
    pub terminator_length: f32,
    /// ns, [0, 1]: Schlick at 0.5, no Fresnel at 0, full Fresnel at 1.
    pub specular_falloff: f32,
}

impl Default for Callisto {
    fn default() -> Self {
        Self {
            fresnel: 1.0,
            fresnel_tint: [1.0; 3],
            fresnel_falloff: 0.75,
            fresnel_tangent: 0.75,
            retro: 1.0,
            retro_tint: [1.0; 3],
            retro_falloff: 0.75,
            retro_tangent: 0.75,
            terminator: 0.0,
            terminator_tint: [1.0; 3],
            terminator_length: 0.5,
            specular_falloff: 0.5,
        }
    }
}

impl Callisto {
    /// `true` when nothing departs from Lambert + GGX, so the file need not say it.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The five `vec4`s `MaterialParams` carries. Exponents are remapped here, once per edit, not
    /// per light per pixel.
    pub fn pack(&self) -> [[f32; 4]; 5] {
        let tinted = |intensity: f32, tint: [f32; 3]| tint.map(|channel| channel * intensity);
        let [fr, fg, fb] = tinted(self.fresnel, self.fresnel_tint);
        let [rr, rg, rb] = tinted(self.retro, self.retro_tint);
        let [tr, tg, tb] = tinted(self.terminator, self.terminator_tint);
        let specular = remap(self.specular_falloff);
        // Lets the shader skip c1 and c2 on the common material, which pays only Proxima.
        let on = if self.is_default() { 0.0 } else { 1.0 };
        [
            [fr, fg, fb, 5.0 * remap(self.fresnel_falloff)],
            [rr, rg, rb, 5.0 * remap(self.retro_falloff)],
            [tr, tg, tb, self.terminator_length],
            [
                5.0 * remap(self.fresnel_tangent),
                5.0 * remap(self.retro_tangent),
                (2.0 - specular).clamp(0.0, 1.0),
                5.0 * specular,
            ],
            [on, 0.0, 0.0, 0.0],
        ]
    }
}

/// The slides' `r(x) = 2(1 − x)`: [0, 1] onto [2, 0], with 0.5 landing on Schlick's 1.
fn remap(x: f32) -> f32 {
    2.0 * (1.0 - x)
}
