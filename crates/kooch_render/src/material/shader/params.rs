//! A surface's parameters, read from its own WGSL (#1158): the members of `struct SurfaceParams`,
//! like an HLSL `cbuffer`, and each `var name: texture_2d<f32>;`, like a `Texture2D`. Starting
//! values are WGSL too — `const SURFACE_DEFAULTS = SurfaceParams(...);`, evaluated by naga. A comment
//! after a declaration may hint `@color` or `@range(lo, hi)` for the editor, and a texture's
//! `@default(white|black|normal)`; the shader compiles the same without them.

use std::fmt::Write;

/// Scalars a material holds for its shader, packed into `material_values` (16 `f32` per slot).
pub const MAX_PARAM_SCALARS: u32 = 16;

/// Textures a material binds for its shader.
pub const MAX_PARAM_TEXTURES: u32 = 4;

/// Group-4 binding of each texture slot, in declaration order; 3 is the sampler.
const TEXTURE_BINDINGS: [u32; MAX_PARAM_TEXTURES as usize] = [0, 1, 2, 4];

/// What a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Float,
    Vec2,
    Vec3,
    Vec4,
    /// A `vec4<f32>` hinted `@color`: the Inspector edits it with a colour picker.
    Color,
    Texture,
}

impl ParamKind {
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
    /// First scalar in the material's block, or the texture slot for a texture.
    pub offset: u32,
}

/// A surface as the engine compiles it: the author's source with texture bindings filled in on
/// the same lines, so an error's line number is still the author's.
pub(super) struct Read {
    pub source: String,
    pub params: Vec<ShaderParam>,
}

/// Reads a surface's parameters. Errors carry a 1-based line.
pub(super) fn read(source: &str) -> Result<Read, (usize, String)> {
    let mut params: Vec<ShaderParam> = Vec::new();
    let mut lines = Vec::new();
    let mut in_struct = false;
    // The declarations naga evaluates the defaults from, and the line the constant starts on.
    let mut struct_text = String::new();
    let mut defaults_text = String::new();
    let mut defaults_line = 0;
    let mut in_defaults = false;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let (code, comment) = match raw.split_once("//") {
            Some((code, comment)) => (code, comment),
            None => (raw, ""),
        };
        let code = code.trim();
        // Not WGSL: the line wgsl-analyzer resolves through `customImports`. Blanked, so line
        // numbers stay the author's.
        if let Some(name) = code.strip_prefix("#import") {
            if name.trim() != "kooch::surface" {
                return Err((
                    line,
                    format!("unknown import `{}` (known: kooch::surface)", name.trim()),
                ));
            }
            lines.push(String::new());
            continue;
        }
        if code.contains("@group") || code.contains("@binding") {
            return Err((
                line,
                "a surface declares no bindings — the engine assigns them".to_owned(),
            ));
        }

        let fields = if in_struct {
            Some(code)
        } else {
            code.strip_prefix("struct SurfaceParams")
                .map(|rest| rest.split_once('{').map_or("", |(_, fields)| fields))
        };
        if in_defaults || code.starts_with("const SURFACE_DEFAULTS") {
            if !in_defaults {
                defaults_line = line;
            }
            in_defaults = !code.contains(';');
            let _ = writeln!(defaults_text, "{code}");
            lines.push(raw.to_owned());
            continue;
        }

        if let Some(fields) = fields {
            let _ = writeln!(struct_text, "{code}");
            in_struct = !fields.contains('}');
            let fields = fields.split('}').next().unwrap_or_default();
            for field in fields.split(',').map(str::trim).filter(|f| !f.is_empty()) {
                let param = scalar(field, comment, &params).map_err(|e| (line, e))?;
                params.push(param);
            }
            lines.push(raw.to_owned());
            continue;
        }

        if let Some(declaration) = code.strip_prefix("var ")
            && declaration.contains("texture_2d")
        {
            let param = texture(declaration, comment, &params).map_err(|e| (line, e))?;
            let binding = TEXTURE_BINDINGS[param.offset as usize];
            let indent = &raw[..raw.len() - raw.trim_start().len()];
            lines.push(format!(
                "{indent}@group(4) @binding({binding}) {}",
                raw.trim_start()
            ));
            params.push(param);
            continue;
        }
        lines.push(raw.to_owned());
    }
    if in_struct {
        return Err((
            source.lines().count(),
            "`struct SurfaceParams` is not closed".to_owned(),
        ));
    }
    if !defaults_text.is_empty() {
        let values = evaluate_defaults(&format!("{struct_text}\n{defaults_text}"))
            .map_err(|e| (defaults_line, format!("SURFACE_DEFAULTS: {e}")))?;
        let expected: u32 = params.iter().map(|p| p.kind.width()).sum();
        if values.len() != expected as usize {
            return Err((
                defaults_line,
                format!(
                    "SURFACE_DEFAULTS holds {} numbers, SurfaceParams {expected}",
                    values.len()
                ),
            ));
        }
        for param in params.iter_mut().filter(|p| p.kind != ParamKind::Texture) {
            let at = param.offset as usize;
            let width = param.kind.width() as usize;
            param.default[..width].copy_from_slice(&values[at..at + width]);
        }
    }
    Ok(Read {
        source: lines.join("\n"),
        params,
    })
}

