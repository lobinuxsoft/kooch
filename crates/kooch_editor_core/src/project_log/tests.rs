use super::*;

#[test]
fn a_json_line_keeps_its_level_target_and_fields() {
    let line = r#"{"timestamp":"2026-07-27T05:00:09Z","level":"WARN","target":"kooch_physics","message":"a joint is waiting","entity":7}"#;
    let (level, target, message) = parse(line).expect("parses");

    assert_eq!(level, Level::WARN);
    assert_eq!(target, "kooch_physics");
    assert_eq!(message, "a joint is waiting entity=7");
}

/// Cargo shares the pipe and prints no JSON. Dropping it would hide
/// the build, which is most of what a first Play shows.
#[test]
fn cargos_output_survives_as_text() {
    assert!(parse("   Compiling kooch_physics v0.1.0").is_none());
}

/// A line that is JSON but not a log line is not one either.
#[test]
fn unrelated_json_is_not_a_log_line() {
    assert!(parse(r#"{"hello":"world"}"#).is_none());
    assert!(parse(r#"{"level":"SHOUT","message":"x"}"#).is_none());
}

/// Fields are ordered so the same event reads the same way twice —
/// a JSON object has no order of its own.
#[test]
fn fields_are_ordered_deterministically() {
    let line = r#"{"level":"INFO","target":"t","message":"m","b":2,"a":1}"#;
    let (_, _, message) = parse(line).expect("parses");
    assert_eq!(message, "m a=1 b=2");
}

/// An escape that survives is drawn as glyphs by whoever renders it,
/// and cargo is not the only thing that might colourise.
#[test]
fn text_lines_are_stripped_of_escapes() {
    let buffer = LogBuffer::new();
    record(&buffer, "\u{1b}[32m   Compiling\u{1b}[0m kooch_core");

    let entry = &buffer.snapshot()[0];
    assert_eq!(entry.message, "   Compiling kooch_core");
    assert!(entry.is_from_project(), "cargo's output is the project's");
}
