//! A hand-written function as a node: its pins read from its signature (#1807).

use serde::{Deserialize, Serialize};

use super::codegen::widened;
use super::{Node, Width};

/// One value a function takes or gives back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FunctionPin {
    pub(crate) name: String,
    /// Components, 1..=4: a wire is a `vec4<f32>` and the call narrows it.
    pub(crate) width: u8,
    pub(crate) default: [f32; 4],
}

/// Every `///` function in `source` as a node, with what its comment says beside it. A function
/// without one is the file's own helper, and an entry point like `surface` is no node.
pub(crate) fn functions(path: &str, source: &str) -> Vec<(Node, String)> {
    let mut found = Vec::new();
    let mut about = String::new();
    let lines: Vec<&str> = source.lines().collect();
    for (at, line) in lines.iter().enumerate() {
        if let Some(text) = line.strip_prefix("///") {
            about.push_str(text.trim());
            about.push(' ');
            continue;
        }
        if line.starts_with("fn ")
            && !about.is_empty()
            && let Some(node) = lines.get(at..).and_then(|rest| parse(path, source, rest))
        {
            found.push((node, about.trim().to_owned()));
        }
        about.clear();
    }
    found
}

/// The node for the function starting at `lines[0]`, when every type in it fits a wire.
fn parse(path: &str, source: &str, lines: &[&str]) -> Option<Node> {
    let end = lines.iter().position(|line| line.contains('{'))?;
    let head = lines.get(..=end)?;
    let first = head.first()?.strip_prefix("fn ")?;
    let (function, _) = first.split_once('(')?;
    let mut text = String::new();
    let mut hints = Vec::new();
    for line in head {
        let (code, comment) = line.split_once("//").unwrap_or((line, ""));
        text.push_str(code);
        // Written after a parameter's comma, so it belongs to the one that line declares.
        hints.push((code.to_owned(), comment.to_owned()));
    }
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let parameters = text.get(open + 1..close)?;
    let returns = text.get(close + 1..)?.split('{').next()?;
    let returns = returns.trim().strip_prefix("->")?.trim();

    let mut reads = None;
    let mut inputs = Vec::new();
    for parameter in parameters.split(',').filter(|p| !p.trim().is_empty()) {
        let (name, kind) = parameter.split_once(':')?;
        let (name, kind) = (name.trim(), kind.trim());
        match kind {
            "UiInput" if inputs.is_empty() => reads = Some("ui".to_owned()),
            "SurfaceInput" if inputs.is_empty() => reads = Some("surface".to_owned()),
            _ => inputs.push(FunctionPin {
                name: name.to_owned(),
                width: width_of(kind)?,
                default: default_of(name, &hints),
            }),
        }
    }
    let (outputs, fields) = match width_of(returns) {
        Some(width) => (vec![pin("out", width)], false),
        None => (fields_of(source, returns)?, true),
    };
    Some(Node::Function {
        path: path.to_owned(),
        function: function.trim().to_owned(),
        inputs,
        outputs,
        fields,
        reads,
    })
}

impl Node {
    /// Output `index` of a function returning a struct held in `value`, as a wire.
    pub(crate) fn field_of(&self, index: usize, value: &str) -> Option<String> {
        let Self::Function {
            outputs,
            fields: true,
            ..
        } = self
        else {
            return None;
        };
        let pin = outputs.get(index)?;
        Some(widened(&format!("{value}.{}", pin.name), pin.width))
    }

    /// How wide input pin `input` is, where the node knows.
    pub(crate) fn pin_width(&self, input: usize) -> Width {
        let Self::Function { inputs, .. } = self else {
            return Width::Any;
        };
        match inputs.get(input).map(|pin| pin.width) {
            Some(1) => Width::One,
            Some(2) => Width::Two,
            Some(3) => Width::Three,
            Some(4) => Width::Four,
            _ => Width::Any,
        }
    }
}

fn pin(name: &str, width: u8) -> FunctionPin {
    FunctionPin {
        name: name.to_owned(),
        width,
        default: [0.0; 4],
    }
}

fn width_of(kind: &str) -> Option<u8> {
    match kind {
        "f32" => Some(1),
        "vec2<f32>" | "vec2f" => Some(2),
        "vec3<f32>" | "vec3f" => Some(3),
        "vec4<f32>" | "vec4f" => Some(4),
        _ => None,
    }
}

/// `@default(a, b)` from the line declaring `name`: a value a wire does not bring.
fn default_of(name: &str, hints: &[(String, String)]) -> [f32; 4] {
    let declares = |code: &str| {
        code.split([',', '('])
            .any(|part| part.split(':').next().is_some_and(|n| n.trim() == name))
    };
    let mut value = [0.0; 4];
    let hint = hints
        .iter()
        .find(|(code, _)| declares(code))
        .and_then(|(_, comment)| comment.split("@default(").nth(1))
        .and_then(|rest| rest.split(')').next());
    let Some(hint) = hint else {
        return value;
    };
    let numbers: Vec<f32> = hint
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    match numbers.as_slice() {
        [only] => value = [*only; 4],
        many => value.iter_mut().zip(many).for_each(|(v, n)| *v = *n),
    }
    value
}

/// The members of `struct kind` in `source`, when each fits a wire.
fn fields_of(source: &str, kind: &str) -> Option<Vec<FunctionPin>> {
    let body = source.split(&format!("struct {kind} {{")).nth(1)?;
    let body = body.split('}').next()?;
    body.lines()
        .map(|line| line.split("//").next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, kind) = line.trim_end_matches(',').split_once(':')?;
            Some(pin(name.trim(), width_of(kind.trim())?))
        })
        .collect()
}

#[cfg(test)]
mod tests;
