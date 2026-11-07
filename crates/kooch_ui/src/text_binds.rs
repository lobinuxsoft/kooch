//! A label whose text names fields, `[HP {Hud.hp:>3%}]` (#1852): the holes follow the fields, read
//! through reflection like `bind:param`. The spec is Python's, which `strfmt` parses, plus `%`.

use kooch_ecs::ReflectValue;
use strfmt::{Alignment, Formatter, strfmt_map};

use crate::UiNode;
use crate::fields::FieldPath;

#[derive(Debug, Clone, PartialEq)]
pub struct TextBind {
    pub node: UiNode,
    pub template: Box<str>,
    /// One per hole, in the order they appear.
    pub holes: Vec<FieldPath>,
    /// What each hole showed last: the text is formatted again only when one of them moves.
    pub(crate) shown: Vec<Option<ReflectValue>>,
}

impl TextBind {
    /// `None` for plain text: one with no hole that names a `Type.field`, or with unbalanced braces.
    pub fn parse(node: UiNode, text: &str) -> Option<Self> {
        let mut holes = Vec::new();
        let mut plain = false;
        strfmt_map(text, |fmt: Formatter| {
            match FieldPath::parse(fmt.key) {
                Some(path) => holes.push(path),
                None => plain = true,
            }
            Ok(())
        })
        .ok()?;
        if plain || holes.is_empty() {
            return None;
        }
        let shown = vec![None; holes.len()];
        Some(Self {
            node,
            template: text.into(),
            holes,
            shown,
        })
    }

    /// The template with `shown` in its holes; a hole with no value keeps its `{…}`.
    pub fn format(&self) -> Option<String> {
        let mut next = 0;
        strfmt_map(&self.template, |fmt: Formatter| {
            let value = self.shown.get(next).and_then(Option::as_ref);
            next += 1;
            hole(fmt, value)
        })
        .ok()
    }
}

fn hole(mut fmt: Formatter, value: Option<&ReflectValue>) -> strfmt::Result<()> {
    let Some((text, number)) = value.and_then(|value| shown(value, fmt.ty(), fmt.precision()))
    else {
        return fmt.skip();
    };
    fmt.set_default_align(if number {
        Alignment::Right
    } else {
        Alignment::Left
    });
    // The precision went into `text`; on a string it would cut it.
    fmt.set_precision(None);
    fmt.str_unchecked(&text)
}

/// A value as text, and whether it is a number (they align right). `%` is ×100, whole by default.
fn shown(
    value: &ReflectValue,
    ty: Option<char>,
    precision: Option<usize>,
) -> Option<(String, bool)> {
    let whole = |v: i128| match ty {
        Some('%') => format!("{}%", v.saturating_mul(100)),
        _ => v.to_string(),
    };
    let real = |v: f64| match ty {
        Some('%') => format!("{:.*}%", precision.unwrap_or(0), v * 100.0),
        _ => match precision {
            Some(p) => format!("{v:.p$}"),
            None => v.to_string(),
        },
    };
    Some(match *value {
        ReflectValue::F32(v) => (real(v as f64), true),
        ReflectValue::F64(v) => (real(v), true),
        ReflectValue::U8(v) => (whole(v.into()), true),
        ReflectValue::U16(v) => (whole(v.into()), true),
        ReflectValue::U32(v) => (whole(v.into()), true),
        ReflectValue::U64(v) => (whole(v.into()), true),
        ReflectValue::I8(v) => (whole(v.into()), true),
        ReflectValue::I16(v) => (whole(v.into()), true),
        ReflectValue::I32(v) => (whole(v.into()), true),
        ReflectValue::I64(v) => (whole(v.into()), true),
        ReflectValue::Bool(v) => (v.to_string(), false),
        ReflectValue::String(ref v) => (v.clone(), false),
        _ => return None,
    })
}
