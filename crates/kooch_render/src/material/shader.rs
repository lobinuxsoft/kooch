//! `Shader` — the surface function a material shades with (#1157), and the parameters it declares
//! (#1158).
//!
//! Parameters are plain WGSL, as close to an HLSL `cbuffer` and `Texture2D` as WGSL allows; hints
//! in a trailing comment only change how the editor shows a field.
//!
//! ```wgsl
//! // kind: surface
//! struct SurfaceParams {
//!     tint: vec4<f32>,   // @color @default(1, 0.5, 0.2, 1)
//!     strength: f32,     // @range(0, 4) @default(1)
//! }
//! var detail: texture_2d<f32>;   // @default(white)
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
/// Every kind composes into the same frames; the kind's glue decides how they treat the result (#1178).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderKind {
    /// Opaque shading on the visibility buffer: defines `fn surface(SurfaceInput) -> SurfaceOutput`.
    #[default]
    Surface,
    /// A colour no light touches: defines `fn unlit(SurfaceInput) -> UnlitOutput` (#1179).
    Unlit,
    /// One full-screen draw over the frame the camera produced: defines
    /// `fn post_process(SurfaceInput) -> vec4<f32>` and reads `sample_scene` (#1201).
    PostProcess,
    /// A lit surface blended over the opaque scene, sorted back to front: defines
    /// `fn surface(SurfaceInput) -> SurfaceOutput` and sets its `alpha` (#452).
    Transparent,
}

impl ShaderKind {
    /// What a `// kind:` line names, in the order errors list them.
    pub const NAMES: [&'static str; 4] = ["surface", "unlit", "post_process", "transparent"];

    fn parse(name: &str) -> Option<Self> {
        match name {
            "surface" => Some(Self::Surface),
            "unlit" => Some(Self::Unlit),
            "post_process" => Some(Self::PostProcess),
            "transparent" => Some(Self::Transparent),
            _ => None,
        }
    }

    /// WGSL the frames read: `SURFACE_UNLIT`, `SURFACE_TRANSPARENT`, and for an unlit shader the
    /// `surface` they call.
    fn glue(self) -> &'static str {
        match self {
            // A post-process has a frame of its own, so it needs no glue at all.
            Self::Surface | Self::PostProcess => {
                "const SURFACE_UNLIT: bool = false;\nconst SURFACE_TRANSPARENT: bool = false;\n"
            }
            Self::Transparent => {
                "const SURFACE_UNLIT: bool = false;\nconst SURFACE_TRANSPARENT: bool = true;\n"
            }
            Self::Unlit => UNLIT_GLUE,
        }
    }
}

/// An unlit colour rides in `emissive`, the one output the frames already add past the light.
const UNLIT_GLUE: &str = "\
const SURFACE_UNLIT: bool = true;
const SURFACE_TRANSPARENT: bool = false;
fn surface(input: SurfaceInput) -> SurfaceOutput {
    let unlit = unlit(input);
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.emissive = unlit.color;
    out.alpha = unlit.alpha;
    out.alpha_clip = unlit.alpha_clip;
    return out;
}
";

/// A WGSL body, the stage it belongs to and the parameters it declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    pub kind: ShaderKind,
    /// The author's source with texture bindings filled in, line for line.
    pub source: String,
    /// In declaration order, which is also the packing order.
    pub params: Vec<ShaderParam>,
}

impl Shader {
    /// Reads the kind from the leading comments (none means a surface) and the parameters from
    /// the code.
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
        let read = params::read(source)
            .map_err(|(line, message)| ShaderParseError::Param { line, message })?;
        Ok(Self {
            kind,
            source: read.source,
            params: read.params,
        })
    }

    /// Whether it assigns `alpha_clip`, and so rasterises in the masked bin (#452). Comments do not
    /// count: a shader that only mentions it stays opaque.
    pub fn masked(&self) -> bool {
        masks(&self.source)
    }

    /// Whether its cut reads uv and textures alone, and so can be baked into geometry (#452).
    pub fn masks_still(&self) -> bool {
        masks_still(&self.source)
    }

    /// The WGSL the engine composes ahead of the source: the kind's glue, `surface_params` and
    /// `surface_texture_dims`.
    pub fn params_wgsl(&self) -> String {
        format!("{}{}", self.kind.glue(), params::generated(&self.params))
    }

    /// The engine's PBR surface, parsed once.
    pub fn default_surface() -> &'static Self {
        static DEFAULT: std::sync::OnceLock<Shader> = std::sync::OnceLock::new();
        DEFAULT.get_or_init(|| {
            Self::parse(crate::meshlet::DEFAULT_SURFACE_SHADER)
                .expect("the engine's surface parses")
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
    /// A parameter declaration that does not parse, or one past the budget.
    Param {
        line: usize,
        message: String,
    },
}

impl fmt::Display for ShaderParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "shader is not valid UTF-8: {e}"),
            Self::Kind(name) => write!(
                f,
                "unknown shader kind `{name}` (known: {})",
                ShaderKind::NAMES.join(", ")
            ),
            Self::Param { line, message } => write!(f, "line {line}: {message}"),
        }
    }
}

impl std::error::Error for ShaderParseError {}

#[cfg(test)]
mod tests;

/// What a cut that can be baked never reads: where the surface is, who looks at it, when.
const MOVING_INPUTS: [&str; 4] = [
    "input.time",
    "input.world_position",
    "input.camera_position",
    "input.frag_coord",
];

/// Whether `source` masks by uv and textures alone (#452). Conservative: a mention anywhere refuses,
/// even where the alpha itself never reads it.
pub fn masks_still(source: &str) -> bool {
    masks(source)
        && !source.lines().any(|line| {
            let code = line.split("//").next().unwrap_or_default();
            MOVING_INPUTS.iter().any(|input| code.contains(input))
        })
}

/// Whether `source`'s code, outside comments, assigns an output's `alpha_clip`.
pub fn masks(source: &str) -> bool {
    source.lines().any(|line| {
        let code = line.split("//").next().unwrap_or_default();
        code.find(".alpha_clip").is_some_and(|at| {
            let rest = code[at + ".alpha_clip".len()..].trim_start();
            rest.starts_with('=') && !rest.starts_with("==")
        })
    })
}