/// `SURFACE_DEFAULTS`' members flattened in declaration order, as naga evaluates the constant.
fn evaluate_defaults(declarations: &str) -> Result<Vec<f32>, String> {
    let module = naga::front::wgsl::parse_str(declarations).map_err(|e| e.message().to_owned())?;
    let (_, constant) = module
        .constants
        .iter()
        .find(|(_, c)| c.name.as_deref() == Some("SURFACE_DEFAULTS"))
        .ok_or("not a constant")?;
    let mut values = Vec::new();
    flatten(&module, constant.init, &mut values)?;
    Ok(values)
}

fn flatten(
    module: &naga::Module,
    expression: naga::Handle<naga::Expression>,
    values: &mut Vec<f32>,
) -> Result<(), String> {
    use naga::{Expression, Literal};
    match module.global_expressions[expression] {
        Expression::Literal(literal) => values.push(match literal {
            Literal::F32(v) => v,
            Literal::AbstractFloat(v) => v as f32,
            Literal::AbstractInt(v) => v as f32,
            Literal::I32(v) => v as f32,
            Literal::U32(v) => v as f32,
            _ => return Err("a default is a number".to_owned()),
        }),
        Expression::Compose { ref components, .. } => {
            for component in components {
                flatten(module, *component, values)?;
            }
        }
        Expression::Splat { size, value } => {
            let mut one = Vec::new();
            flatten(module, value, &mut one)?;
            for _ in 0..size as usize {
                values.extend_from_slice(&one);
            }
        }
        Expression::ZeroValue(ty) => values.resize(values.len() + zeros(module, ty), 0.0),
        _ => return Err("a default is a constant expression".to_owned()),
    }
    Ok(())
}

/// Scalars a zero value of `ty` spans.
fn zeros(module: &naga::Module, ty: naga::Handle<naga::Type>) -> usize {
    match module.types[ty].inner {
        naga::TypeInner::Vector { size, .. } => size as usize,
        naga::TypeInner::Struct { ref members, .. } => {
            members.iter().map(|m| zeros(module, m.ty)).sum()
        }
        _ => 1,
    }
}

