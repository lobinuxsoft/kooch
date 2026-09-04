//! `Material` — CPU-side PBR asset stored in `Assets<Material>`.
//!
//! Authored as RON in a `*.material` file; the inspector exposes it
//! as a typed asset reference on `MeshRenderer.material`. Convertible
//! to [`MaterialParams`](super::MaterialParams) for GPU upload — the
//! runtime sync system mirrors the asset storage into the
//! `MaterialPool` storage buffer once per change.
//!
//! Field set carries the PBR scalars (`base_color`, `metallic`,
//! `roughness`, `emissive`) plus optional texture references (`albedo`,
//! `normal`, `metal_roughness`) stored as [`Guid`]s — the same
//! persistible identifier `MeshRenderer` uses for mesh/material. A
//! `None` texture falls back to the scalar (`base_color` for albedo, a
//! flat normal, scalar metal/roughness) so pre-texture projects keep
//! their look with zero migration.

use std::fmt;

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use serde::{Deserialize, Serialize};

use super::MaterialParams;

/// What a material file is called.
///
/// 🔴 Named after the thing, not the syntax. RON is the format, and
/// leaving the extension as `ron` meant the first RON asset type to
/// arrive after it collided: the scan resolves a loader by extension
/// with a `find`, so a block written as `.ron` was typed as a material.
/// One extension, one type.
pub const MATERIAL_EXTENSION: &str = "material";

/// CPU-side PBR material. The fields match Unity's "Standard
/// (Specular setup)" minus textures — enough to colour-modulate the
/// deferred normal-debug pass without a real shading rig.
///
/// `base_color` is RGBA in linear space. `metallic`, `roughness`,
/// `emissive` are scalar coefficients.
///
/// Defaults to a neutral white diffuse so a fresh `Material::new()`
/// (or a RON file with all fields elided) produces the same look as
/// the legacy hard-coded `MaterialParams::default`.
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
    ///
    /// A mesh's UVs describe the surface once; how densely a texture
    /// sits on it is the material's business, not the mesh's. Without
    /// this a 1024-pixel grid stretches to a single tile over a floor
    /// however large the floor is, which is the one thing a prototype
    /// grid exists not to do — one square is supposed to be a known
    /// distance.
    ///
    /// 🔴 Scaling the mesh's UVs instead is not the same thing: the mesh
    /// is shared, so it would change every object that uses it.
    #[serde(default = "default_uv_scale")]
    pub uv_scale: [f32; 2],
    /// Where the maps start, in the same units. Slides the texture
    /// across the surface; whole numbers change nothing on a tiling
    /// texture, which is the point.
    #[serde(default)]
    pub uv_offset: [f32; 2],
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
        }
    }

    /// Attaches an albedo map by asset [`Guid`].
    pub fn with_albedo(mut self, guid: Guid) -> Self {
        self.albedo = Some(guid);
        self
    }

    /// Attaches a tangent-space normal map by asset [`Guid`].
    pub fn with_normal(mut self, guid: Guid) -> Self {
        self.normal = Some(guid);
        self
    }

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
///
/// PR5 carries only `Material` as a RON-authored asset, so the
/// extension is unambiguous. When other RON-authored asset types
/// arrive (`Scene`, `Prefab`, …) we discriminate by inspecting the
/// nominal struct tag at the head of the file (`Material(...)` vs
/// `Scene(...)`) and the eager-import logic gains a per-type tier.
/// Until then, every `.ron` under `assets/` is parsed as a Material.
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
