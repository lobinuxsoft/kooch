//! Reading what a hosted project said.
//!
//! The project logs as JSON when the editor hosts it — `KOOCH_LOG_FORMAT`,
//! set at launch — so its lines arrive with their own level, target and
//! fields rather than as one pre-formatted string.
//!
//! # Why not just forward the text
//!
//! That is what this replaced, and it lost everything that makes a log
//! usable. A formatted line is opaque: the editor wrapped the project's
//! whole line, timestamp and level and all, inside a line of its own, so
//! the Console showed `INFO` twice, attributed every line to the
//! forwarding module, and could not filter a project's warnings from its
//! chatter because to the editor they were all the same `info`.
//!
//! # Not everything on that pipe is ours
//!
//! Cargo builds the project on the same stdout: `Compiling`, `Finished`,
//! warnings, and whatever a build script prints. None of it is JSON and
//! all of it is worth showing, so anything that does not parse is kept as
//! plain text rather than dropped.

use kooch_core::LogBuffer;
use tracing::Level;

/// Records one line of a project's output.
pub(crate) fn record(buffer: &LogBuffer, line: &str) {
    match parse(line) {
        Some((level, target, message)) => buffer.push_project(level, target, message),
        // Cargo's own output, or anything else on that pipe. Stripped
        // because a child is arbitrary and an escape that survives gets
        // drawn as glyphs.
        None => buffer.push_project(Level::INFO, "cargo", kooch_core::strip_ansi(line)),
    }
}

/// Pulls the level, target and message out of a JSON log line.
///
/// `None` for anything that is not one, which is the caller's cue to keep
/// it as text.
fn parse(line: &str) -> Option<(Level, String, String)> {
    let value: kooch_remote::serde_json::Value =
        kooch_remote::serde_json::from_str(line.trim()).ok()?;
    let object = value.as_object()?;

    let level = match object.get("level")?.as_str()? {
        "ERROR" => Level::ERROR,
        "WARN" => Level::WARN,
        "INFO" => Level::INFO,
        "DEBUG" => Level::DEBUG,
        "TRACE" => Level::TRACE,
        _ => return None,
    };
    let target = object
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("project")
        .to_owned();
    let message = object
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();

    // The structured fields, appended the way the text formatter shows
    // them. `a sensor was entered a=8 b=9` is one line whether it arrived
    // as text or as JSON, so the two sources read alike.
    let mut extras: Vec<String> = object
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "level" | "target" | "message" | "timestamp" | "filename" | "line_number"
            )
        })
        .map(|(key, value)| match value.as_str() {
            Some(text) => format!("{key}={text}"),
            None => format!("{key}={value}"),
        })
        .collect();
    extras.sort();

    let message = match extras.is_empty() {
        true => message,
        false => format!("{message} {}", extras.join(" ")),
    };
    Some((level, target, message))
}

#[cfg(test)]
mod tests {
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
}
