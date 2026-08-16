use super::*;

/// The whole point: an editor whose version is invisible is one nobody
/// can compare against the engine a project is pinned to.
#[test]
fn the_title_carries_the_version() {
    let version = crate::engine_vendor::editor_engine_version();
    assert!(window_title(None).contains(version));
    assert!(window_title(Some("roll-a-ball")).contains(version));
}

/// The project first, because a task switcher truncates from the right
/// and "Kóoch 0.2.31 — roll-a-b…" answers the question nobody asked.
#[test]
fn the_project_comes_first() {
    let title = window_title(Some("roll-a-ball"));
    assert!(title.starts_with("roll-a-ball"), "got {title}");
}
