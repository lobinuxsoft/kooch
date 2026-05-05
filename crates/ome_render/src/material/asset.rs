//! `Material` — CPU-side PBR asset stored in `Assets<Material>`.
//!
//! Authored as RON (`*.ome_material.ron`); the inspector exposes it
//! as a typed asset reference on `MeshRenderer.material`. Convertible
//! to [`MaterialParams`](super::MaterialParams) for GPU upload — the
//! runtime sync system mirrors the asset storage into the
//! `MaterialPool` storage buffer once per change.
//!
//! Field set is intentionally minimal: PR5 only carries the PBR
//! scalars (`base_color`, `metallic`, `roughness`, `emissive`). Texture
//! references (`albedo`, `normal`, `metal_roughness`, etc.) land
//! alongside bindless texturing in a follow-up; the field shape
//! changes additively, no migration burden on early projects.

use std::fmt;

use ome_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use serde::{Deserialize, Serialize};

use super::MaterialParams;

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
}

impl Material {
    /// Constructs a material with explicit values for every field.
    pub fn new(base_color: [f32; 4], metallic: f32, roughness: f32, emissive: f32) -> Self {
        Self {
            base_color,
            metallic,
            roughness,
            emissive,
        }
    }

    /// Builds the GPU-side packed representation.
    pub fn to_params(&self) -> MaterialParams {
        MaterialParams::new(self.base_color, self.metallic, self.roughness, self.emissive)
    }
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: default_base_color(),
            metallic: 0.0,
            roughness: default_roughness(),
            emissive: 0.0,
        }
    }
}

fn default_base_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_roughness() -> f32 {
    0.5
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
        &["ron"]
    }

    fn load(
        &self,
        bytes: &[u8],
        _ctx: &mut LoadContext<'_>,
    ) -> AssetResult<Material> {
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
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extension_is_ron() {
        assert_eq!(MaterialLoader.extensions(), &["ron"]);
    }

    #[test]
    fn default_matches_legacy_white_diffuse() {
        let m = Material::default();
        assert_eq!(m.base_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(m.metallic, 0.0);
        assert_eq!(m.roughness, 0.5);
        assert_eq!(m.emissive, 0.0);
    }

    #[test]
    fn to_params_round_trips_scalars() {
        let m = Material::new([0.2, 0.3, 0.4, 1.0], 0.6, 0.7, 1.5);
        let p = m.to_params();
        assert_eq!(p.base_color, [0.2, 0.3, 0.4, 1.0]);
        assert_eq!(p.metallic(), 0.6);
        assert_eq!(p.roughness(), 0.7);
        assert_eq!(p.emissive(), 1.5);
    }

    #[test]
    fn ron_minimal_uses_defaults() {
        // Empty struct literal — every field falls through to its
        // serde default. Exercises the back-compat contract: future
        // schemas must preserve this property.
        let mut ctx = LoadContext {
            path: Path::new("empty.ome_material.ron"),
        };
        let m = MaterialLoader.load(b"()", &mut ctx).expect("empty struct parses");
        assert_eq!(m, Material::default());
    }

    #[test]
    fn ron_full_round_trip() {
        let original = Material::new([0.9, 0.1, 0.05, 1.0], 0.0, 0.4, 0.0);
        let text = ron::ser::to_string_pretty(&original, ron::ser::PrettyConfig::default())
            .expect("serialize");
        let mut ctx = LoadContext {
            path: Path::new("red.ome_material.ron"),
        };
        let parsed = MaterialLoader.load(text.as_bytes(), &mut ctx).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn ron_partial_fills_remaining_with_defaults() {
        let text = r#"(
    base_color: (0.1, 0.2, 0.3, 1.0),
    emissive: 2.0,
)"#;
        let mut ctx = LoadContext {
            path: Path::new("partial.ome_material.ron"),
        };
        let m = MaterialLoader.load(text.as_bytes(), &mut ctx).expect("parse");
        assert_eq!(m.base_color, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(m.emissive, 2.0);
        // Missing fields fell back to defaults.
        assert_eq!(m.metallic, 0.0);
        assert_eq!(m.roughness, 0.5);
    }

    #[test]
    fn invalid_bytes_return_loader_error() {
        let mut ctx = LoadContext {
            path: Path::new("garbage.ome_material.ron"),
        };
        let err = MaterialLoader
            .load(b"not RON at all =", &mut ctx)
            .expect_err("garbage rejected");
        assert!(matches!(err, AssetError::Loader(_)));
    }
}
