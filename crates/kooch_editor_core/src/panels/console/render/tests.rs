use super::*;
fn entry(level: Level, target: &str, message: &str) -> LogEntry {
    LogEntry {
        seq: 0,
        level,
        target: target.to_owned(),
        message: message.to_owned(),
        from_project: false,
    }
}

fn from_project(level: Level, target: &str, message: &str) -> LogEntry {
    LogEntry {
        from_project: true,
        ..entry(level, target, message)
    }
}

/// The numbers are what anyone scans a log line for, so they have to
/// come out of the prose.
#[test]
fn fields_are_separated_from_prose() {
    let parts = split_fields("a sensor was entered a=8 b=9");
    let rendered: Vec<String> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => format!("text:{t}"),
            Part::Field { key, value } => format!("field:{key}{value}"),
        })
        .collect();
    assert_eq!(
        rendered,
        vec!["text:a sensor was entered", "field:a=8", "field:b=9"],
    );
}

/// A message with nothing structured in it stays one run, or wrapping
/// breaks at every space.
#[test]
fn prose_without_fields_is_one_part() {
    let parts = split_fields("scene loaded from disk");
    assert_eq!(parts.len(), 1);
    assert!(matches!(parts[0], Part::Text("scene loaded from disk")));
}

/// An equals sign inside prose must not eat the line — colouring it
/// oddly is a better failure than losing it.
#[test]
fn an_equals_in_prose_does_not_swallow_anything() {
    let parts = split_fields("solved x=1 for the case");
    let text: String = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => (*t).to_owned(),
            Part::Field { key, value } => format!("{key}{value}"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(text, "solved x=1 for the case");
}

#[test]
fn an_empty_message_produces_nothing() {
    assert!(split_fields("").is_empty());
}

#[test]
fn a_long_module_path_is_shortened_to_its_last_segment() {
    assert_eq!(
        short_target("kooch_editor_core::actions::handlers"),
        "handlers"
    );
    assert_eq!(short_target("physics_smoke"), "physics_smoke");
}
