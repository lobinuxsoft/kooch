//! `// param` declarations: parsing, the per-material budget, and the WGSL generated from them.

use std::fmt::Write;

/// Scalars a material holds for its shader, packed into `material_values` (16 `f32` per slot).
pub const MAX_PARAM_SCALARS: u32 = 16;

/// Textures a material binds for its shader: the three PBR slots and `extra_tex`.
pub const MAX_PARAM_TEXTURES: u32 = 4;

/// The group-4 texture each declared texture occupies, in declaration order.
const TEXTURE_GLOBALS: [&str; MAX_PARAM_TEXTURES as usize] =
    ["albedo_tex", "normal_tex", "metal_rough_tex", "extra_tex"];

/// What a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Float,
    Vec2,
    Vec3,
    Vec4,
    /// A `vec4` the Inspector edits with a colour picker.
    Color,
    Texture,
}

impl ParamKind {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "float" => Self::Float,
            "vec2" => Self::Vec2,
            "vec3" => Self::Vec3,
            "vec4" => Self::Vec4,
            "color" => Self::Color,
            "texture" => Self::Texture,
            _ => return None,
        })
    }

    /// Scalars it takes in `material_values`.
    pub fn width(self) -> u32 {
        match self {
            Self::Float => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 | Self::Color => 4,
            Self::Texture => 0,
        }
    }

    fn wgsl(self) -> &'static str {
        match self {
            Self::Float => "f32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 | Self::Color | Self::Texture => "vec4<f32>",
        }
    }
}

/// What an unassigned texture parameter samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureDefault {
    #[default]
    White,
    Black,
    /// A flat tangent-space normal.
    Normal,
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderParam {
    pub name: String,
    pub kind: ParamKind,
    /// Numeric default, unused components zero.
    pub default: [f32; 4],
    pub texture: TextureDefault,
    /// Slider bounds for a float.
    pub range: Option<[f32; 2]>,
    /// First scalar in the material's block, or the texture index for a texture.
    pub offset: u32,
}

/// Parses `name: kind [= default] [range(lo, hi)]`, placed after `before`.
pub(super) fn parse(declaration: &str, before: &[ShaderParam]) -> Result<ShaderParam, String> {
    let (name, rest) = declaration
        .split_once(':')
        .ok_or("expected `param name: kind`")?;
    let name = name.trim();
    if !is_identifier(name) {
        return Err(format!("`{name}` is not a WGSL identifier"));
    }
    if before.iter().any(|p| p.name == name) {
        return Err(format!("`{name}` is declared twice"));
    }

    let (rest, range) = match rest.split_once("range(") {
        Some((head, tail)) => {
            let bounds = tail
                .trim_end()
                .strip_suffix(')')
                .ok_or("unclosed `range(`")?;
            let bounds = numbers(bounds)?;
            let [lo, hi] = bounds[..] else {
                return Err("`range` takes two numbers".to_owned());
            };
            (head, Some([lo, hi]))
        }
        None => (rest, None),
    };
    let (kind, default) = match rest.split_once('=') {
        Some((kind, default)) => (kind.trim(), Some(default.trim())),
        None => (rest.trim(), None),
    };
    let kind = ParamKind::parse(kind).ok_or_else(|| {
        format!("unknown kind `{kind}` (float, vec2, vec3, vec4, color, texture)")
    })?;

    let mut param = ShaderParam {
        name: name.to_owned(),
        kind,
        default: [0.0; 4],
        texture: TextureDefault::White,
        range,
        offset: 0,
    };
    if kind == ParamKind::Texture {
        param.texture = match default {
            None | Some("white") => TextureDefault::White,
            Some("black") => TextureDefault::Black,
            Some("normal") => TextureDefault::Normal,
            Some(other) => return Err(format!("texture default `{other}` (white, black, normal)")),
        };
        param.offset = before
            .iter()
            .filter(|p| p.kind == ParamKind::Texture)
            .count() as u32;
        if param.offset >= MAX_PARAM_TEXTURES {
            return Err(format!("more than {MAX_PARAM_TEXTURES} textures"));
        }
        return Ok(param);
    }

    if let Some(default) = default {
        let values = numbers(default.trim_start_matches('(').trim_end_matches(')'))?;
        if values.len() != kind.width() as usize && values.len() != 1 {
            return Err(format!("`{name}` takes {} numbers", kind.width()));
        }
        for (i, slot) in param
            .default
            .iter_mut()
            .take(kind.width() as usize)
            .enumerate()
        {
            *slot = values.get(i).copied().unwrap_or(values[0]);
        }
    }
    param.offset = before.iter().map(|p| p.kind.width()).sum();
    if param.offset + kind.width() > MAX_PARAM_SCALARS {
        return Err(format!("more than {MAX_PARAM_SCALARS} scalars"));
    }
    Ok(param)
}

fn numbers(list: &str) -> Result<Vec<f32>, String> {
    list.split(',')
        .map(|n| {
            n.trim()
                .parse::<f32>()
                .map_err(|_| format!("`{}` is not a number", n.trim()))
        })
        .collect()
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `SurfaceParams`, `surface_params(material_id)` and `sample_<name>(input, uv, scale)`.
pub(super) fn wgsl(params: &[ShaderParam]) -> String {
    let mut out = String::from("struct SurfaceParams {\n");
    let scalars: Vec<&ShaderParam> = params
        .iter()
        .filter(|p| p.kind != ParamKind::Texture)
        .collect();
    for param in &scalars {
        let _ = writeln!(out, "    {}: {},", param.name, param.kind.wgsl());
    }
    // WGSL has no empty struct.
    if scalars.is_empty() {
        out.push_str("    _none: f32,\n");
    }
    out.push_str("}\n\nfn surface_params(material_id: u32) -> SurfaceParams {\n");
    let _ = writeln!(out, "    let base = material_id * {MAX_PARAM_SCALARS}u;");
    out.push_str("    var p: SurfaceParams;\n");
    for param in &scalars {
        let component = |i: u32| format!("material_values[base + {}u]", param.offset + i);
        let value = match param.kind.width() {
            1 => component(0),
            width => format!(
                "{}({})",
                param.kind.wgsl(),
                (0..width).map(component).collect::<Vec<_>>().join(", ")
            ),
        };
        let _ = writeln!(out, "    p.{} = {value};", param.name);
    }
    out.push_str("    return p;\n}\n");
    for param in params.iter().filter(|p| p.kind == ParamKind::Texture) {
        // `scale` is whatever tiles the coordinate: the derivatives have to be tiled with it.
        let _ = write!(
            out,
            "\nfn sample_{name}(input: SurfaceInput, uv: vec2<f32>, scale: vec2<f32>) -> vec4<f32> {{\n    \
             let d = scale * input.mip_bias_scale;\n    \
             return textureSampleGrad({texture}, material_sampler, uv, input.ddx_uv * d, input.ddy_uv * d);\n}}\n",
            name = param.name,
            texture = TEXTURE_GLOBALS[param.offset as usize],
        );
    }
    out
}
