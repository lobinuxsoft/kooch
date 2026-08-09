//! A file created outside the trees the editor scans is a file nothing
//! reads — no GUID, no compile, and no complaint.

use std::path::Path;

use super::model::FolderRole;

fn role(path: &str) -> FolderRole {
    FolderRole::of(Path::new(path), Some(Path::new("/proj")))
}

#[test]
fn the_two_scanned_trees_are_known() {
    assert_eq!(role("/proj/assets"), FolderRole::Assets);
    assert_eq!(role("/proj/assets/props/rock"), FolderRole::Assets);
    assert_eq!(role("/proj/src"), FolderRole::Source);
    assert_eq!(role("/proj/src/enemies"), FolderRole::Source);
}

/// The project root itself is the one people reach for first, and it is
/// scanned by neither.
#[test]
fn the_project_root_is_neither() {
    assert_eq!(role("/proj"), FolderRole::Other);
    assert_eq!(role("/proj/scenes"), FolderRole::Other);
}

/// 🔴 A prefix match on the string would call `/proj/assets_backup` an
/// assets folder, and a material written there is registered by nothing.
#[test]
fn a_similar_name_is_not_a_match() {
    assert_eq!(role("/proj/assets_backup"), FolderRole::Other);
    assert_eq!(role("/proj/src_old"), FolderRole::Other);
}

/// The engine's read-only tree, and anything else outside the project.
#[test]
fn a_folder_outside_the_project_is_other() {
    assert_eq!(role("/engine/assets/materials"), FolderRole::Other);
}

#[test]
fn with_no_project_nothing_is_scanned() {
    assert_eq!(
        FolderRole::of(Path::new("/proj/assets"), None),
        FolderRole::Other,
    );
}

/// The refusal names the folder that would work. "Disabled" on its own
/// is a dead end — the whole point is telling someone where to go.
#[test]
fn a_refusal_names_the_right_folder() {
    assert_eq!(FolderRole::Assets.refusal(FolderRole::Assets), None);
    assert!(
        FolderRole::Other
            .refusal(FolderRole::Assets)
            .is_some_and(|why| why.contains("assets/")),
    );
    assert!(
        FolderRole::Assets
            .refusal(FolderRole::Source)
            .is_some_and(|why| why.contains("src/")),
    );
}
