//! Numeric entry that accepts arithmetic, not just literals.

/// A [`DragValue`](egui::DragValue) whose text entry evaluates arithmetic.
pub(crate) fn drag<Num: egui::emath::Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value).custom_parser(evaluate)
}

/// Evaluates what the author typed, or `None` if it is not a number.
pub(crate) fn evaluate(text: &str) -> Option<f64> {
    let normalised = normalise(text);
    if normalised.is_empty() {
        return None;
    }
    if let Ok(value) = normalised.parse::<f64>() {
        return Some(value);
    }
    // A non-finite result is a refusal, not a value: `1/0` in a field for
    // a collider's radius is a typo, and writing `inf` into the scene
    // there would be a worse answer than leaving the number alone.
    exmex::eval_str::<f64>(&normalised)
        .ok()
        .filter(|v| v.is_finite())
}

/// Whitespace out, the typographic minus turned into the ASCII one.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect()
}

#[cfg(test)]
mod tests;
