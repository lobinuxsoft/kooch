//! Graph → `.shader`: the parameters it declares, one `let` per node, and the graph itself in the
//! block comment that rides above them (#1159).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use egui_snarl::{InPinId, OutPinId};

use super::{Graph, Node, embed};

mod emit;
mod helpers;

use emit::Body;
use helpers::helpers;

/// The whole `.shader` a graph generates: header, parameters, body and the graph itself.
pub(crate) fn generate(graph: &Graph) -> Result<String, String> {
    let (output, kind) = graph
        .node_ids()
        .find_map(|(id, node)| Some((id, node.output_kind()?.to_owned())))
        .ok_or("the graph has no Output node")?;
    let unlit = kind == "unlit";
    let post = kind == "post_process";
    let transparent = kind == "transparent";

    let wires: HashMap<InPinId, OutPinId> = graph.wires().map(|(from, to)| (to, from)).collect();
    let mut body = Body {
        graph,
        wires,
        names: HashMap::new(),
        visiting: HashSet::new(),
        lines: String::new(),
        next: 0,
        locals: 0,
        post,
    };
    // What an unconnected output falls back to. 🔴 Not zero for the normal: `normalize` of it is
    // NaN, which naga refuses outright.
    let surface: &[&str] = &[
        "vec4<f32>(0.0)",
        "vec4<f32>(normalize(input.world_normal), 0.0)",
        "vec4<f32>(0.0)",
        "vec4<f32>(0.5)",
        "vec4<f32>(0.0)",
    ];
    // Alpha, then the clip it is tested against (#452).
    let fallbacks: Vec<&str> = if post {
        vec!["vec4<f32>(0.0)", "vec4<f32>(1.0)"]
    } else if unlit {
        vec!["vec4<f32>(0.0)", "vec4<f32>(1.0)", "vec4<f32>(0.0)"]
    } else {
        [surface, &["vec4<f32>(1.0)", "vec4<f32>(0.0)"]].concat()
    };
    // Masked only when the clip is wired: an unwired clip would rasterise in the slower bin for
    // nothing.
    let clip = (!post).then_some(fallbacks.len() - 1).filter(|&input| {
        body.wires.contains_key(&InPinId {
            node: output,
            input,
        })
    });
    let outputs: Vec<String> = fallbacks
        .iter()
        .enumerate()
        .map(|(input, fallback)| body.input_or(output, input, fallback))
        .collect::<Result<_, _>>()?;

    let mut source = String::new();
    let _ = writeln!(source, "// kind: {kind}");
    let _ = writeln!(source, "{}", embed(graph)?);
    let _ = writeln!(
        source,
        "// Written by the shader graph. Edit it there: saving the graph replaces this file.\n"
    );
    source.push_str(&declarations(graph));
    source.push_str(&helpers(graph));
    let (function, returns) = match (unlit, post) {
        (true, _) => ("unlit", "UnlitOutput"),
        (_, true) => ("post_process", "vec4<f32>"),
        _ => ("surface", "SurfaceOutput"),
    };
    let _ = writeln!(source, "fn {function}(input: SurfaceInput) -> {returns} {{");
    if graph.node_ids().any(|(_, n)| n.declared().is_some()) {
        let _ = writeln!(source, "    let p = surface_params(input.material_id);");
    }
    source.push_str(&body.lines);
    if post {
        let _ = writeln!(
            source,
            "    return vec4<f32>({}.rgb, {}.x);\n}}",
            outputs[0], outputs[1]
        );
        return finish(source);
    }
    let _ = writeln!(source, "    var out: {returns};");
    if unlit {
        let _ = writeln!(source, "    out.color = {}.rgb;", outputs[0]);
        let _ = writeln!(source, "    out.alpha = {}.x;", outputs[1]);
    } else {
        let _ = writeln!(source, "    out.base_color = {}.rgb;", outputs[0]);
        let _ = writeln!(source, "    out.normal = normalize({}.xyz);", outputs[1]);
        let _ = writeln!(source, "    out.metallic = {}.x;", outputs[2]);
        let _ = writeln!(source, "    out.roughness = {}.x;", outputs[3]);
        let _ = writeln!(source, "    out.emissive = {}.rgb;", outputs[4]);
        if transparent || clip.is_some() {
            let _ = writeln!(source, "    out.alpha = {}.x;", outputs[5]);
        }
    }
    if let Some(input) = clip {
        let _ = writeln!(source, "    out.alpha_clip = {}.x;", outputs[input]);
    }
    let _ = writeln!(source, "    return out;\n}}");
    finish(source)
}

