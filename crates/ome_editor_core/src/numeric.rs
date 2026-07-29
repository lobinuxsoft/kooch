//! Numeric entry that accepts arithmetic, not just literals.
//!
//! Half the numbers an author types are the result of a calculation they
//! did in their head: half of 9, a third of the room's width, 45 degrees
//! plus a nudge. Typing `4.5` instead of `9/2` is the author doing the
//! machine's job, and it loses the intent — six months later `4.5` says
//! nothing about where it came from.
//!
//! # Why a crate and not a parser
//!
//! Arithmetic looks like an afternoon and is not: precedence, unary minus,
//! parentheses, scientific notation, the difference between `-2^2` and
//! `(-2)^2`. [`exmex`] is MIT/Apache, still released this month, and its
//! two dependencies (`regex`, `smallvec`) were already in this tree — so
//! adopting it costs nothing that was not already being paid.
//!
//! `evalexpr` was the obvious candidate and was rejected twice over: it is
//! AGPL-3.0, which this project cannot take, and its `/` is integer
//! division when both sides are integers — `9/2` would answer `4`, which
//! is worse than refusing to answer.

/// A [`DragValue`](egui::DragValue) whose text entry evaluates arithmetic.
///
/// Everything else about it is unchanged, so callers keep adding their own
/// `.speed()` and `.range()`. The range still clamps the *result*: typing
/// `600/2` into a field that stops at 255 gives 255, exactly as typing
/// `300` would.
pub(crate) fn drag<Num: egui::emath::Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value).custom_parser(evaluate)
}

/// Evaluates what the author typed, or `None` if it is not a number.
///
/// A plain literal never reaches the expression parser: it is the common
/// case, and `f64::from_str` is both faster and stricter about the forms
/// that already worked before this existed.
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
///
/// Both come from egui's own default parser: whitespace so that a pasted
/// `1 000` and a typed `9 / 2` behave, and U+2212 because that is what a
/// system keyboard layout or a copied value from elsewhere produces, and
/// no expression parser recognises it.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::evaluate;

    /// The case the user asked for.
    #[test]
    fn a_division_is_evaluated_as_a_real_one() {
        assert_eq!(evaluate("9/2"), Some(4.5));
    }

    /// What already worked has to keep working, unchanged — this widget is
    /// how every number in the editor is typed.
    #[test]
    fn plain_literals_are_untouched() {
        assert_eq!(evaluate("4.5"), Some(4.5));
        assert_eq!(evaluate("-3"), Some(-3.0));
        assert_eq!(evaluate("1e-5"), Some(1e-5));
        assert_eq!(evaluate("0"), Some(0.0));
    }

    #[test]
    fn precedence_and_parentheses_hold() {
        assert_eq!(evaluate("1+2*3"), Some(7.0));
        assert_eq!(evaluate("(1+2)*3"), Some(9.0));
    }

    /// egui's own parser accepts these, so the replacement has to as well
    /// or pasting a value that used to work starts failing.
    #[test]
    fn whitespace_and_the_typographic_minus_are_accepted() {
        assert_eq!(evaluate(" 9 / 2 "), Some(4.5));
        assert_eq!(evaluate("\u{2212}3"), Some(-3.0));
    }

    /// Refusing leaves the previous value in place. Answering `inf` would
    /// write a number no field on screen can mean.
    #[test]
    fn nonsense_and_non_finite_results_are_refused() {
        assert_eq!(evaluate(""), None);
        assert_eq!(evaluate("   "), None);
        assert_eq!(evaluate("hello"), None);
        assert_eq!(evaluate("1/0"), None);
    }
}
