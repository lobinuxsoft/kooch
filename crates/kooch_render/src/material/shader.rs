//! `Shader` — the surface function a material shades with (#1157).

use std::fmt;

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

/// What a shader file is called.
pub const SHADER_EXTENSION: &str = "shader";

/// Static type name [`AssetEntry`](kooch_core::asset_database::AssetEntry)s carry for a shader.
pub const SHADER_TYPE_NAME: &str = "kooch_render::material::shader::Shader";

/// Which stage a shader plugs into, declared by a `// kind: <name>` line in its leading comments.
/// Only surfaces exist yet; the rest of #784 adds kinds rather than a second asset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderKind {
    /// Opaque shading on the visibility buffer: defines `fn surface(SurfaceInput) -> SurfaceOutput`.
    #[default]
    Surface,
}

impl ShaderKind {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "surface" => Some(Self::Surface),
            _ => None,
        }
    }
}

/// A WGSL body and the stage it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shader {
    pub kind: ShaderKind,
    pub source: String,
}

impl Shader {
    /// Reads the kind from the leading comments; a file that names none is a surface.
    pub fn parse(source: &str) -> Result<Self, ShaderParseError> {
        let header = source
            .lines()
            .map(str::trim)
            .take_while(|line| line.is_empty() || line.starts_with("//"));
        let mut kind = ShaderKind::default();
        for line in header {
            if let Some(name) = line
                .strip_prefix("//")
                .and_then(|l| l.trim().strip_prefix("kind:"))
            {
                let name = name.trim();
                kind = ShaderKind::parse(name)
                    .ok_or_else(|| ShaderParseError::Kind(name.to_owned()))?;
            }
        }
        Ok(Self {
            kind,
            source: source.to_owned(),
        })
    }
}

/// `AssetLoader<Shader>` for `*.shader` files.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShaderLoader;

impl AssetLoader<Shader> for ShaderLoader {
    fn extensions(&self) -> &[&'static str] {
        &[SHADER_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Shader> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(ShaderParseError::Utf8(e))))?;
        Shader::parse(text).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

/// Why a shader file did not load.
#[derive(Debug)]
pub enum ShaderParseError {
    Utf8(std::str::Utf8Error),
    Kind(String),
}

impl fmt::Display for ShaderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "shader is not valid UTF-8: {e}"),
            Self::Kind(name) => write!(f, "unknown shader kind `{name}` (known: surface)"),
        }
    }
}

impl std::error::Error for ShaderParseError {}

#[cfg(test)]
mod tests;
