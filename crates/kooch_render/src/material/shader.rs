//! `Shader` — the surface function a material shades with (#1157), and the parameters it declares
//! (#1158).
//!
//! The header plays the part of Shader Forge's `Properties` block: plain comments the compiler
//! ignores and the editor reads, so the file stays valid WGSL with nothing generated inside it.
//!
//! ```wgsl
//! // kind: surface
//! // param tint: color = (1, 0.5, 0.2, 1)
//! // param strength: float = 1.0 range(0, 4)
//! // param detail: texture = white
//! ```

use std::fmt;

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

mod params;

pub use params::{MAX_PARAM_SCALARS, MAX_PARAM_TEXTURES, ParamKind, ShaderParam, TextureDefault};

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

/// A WGSL body, the stage it belongs to and the parameters it declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    pub kind: ShaderKind,
    pub source: String,
    /// In declaration order, which is also the packing order.
    pub params: Vec<ShaderParam>,
}

impl Shader {
    /// Reads the header: the leading comment lines, where a file that names no kind is a surface.
    pub fn parse(source: &str) -> Result<Self, ShaderParseError> {
        let header = source
            .lines()
            .map(str::trim)
            .enumerate()
            .take_while(|(_, line)| line.is_empty() || line.starts_with("//"));
        let mut kind = ShaderKind::default();
        let mut params = Vec::new();
        for (index, line) in header {
            let Some(directive) = line.strip_prefix("//").map(str::trim) else {
                continue;
            };
            if let Some(name) = directive.strip_prefix("kind:") {
                let name = name.trim();
                kind = ShaderKind::parse(name)
                    .ok_or_else(|| ShaderParseError::Kind(name.to_owned()))?;
            } else if let Some(declaration) = directive.strip_prefix("param ") {
                let param = params::parse(declaration, &params).map_err(|message| {
                    ShaderParseError::Param {
                        line: index + 1,
                        message,
                    }
                })?;
                params.push(param);
            }
        }
        Ok(Self {
            kind,
            source: source.to_owned(),
            params,
        })
    }

    /// The WGSL the engine composes ahead of the body: `SurfaceParams`, `surface_params` and a
    /// `sample_<name>` per texture.
    pub fn params_wgsl(&self) -> String {
        params::wgsl(&self.params)
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
    /// A `// param` line that does not parse, or one past the budget.
    Param {
        line: usize,
        message: String,
    },
}

impl fmt::Display for ShaderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "shader is not valid UTF-8: {e}"),
            Self::Kind(name) => write!(f, "unknown shader kind `{name}` (known: surface)"),
            Self::Param { line, message } => write!(f, "line {line}: {message}"),
        }
    }
}

impl std::error::Error for ShaderParseError {}

#[cfg(test)]
mod tests;
