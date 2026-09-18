//! Reading what a hosted project said.

use kooch_core::LogBuffer;
use tracing::Level;

/// Records one line of a project's output.
pub(crate) fn record(buffer: &LogBuffer, line: &str) {
    match parse(line) {
        Some((level, target, message)) => buffer.push_project(level, target, message),
        // Cargo's own output, or anything else on that pipe. Stripped
        // because a child is arbitrary and an escape that survives gets
        // drawn as glyphs.
        None => {
            let line = kooch_core::strip_ansi(line);
            buffer.push_project(cargo_level(&line), "cargo", line)
        }
    }
}

/// Progress lines are a few hundred per rebuild and say nothing once the build is done; they sit
/// at DEBUG so the console keeps what went wrong and how it finished.
fn cargo_level(line: &str) -> Level {
    let verb = line.split_whitespace().next().unwrap_or_default();
    match verb {
        "Compiling" | "Checking" | "Locking" | "Updating" | "Downloading" | "Downloaded"
        | "Fresh" | "Adding" | "Blocking" => Level::DEBUG,
        _ => Level::INFO,
    }
}

/// Pulls the level, target and message out of a JSON log line.
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
mod tests;
