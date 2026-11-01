//! An element's `style` attribute as the UI Builder edits it: declarations in the written order.
//! Setting a side splits the shorthand that covers it in place, so a later shorthand never
//! overrides the field that was just edited.

use super::tokens::components;

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineStyle {
    declarations: Vec<(String, String)>,
}

impl InlineStyle {
    /// `a: b; c: d`, newlines allowed; a line without `:` is dropped.
    pub fn parse(text: &str) -> Self {
        let declarations = text
            .split([';', '\n'])
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .filter(|(name, value)| !name.is_empty() && !value.is_empty())
            .collect();
        Self { declarations }
    }

    pub fn write(&self) -> String {
        self.declarations
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The value that wins, a shorthand's side included.
    pub fn get(&self, name: &str) -> Option<String> {
        self.declarations.iter().rev().find_map(|(own, value)| {
            if own == name {
                return Some(value.clone());
            }
            expand(own, value)?
                .into_iter()
                .find(|(side, _)| side == name)
                .map(|(_, value)| value)
        })
    }

    /// `None` removes it, from any shorthand too.
    pub fn set(&mut self, name: &str, value: Option<&str>) {
        let mut at = 0;
        while at < self.declarations.len() {
            let (own, written) = &self.declarations[at];
            match expand(own, written) {
                Some(sides) if sides.iter().any(|(side, _)| side == name) => {
                    let count = sides.len();
                    self.declarations.splice(at..=at, sides);
                    at += count;
                }
                _ => at += 1,
            }
        }
        // Where the last one was, once the ones before it are gone.
        let last = self.declarations.iter().rposition(|(own, _)| own == name);
        let before = last.map(|last| {
            let gone = self.declarations[..last]
                .iter()
                .filter(|(own, _)| own == name);
            last - gone.count()
        });
        self.declarations.retain(|(own, _)| own != name);
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            let at = before.unwrap_or(self.declarations.len());
            self.declarations
                .insert(at, (name.to_owned(), value.to_owned()));
        }
    }
}

/// The longhands a shorthand stands for; `None` for a longhand or an unknown shape.
fn expand(name: &str, value: &str) -> Option<Vec<(String, String)>> {
    let side = |side: &str| match name {
        "border-width" => format!("border-{side}-width"),
        _ => format!("{name}-{side}"),
    };
    match name {
        "margin" | "padding" | "border-width" => {
            let parts = components(value);
            let [top, right, bottom, left] = match parts.as_slice() {
                [all] => [all, all, all, all],
                [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
                [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
                [top, right, bottom, left] => [top, right, bottom, left],
                _ => return None,
            };
            Some(
                SIDES
                    .iter()
                    .zip([top, right, bottom, left])
                    .map(|(name, value)| (side(name), value.clone()))
                    .collect(),
            )
        }
        "border" => {
            let mut out = Vec::new();
            for part in components(value) {
                if part.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                    out.extend(SIDES.map(|name| (format!("border-{name}-width"), part.clone())));
                } else {
                    out.push(("border-color".to_owned(), part));
                }
            }
            Some(out)
        }
        _ => None,
    }
}

#[test]
fn a_side_splits_its_shorthand() {
    let mut style = InlineStyle::parse("margin-top: 1px; margin: 2px 4px; color: red");
    assert_eq!(style.get("margin-left").as_deref(), Some("4px"));
    style.set("margin-top", Some("8px"));
    assert_eq!(style.get("margin-top").as_deref(), Some("8px"));
    assert_eq!(
        style.write(),
        "margin-top: 8px; margin-right: 4px; margin-bottom: 2px; margin-left: 4px; color: red"
    );
}
