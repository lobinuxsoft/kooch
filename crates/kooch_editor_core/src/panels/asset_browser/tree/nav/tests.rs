use super::*;

fn folder(path: &str, open: bool) -> AssetRow {
    AssetRow {
        path: PathBuf::from(path),
        is_folder: true,
        open,
    }
}

fn file(path: &str) -> AssetRow {
    AssetRow {
        path: PathBuf::from(path),
        is_folder: false,
        open: false,
    }
}

/// `assets/` open, with two files, then a closed `src/`.
fn nav() -> AssetNav {
    AssetNav {
        rows: vec![
            folder("/p/assets", true),
            file("/p/assets/a.png"),
            file("/p/assets/b.png"),
            folder("/p/src", false),
        ],
        ..Default::default()
    }
}

/// A first press has to land somewhere, or it reads as a key that was
/// not received.
#[test]
fn the_first_step_lands_on_the_first_row() {
    let mut n = nav();
    n.step(1);
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
}

#[test]
fn stepping_stops_at_both_ends() {
    let mut n = nav();
    n.to_edge(false);
    n.step(-5);
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
    n.step(50);
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/src")));
}

#[test]
fn right_opens_a_closed_folder() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/src"));
    n.expand_or_enter();
    assert_eq!(n.toggle, Some((PathBuf::from("/p/src"), true)));
}

/// Held Right should descend rather than stop, which is what makes it
/// feel like a tree.
#[test]
fn right_on_an_open_folder_steps_into_it() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/assets"));
    n.expand_or_enter();
    assert_eq!(n.toggle, None, "an open folder has nothing to open");
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets/a.png")));
}

#[test]
fn left_closes_an_open_folder() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/assets"));
    n.collapse_or_parent();
    assert_eq!(n.toggle, Some((PathBuf::from("/p/assets"), false)));
}

/// From a file, Left goes to the folder that contains it — by path,
/// because the row above can belong to another branch.
#[test]
fn left_on_a_file_goes_to_its_own_parent() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/assets/b.png"));
    n.collapse_or_parent();
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
}

/// A parent that is not drawn — filtered out, or above a collapsed
/// root — is not somewhere to jump to.
#[test]
fn left_does_not_jump_to_a_parent_that_is_not_on_screen() {
    let mut n = AssetNav {
        rows: vec![file("/p/assets/deep/x.png")],
        cursor: Some(PathBuf::from("/p/assets/deep/x.png")),
        ..Default::default()
    };
    n.collapse_or_parent();
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets/deep/x.png")));
}

/// A folder closing over the cursor, or a deleted file, leaves a path
/// that no longer has a row.
#[test]
fn a_cursor_whose_row_vanished_recovers_on_the_next_key() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/gone/away.png"));
    assert!(n.current().is_none());
    n.step(1);
    assert_eq!(n.cursor.as_deref(), Some(Path::new("/p/assets")));
}

#[test]
fn a_toggle_is_only_taken_by_the_folder_it_names() {
    let mut n = nav();
    n.toggle = Some((PathBuf::from("/p/src"), true));
    assert_eq!(n.take_toggle_for(Path::new("/p/assets")), None);
    assert_eq!(n.take_toggle_for(Path::new("/p/src")), Some(true));
    assert_eq!(n.take_toggle_for(Path::new("/p/src")), None, "taken once");
}

/// The selection follows the cursor however it was moved, and reports
/// once — a repeat would re-select every frame and fight a click.
#[test]
fn a_cursor_move_is_reported_exactly_once() {
    let mut n = nav();
    assert_eq!(n.take_cursor_move(), None, "nothing has moved yet");
    n.step(1);
    assert_eq!(n.take_cursor_move(), Some(folder("/p/assets", true)));
    assert_eq!(n.take_cursor_move(), None, "reported once");
    n.cursor = Some(PathBuf::from("/p/assets/b.png"));
    assert_eq!(n.take_cursor_move(), Some(file("/p/assets/b.png")));
}

/// Losing focus clears the cursor. That must not read as "select
/// nothing", or clicking into the Inspector would empty it.
#[test]
fn a_cleared_cursor_does_not_report_a_selection() {
    let mut n = nav();
    n.cursor = Some(PathBuf::from("/p/src"));
    n.take_cursor_move();
    n.cursor = None;
    assert_eq!(n.take_cursor_move(), None);
    // And the same row is selectable again afterwards, rather than
    // being swallowed as "unchanged".
    n.cursor = Some(PathBuf::from("/p/src"));
    assert_eq!(n.take_cursor_move(), Some(folder("/p/src", false)));
}

#[test]
fn an_empty_tree_has_nowhere_to_put_a_cursor() {
    let mut n = AssetNav::default();
    n.step(1);
    n.to_edge(true);
    n.expand_or_enter();
    n.collapse_or_parent();
    assert_eq!(n.cursor, None);
}
