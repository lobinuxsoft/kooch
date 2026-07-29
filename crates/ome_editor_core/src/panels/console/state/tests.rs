//! What the panel's copy of the log has to get right.

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

#[test]
fn the_level_filter_keeps_the_more_severe() {
    let state = ConsoleState {
        levels: errors_and_warnings(),
        ..Default::default()
    };
    assert!(state.shows(&entry(Level::ERROR, "t", "m")));
    assert!(state.shows(&entry(Level::WARN, "t", "m")));
    assert!(!state.shows(&entry(Level::INFO, "t", "m")));
}

#[test]
fn the_text_filter_matches_message_or_target() {
    let state = ConsoleState {
        filter: "joint".to_owned(),
        ..Default::default()
    };
    assert!(state.shows(&entry(Level::INFO, "other", "a joint broke")));
    assert!(state.shows(&entry(Level::INFO, "ome_joints", "something")));
    assert!(!state.shows(&entry(Level::INFO, "other", "something")));
}

#[test]
fn the_text_filter_ignores_case() {
    let state = ConsoleState {
        filter: "JOINT".to_owned(),
        ..Default::default()
    };
    assert!(state.shows(&entry(Level::INFO, "t", "a Joint broke")));
}

/// The allocation-free search has to agree with the one it replaced,
/// including at the ends of the string, which is where a windowing
/// implementation goes wrong.
#[test]
fn the_search_matches_at_both_ends() {
    assert!(contains_ignore_case("joint broke", "joint"));
    assert!(contains_ignore_case("a broken joint", "joint"));
    assert!(contains_ignore_case("joint", "joint"));
    assert!(!contains_ignore_case("join", "joint"));
    assert!(contains_ignore_case("anything", ""));
}

/// The filters compose; passing one is not enough.
#[test]
fn the_filters_are_combined() {
    let state = ConsoleState {
        levels: errors_and_warnings(),
        filter: "joint".to_owned(),
        ..Default::default()
    };
    assert!(
        !state.shows(&entry(Level::INFO, "t", "a joint broke")),
        "the level filter was ignored once the text matched",
    );
    assert!(
        !state.shows(&entry(Level::ERROR, "t", "something else")),
        "the text filter was ignored once the level matched",
    );
}

fn filled(lines: usize) -> LogBuffer {
    let buffer = LogBuffer::new();
    for i in 0..lines {
        buffer.push_project(Level::INFO, "test", format!("line {i}"));
    }
    buffer
}

#[test]
fn a_first_sync_takes_the_whole_log() {
    let mut state = ConsoleState::default();
    state.sync(&filled(5));
    assert_eq!(state.entries().len(), 5);
    assert_eq!(state.visible().len(), 5);
}

/// The point of the whole file: a redraw with nothing new must not
/// copy the log again.
#[test]
fn a_sync_with_nothing_new_keeps_what_it_had() {
    let buffer = filled(5);
    let mut state = ConsoleState::default();
    state.sync(&buffer);
    let before: Vec<u64> = state.entries().iter().map(|e| e.seq).collect();

    state.sync(&buffer);

    let after: Vec<u64> = state.entries().iter().map(|e| e.seq).collect();
    assert_eq!(before, after);
}

#[test]
fn new_lines_are_appended() {
    let buffer = filled(3);
    let mut state = ConsoleState::default();
    state.sync(&buffer);
    buffer.push_project(Level::INFO, "test", "the new one");

    state.sync(&buffer);

    assert_eq!(state.entries().len(), 4);
    assert_eq!(state.entries()[3].message, "the new one");
}

/// The buffer is bounded, so the panel's copy has to shrink with it or
/// it becomes the unbounded log the buffer exists to prevent.
#[test]
fn lines_the_buffer_dropped_leave_the_panel_too() {
    let buffer = filled(4);
    let mut state = ConsoleState::default();
    state.sync(&buffer);

    // Push past the cap by pushing far more than four.
    for i in 0..2100 {
        buffer.push_project(Level::INFO, "test", format!("more {i}"));
    }
    state.sync(&buffer);

    assert_eq!(state.entries().len(), buffer.len());
    assert!(
        state.entries().first().map(|e| e.message.as_str()) != Some("line 0"),
        "the panel is still holding a line the buffer dropped",
    );
}

/// Clear has to empty the panel, not leave it showing a log that no
/// longer exists.
#[test]
fn clearing_the_buffer_empties_the_panel() {
    let buffer = filled(5);
    let mut state = ConsoleState::default();
    state.sync(&buffer);
    buffer.clear();

    state.sync(&buffer);

    assert!(state.entries().is_empty());
    assert!(state.visible().is_empty());
}

/// Clear then log again: the sequence restarts above what the panel
/// holds, and the panel must not stitch the two together.
#[test]
fn a_cleared_and_refilled_buffer_is_taken_whole() {
    let buffer = filled(5);
    let mut state = ConsoleState::default();
    state.sync(&buffer);
    buffer.clear();
    buffer.push_project(Level::INFO, "test", "after the clear");

    state.sync(&buffer);

    assert_eq!(state.entries().len(), 1);
    assert_eq!(state.entries()[0].message, "after the clear");
}

/// The filtered view is cached, so it has to notice a filter change.
#[test]
fn changing_the_filter_rebuilds_the_view() {
    let buffer = LogBuffer::new();
    buffer.push_project(Level::INFO, "test", "a joint broke");
    buffer.push_project(Level::INFO, "test", "an asset loaded");

    let mut state = ConsoleState::default();
    state.sync(&buffer);
    assert_eq!(state.visible().len(), 2);

    state.filter = "joint".to_owned();
    state.sync(&buffer);
    assert_eq!(state.visible().len(), 1);

    state.levels = only(Level::ERROR);
    state.sync(&buffer);
    assert_eq!(state.visible().len(), 0);
}

/// The set the old `WARN` threshold stood for.
fn errors_and_warnings() -> super::LevelSet {
    only_these(&[Level::ERROR, Level::WARN])
}

/// A set with one level shown.
fn only(level: Level) -> super::LevelSet {
    only_these(&[level])
}

fn only_these(levels: &[Level]) -> super::LevelSet {
    let mut set = super::LevelSet::default();
    for level in super::ALL_LEVELS {
        set.set(level, levels.contains(&level));
    }
    set
}

fn from_project(level: Level, target: &str, message: &str) -> LogEntry {
    LogEntry {
        from_project: true,
        ..entry(level, target, message)
    }
}
/// The question this panel exists for — "did my trigger fire" — is
/// asked about the project, not the editor.
#[test]
fn project_only_hides_the_editors_own_lines() {
    let state = ConsoleState {
        project_only: true,
        ..Default::default()
    };
    assert!(state.shows(&from_project(Level::INFO, "ome_physics", "a sensor fired")));
    assert!(!state.shows(&entry(Level::INFO, "handlers", "scene loaded")));
}
/// The point of carrying the project's own level: asking for warnings
/// has to hide a project's info, which sniffing a prefix could never
/// do — every forwarded line used to arrive as an `info`.
#[test]
fn a_projects_line_is_filtered_by_its_own_level() {
    let state = ConsoleState {
        levels: errors_and_warnings(),
        ..Default::default()
    };
    assert!(state.shows(&from_project(
        Level::WARN,
        "ome_physics",
        "a joint is waiting"
    )));
    assert!(!state.shows(&from_project(
        Level::INFO,
        "ome_physics",
        "a sensor was entered"
    )));
}