/// 🔴 The graph never writes a file the engine cannot read. A shader that fails to parse is written
/// all the same, fails to reload, and leaves every reader — the Inspector above all — showing the
/// parameters from before it, with nothing on screen saying why (#1159).
fn finish(source: String) -> Result<String, String> {
    match kooch_render::material::Shader::parse(&source) {
        Err(error) => Err(format!("{error}")),
        Ok(_) => Ok(source),
    }
}

/// `struct SurfaceParams`, `SURFACE_DEFAULTS` and the texture declarations the graph's nodes ask
/// for, in the order the nodes were added.
fn declarations(graph: &Graph) -> String {
    let mut out = String::new();
    let params: Vec<_> = graph
        .node_ids()
        .filter_map(|(_, node)| node.declared())
        .collect();
    if !params.is_empty() {
        out.push_str("struct SurfaceParams {\n");
        for param in &params {
            let hint = match param.hint.as_str() {
                "" => String::new(),
                hint => format!("  // {hint}"),
            };
            let _ = writeln!(out, "    {}: {},{hint}", param.name, wgsl_type(param.width));
        }
        out.push_str("}\n\nconst SURFACE_DEFAULTS = SurfaceParams(\n");
        for param in &params {
            let _ = writeln!(
                out,
                "    {},  // {}",
                literal(param.width, param.default),
                param.name
            );
        }
        out.push_str(");\n\n");
    }
    for (_, node) in graph.node_ids() {
        if let Node::Texture { name, fallback, .. } = node {
            let _ = writeln!(out, "var {name}: texture_2d<f32>;  // @default({fallback})");
        }
    }
    if graph
        .node_ids()
        .any(|(_, n)| matches!(n, Node::Texture { .. }))
    {
        out.push('\n');
    }
    out
}

/// A swizzle as WGSL, widened to four components. A pattern naming anything but `x`, `y`, `z` or `w`
/// is refused rather than written: the file would fail to parse with the graph's own name on it.
fn swizzled(value: &str, pattern: &str) -> Result<String, String> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.len() > 4 || !pattern.chars().all(|c| "xyzw".contains(c)) {
        return Err(format!("`{pattern}` is not a swizzle of x, y, z or w"));
    }
    Ok(match pattern.len() {
        1 => format!("vec4<f32>({value}.{pattern})"),
        2 => format!("vec4<f32>({value}.{pattern}, 0.0, 0.0)"),
        3 => format!("vec4<f32>({value}.{pattern}, 0.0)"),
        _ => format!("{value}.{pattern}"),
    })
}

/// Four numbers as a WGSL `vec4<f32>`, each written so it always reads as a float.
fn vec4_literal(value: [f32; 4]) -> String {
    let parts: Vec<String> = value.iter().map(|v| format!("{v:?}")).collect();
    format!("vec4<f32>({})", parts.join(", "))
}

fn wgsl_type(width: u32) -> &'static str {
    match width {
        1 => "f32",
        2 => "vec2<f32>",
        3 => "vec3<f32>",
        _ => "vec4<f32>",
    }
}

/// A member's starting value, as WGSL writes it.
fn literal(width: u32, value: [f32; 4]) -> String {
    let numbers: Vec<String> = value
        .iter()
        .take(width.max(1) as usize)
        .map(|v| format!("{v:?}"))
        .collect();
    match width {
        1 => numbers[0].clone(),
        _ => format!("{}({})", wgsl_type(width), numbers.join(", ")),
    }
}