/// One `name: type` member of `SurfaceParams`.
fn scalar(field: &str, hints: &str, before: &[ShaderParam]) -> Result<ShaderParam, String> {
    let (name, ty) = field
        .split_once(':')
        .ok_or_else(|| format!("`{field}` is not `name: type`"))?;
    let name = identifier(name.trim(), before)?;
    let mut kind = match ty.trim() {
        "f32" => ParamKind::Float,
        "vec2<f32>" | "vec2f" => ParamKind::Vec2,
        "vec3<f32>" | "vec3f" => ParamKind::Vec3,
        "vec4<f32>" | "vec4f" => ParamKind::Vec4,
        other => {
            return Err(format!(
                "`{name}: {other}` — a parameter is f32, vec2<f32>, vec3<f32> or vec4<f32>"
            ));
        }
    };
    if hint(hints, "@color").is_some() {
        if kind != ParamKind::Vec4 {
            return Err(format!("`{name}`: @color needs vec4<f32>"));
        }
        kind = ParamKind::Color;
    }
    let range = match hint(hints, "@range") {
        Some(bounds) if kind == ParamKind::Float => match numbers(bounds)?[..] {
            [lo, hi] => Some([lo, hi]),
            _ => return Err(format!("`{name}`: @range takes two numbers")),
        },
        Some(_) => return Err(format!("`{name}`: @range needs f32")),
        None => None,
    };
    if hint(hints, "@default").is_some() {
        return Err(format!(
            "`{name}`: a starting value goes in `const SURFACE_DEFAULTS = SurfaceParams(...);`"
        ));
    }
    let default = [0.0; 4];
    let offset: u32 = before.iter().map(|p| p.kind.width()).sum();
    if offset + kind.width() > MAX_PARAM_SCALARS {
        return Err(format!("more than {MAX_PARAM_SCALARS} scalars"));
    }
    Ok(ShaderParam {
        name,
        kind,
        default,
        texture: TextureDefault::White,
        range,
        offset,
    })
}

/// One `name: texture_2d<f32>;` after its `var`.
fn texture(declaration: &str, hints: &str, before: &[ShaderParam]) -> Result<ShaderParam, String> {
    let (name, ty) = declaration
        .split_once(':')
        .ok_or_else(|| format!("`var {declaration}` is not `var name: texture_2d<f32>;`"))?;
    let name = identifier(name.trim(), before)?;
    if ty.trim().trim_end_matches(';').trim() != "texture_2d<f32>" {
        return Err(format!("`{name}`: a texture parameter is texture_2d<f32>"));
    }
    let offset = before
        .iter()
        .filter(|p| p.kind == ParamKind::Texture)
        .count() as u32;
    if offset >= MAX_PARAM_TEXTURES {
        return Err(format!("more than {MAX_PARAM_TEXTURES} textures"));
    }
    let texture = match hint(hints, "@default").map(str::trim) {
        None | Some("white") => TextureDefault::White,
        Some("black") => TextureDefault::Black,
        Some("normal") => TextureDefault::Normal,
        Some(other) => {
            return Err(format!(
                "`{name}`: @default({other}) — white, black or normal"
            ));
        }
    };
    Ok(ShaderParam {
        name,
        kind: ParamKind::Texture,
        default: [0.0; 4],
        texture,
        range: None,
        offset,
    })
}

/// A hint's argument list, `""` for a bare `@color`, or `None` when absent.
fn hint<'a>(hints: &'a str, name: &str) -> Option<&'a str> {
    let at = hints.find(name)?;
    let rest = &hints[at + name.len()..];
    match rest.strip_prefix('(') {
        Some(args) => args.split_once(')').map(|(args, _)| args),
        None => Some(""),
    }
}

fn identifier(name: &str, before: &[ShaderParam]) -> Result<String, String> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(format!("`{name}` is not a WGSL identifier"));
    }
    if before.iter().any(|p| p.name == name) {
        return Err(format!("`{name}` is declared twice"));
    }
    Ok(name.to_owned())
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

/// What the engine composes ahead of the surface: `surface_params(material_id)` when there is a
/// `SurfaceParams`, and `surface_texture_dims()` for the mip-level debug view.
pub(super) fn generated(params: &[ShaderParam]) -> String {
    let mut out = String::new();
    let scalars: Vec<&ShaderParam> = params
        .iter()
        .filter(|p| p.kind != ParamKind::Texture)
        .collect();
    if !scalars.is_empty() {
        out.push_str("fn surface_params(material_id: u32) -> SurfaceParams {\n");
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
    }
    let dims = match params.iter().find(|p| p.kind == ParamKind::Texture) {
        Some(first) => format!("vec2<f32>(textureDimensions({}, 0))", first.name),
        None => "vec2<f32>(1.0)".to_owned(),
    };
    let _ = writeln!(
        out,
        "fn surface_texture_dims() -> vec2<f32> {{ return {dims}; }}"
    );
    out
}
