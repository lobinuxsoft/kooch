use super::*;

#[test]
fn a_name_becomes_a_filename() {
    assert_eq!(sanitize_file_stem("Ball"), "Ball");
    assert_eq!(sanitize_file_stem("Player (spare)"), "Player_spare");
}

/// A slash would write outside the folder that was chosen, which is
/// the whole reason this function exists.
#[test]
fn a_path_cannot_be_smuggled_through_a_name() {
    assert_eq!(sanitize_file_stem("enemy/variant"), "enemy_variant");
    assert_eq!(sanitize_file_stem("../../etc/passwd"), "etc_passwd");
}

#[test]
fn a_name_with_nothing_usable_in_it_still_gets_a_file() {
    assert_eq!(sanitize_file_stem(""), "Prefab");
    assert_eq!(sanitize_file_stem("///"), "Prefab");
    assert_eq!(sanitize_file_stem("..."), "Prefab");
}

#[test]
fn without_a_drop_folder_a_prefab_goes_to_assets() {
    let root = Path::new("/p");
    assert_eq!(
        prefab_path(root, "Ball", None),
        PathBuf::from("/p/assets/Ball.prefab"),
    );
}

/// A folder outside the project is not somewhere the catalog scans, so
/// the file would be written and never appear.
#[test]
fn a_destination_outside_the_project_falls_back_to_assets() {
    let root = Path::new("/p");
    assert_eq!(
        prefab_path(root, "Ball", Some(Path::new("/somewhere/else"))),
        PathBuf::from("/p/assets/Ball.prefab"),
    );
}

#[test]
fn a_drop_folder_inside_the_project_is_used() {
    let root = Path::new("/p");
    assert_eq!(
        prefab_path(root, "Ball", Some(Path::new("/p/assets/enemies"))),
        PathBuf::from("/p/assets/enemies/Ball.prefab"),
    );
}

/// The same entity resolves to the same file every time, which is what
/// makes re-saving update a prefab rather than litter the folder. The
/// prompt is what keeps that from being destructive.
///
/// `std::env::temp_dir` rather than a crate — there is no `tempfile` in
/// this workspace, and the rest of the editor's file tests do the same.
#[test]
fn re_saving_resolves_to_the_same_file() {
    let root = std::env::temp_dir().join("kooch_prefab_name_clash");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("assets")).unwrap();

    let first = prefab_path(&root, "Enemy", None);
    std::fs::write(&first, "").unwrap();
    assert_eq!(
        prefab_path(&root, "Enemy", None),
        first,
        "an existing file must resolve to itself, not to a new name",
    );
    let _ = std::fs::remove_dir_all(&root);
}
