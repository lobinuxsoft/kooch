use super::*;

fn with_lines(n: usize) -> ConsoleState {
    let mut state = ConsoleState::default();
    for i in 0..n {
        state.entries.push(LogEntry {
            seq: i as u64,
            level: tracing::Level::INFO,
            target: "test".to_owned(),
            message: format!("line {i}"),
            from_project: false,
        });
        state.visible.push(i);
    }
    state
}

/// A log is read from the bottom, so the first Up offers the newest
/// line rather than the oldest one thousands of rows above.
#[test]
fn the_first_step_up_lands_on_the_newest_line() {
    let mut state = with_lines(50);
    state.move_cursor(-1);
    assert_eq!(state.cursor(), Some(49));
}

#[test]
fn the_cursor_stops_at_both_ends() {
    let mut state = with_lines(3);
    state.cursor_to_edge(false);
    state.move_cursor(-10);
    assert_eq!(state.cursor(), Some(0), "walked off the top");
    state.move_cursor(100);
    assert_eq!(state.cursor(), Some(2), "walked off the bottom");
}

/// Moving the cursor is a statement that the user is reading, and
/// following the tail would drag the view out from under them.
#[test]
fn moving_the_cursor_stops_following() {
    let mut state = with_lines(5);
    assert!(state.follow, "the default is to follow");
    state.move_cursor(-1);
    assert!(!state.follow);
}

/// The filter can shrink the list under a cursor that was valid when
/// it was set, so the clamp is on read.
#[test]
fn a_cursor_past_the_end_is_clamped_not_lost() {
    let mut state = with_lines(10);
    state.cursor_to_edge(true);
    assert_eq!(state.cursor(), Some(9));
    state.visible.truncate(3);
    assert_eq!(
        state.cursor(),
        Some(2),
        "should clamp into the shorter list"
    );
}

#[test]
fn an_empty_log_has_nowhere_for_a_cursor() {
    let mut state = ConsoleState::default();
    state.move_cursor(-1);
    assert_eq!(state.cursor(), None);
    assert!(state.cursor_line().is_none());
}
