//! `Material` — CPU-side PBR asset stored in `Assets<Material>`.

use std::fmt;

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use serde::{Deserialize, Serialize};

use super::MaterialParams;
use super::values::ParamValues;

/// What a material file is called.
pub const MATERIAL_EXTENSION: &str = "material";

/// CPU-side PBR material. The fields match Unity's "Standard (Specular setup)" minus textures —
/// enough to colour-modulate the deferred normal-debug pass without a real shading rig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    #[serde(default = "default_base_color")]
    pub base_color: [f32; 4],
    #[serde(default)]
    pub metallic: f32,
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    #[serde(default)]
    pub emissive: f32,
    /// Albedo/base-color map. `None` → modulate by `base_color` scalar.
    #[serde(default)]
    pub albedo: Option<Guid>,
    /// Tangent-space normal map. `None` → geometric normal (flat).
    #[serde(default)]
    pub normal: Option<Guid>,
    /// Packed metal (B) + roughness (G) map, glTF convention. `None` →
    /// `metallic` / `roughness` scalars.
    #[serde(default)]
    pub metal_roughness: Option<Guid>,
    /// How many times the maps repeat across the mesh's UVs.
    #[serde(default = "default_uv_scale")]
    pub uv_scale: [f32; 2],
    /// Where the maps start, in the same units. Slides the texture
    /// across the surface; whole numbers change nothing on a tiling
    /// texture, which is the point.
    #[serde(default)]
    pub uv_offset: [f32; 2],
    /// The `.shader` this material shades with. `None` → the engine's PBR surface.
    #[serde(default)]
    pub shader: Option<Guid>,
    /// Values for `shader`'s declared parameters, by name (#1158). Absent ones use the default.
    #[serde(default, skip_serializing_if = "ParamValues::is_empty")]
    pub values: ParamValues,
}

impl Material {
    /// Constructs a material with explicit PBR scalars and no textures.
    /// Attach maps fluently with [`with_albedo`](Self::with_albedo) etc.
    pub fn new(base_color: [f32; 4], metallic: f32, roughness: f32, emissive: f32) -> Self {
        Self {
            base_color,
            metallic,
            roughness,
            emissive,
            albedo: None,
            normal: None,
            metal_roughness: None,
            uv_scale: default_uv_scale(),
            uv_offset: [0.0, 0.0],
            shader: None,
            values: ParamValues::new(),
        }
    }

    /// Attaches an albedo map by asset [`Guid`].
    pub fn with_albedo(mut self, guid: Guid) -> Self {
        self.albedo = Some(guid);
        self
    }

    #[cfg(test)]
    /// Attaches a tangent-space normal map by asset [`Guid`].
    pub fn with_normal(mut self, guid: Guid) -> Self {
        self.normal = Some(guid);
        self
    }

    #[cfg(test)]
    /// Attaches a packed metal-roughness map by asset [`Guid`].
    pub fn with_metal_roughness(mut self, guid: Guid) -> Self {
        self.metal_roughness = Some(guid);
        self
    }

    /// Sets the texture transform.
    pub fn with_uv(mut self, scale: [f32; 2], offset: [f32; 2]) -> Self {
        self.uv_scale = scale;
        self.uv_offset = offset;
        self
    }

    /// Builds the GPU-side packed representation.
    pub fn to_params(&self) -> MaterialParams {
        MaterialParams::new(
            self.base_color,
            self.metallic,
            self.roughness,
            self.emissive,
        )
        .with_uv(self.uv_scale, self.uv_offset)
    }
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: default_base_color(),
            metallic: 0.0,
            roughness: default_roughness(),
            emissive: 0.0,
            albedo: None,
            normal: None,
            metal_roughness: None,
            uv_scale: default_uv_scale(),
            uv_offset: [0.0, 0.0],
            shader: None,
            values: ParamValues::new(),
        }
    }
}

fn default_base_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_roughness() -> f32 {
    0.5
}

/// One tile across the mesh's UVs — the texture as the mesh's author
/// laid it out. Anything else is the material's decision, so this is the
/// value a `.ron` that says nothing gets.
fn default_uv_scale() -> [f32; 2] {
    [1.0, 1.0]
}

/// `AssetLoader<Material>` for `*.ron` files.
#[derive(Debug, Default, Clone, Copy)]
pub struct MaterialLoader;

impl AssetLoader<Material> for MaterialLoader {
    fn extensions(&self) -> &[&'static str] {
        &[MATERIAL_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Material> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(MaterialParseError::Utf8(e))))?;
        let mat: Material = ron::from_str(text)
            .map_err(|e| AssetError::Loader(Box::new(MaterialParseError::Ron(e))))?;
        Ok(mat)
    }
}

/// Domain errors specific to material parsing.
#[derive(Debug)]
pub enum MaterialParseError {
    Utf8(std::str::Utf8Error),
    Ron(ron::error::SpannedError),
}

impl fmt::Display for MaterialParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "material RON is not valid UTF-8: {e}"),
            Self::Ron(e) => write!(f, "material RON parse failed: {e}"),
        }
    }
}

impl std::error::Error for MaterialParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Utf8(e) => Some(e),
            Self::Ron(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests;
