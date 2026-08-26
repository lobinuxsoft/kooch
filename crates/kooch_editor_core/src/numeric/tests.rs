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
