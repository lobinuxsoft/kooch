use std::time::Duration;

use super::{ScriptSync, SyncState, fingerprint, sync_scripts_system};

/// A directory of its own, the way the rest of this crate does it —
/// there is no `tempfile` in this workspace.
fn src_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_script_sync_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

fn write(dir: &std::path::Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, body).expect("write");
}

/// A deleted file moves the fingerprint even though no mtime grew.
///
/// 🔴 This is why the count is there. Removing the newest file leaves an
/// older maximum behind, so mtime alone reports "nothing has happened
/// since" while a system just stopped existing — and the registration
/// for it would keep naming a module that is gone, which does not run,
/// it fails to compile.
#[test]
fn a_deleted_file_moves_the_fingerprint() {
    let dir = src_dir("deleted");
    write(&dir, "one.rs", "pub fn one() {}");
    write(&dir, "two.rs", "pub fn two() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    std::fs::remove_file(dir.join("two.rs")).expect("remove");
    let after = fingerprint(&dir).expect("fingerprint");

    assert_ne!(
        before, after,
        "the fingerprint did not move, so a removed system would go unnoticed \
         until something else in src/ happened to be saved"
    );
}

/// Writing the generated file does not itself look like a change.
///
/// Without the exclusion every regeneration would move the fingerprint,
/// scheduling a scan that reads every file in the project to conclude
/// nothing needs doing — on a FUSE mount, after every save.
#[test]
fn the_generated_file_is_not_a_change() {
    let dir = src_dir("generated");
    write(&dir, "one.rs", "pub fn one() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    write(&dir, "registrations.rs", "// AUTO-GENERATED\n");
    let after = fingerprint(&dir).expect("fingerprint");

    assert_eq!(
        before, after,
        "`registrations.rs` counted towards the fingerprint, so writing it \
         schedules a scan that can only find nothing"
    );
}

/// Nested directories are seen. A project puts systems in `src/systems/`.
#[test]
fn a_nested_file_is_counted() {
    let dir = src_dir("nested");
    write(&dir, "one.rs", "pub fn one() {}");
    let flat = fingerprint(&dir).expect("fingerprint");

    write(&dir, "systems/spin.rs", "pub fn spin() {}");
    let nested = fingerprint(&dir).expect("fingerprint");

    assert_eq!(flat.1 + 1, nested.1, "the walk stopped at the top level");
}

/// Non-Rust files are ignored, so saving an asset beside the code does
/// not announce a rebuild nobody needs.
#[test]
fn only_rust_counts() {
    let dir = src_dir("only_rust");
    write(&dir, "one.rs", "pub fn one() {}");
    let before = fingerprint(&dir).expect("fingerprint");

    write(&dir, "notes.md", "# not code");
    assert_eq!(before, fingerprint(&dir).expect("fingerprint"));
}

/// Acknowledging clears the warning, and nothing else does.
#[test]
fn acknowledging_clears_the_warning() {
    let mut sync = ScriptSync {
        state: SyncState::NeedsRebuild,
        fingerprint: Some((Duration::ZERO, 1)),
        ..Default::default()
    };
    sync.acknowledge();
    assert_eq!(sync.state, SyncState::Current);
}

// ---- what a move in src/ means for the build ------------------------

use kooch_core::resource::Resources;

use crate::actions::register_scripts;
use crate::project_state::{ActiveProject, ProjectState};

/// A project with a `src/` the codegen can scan, and the resources the
/// poll reads.
fn project(name: &str) -> (std::path::PathBuf, Resources) {
    let root = std::env::temp_dir().join(format!("kooch_sync_project_{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create src");

    let mut state = ProjectState::new();
    state.active_project = Some(ActiveProject {
        manifest: crate::project::ProjectManifest::new(name),
        root_path: root.clone(),
    });
    let mut resources = Resources::new();
    resources.insert(state);
    resources.insert(ScriptSync::default());
    (root, resources)
}

/// 🔴 The one that motivated the change: a field is not in the generated
/// file, and the editor reads fields out of the compiled dylib.
///
/// Reported as *"modifiqué el script y le di a Code Sync y no hizo
/// nada"*. It did exactly what it was told; the notice was keyed on the
/// generated file changing, so the edits that change no type — a field,
/// a body, a default — left it dark while the build went stale.
#[test]
fn an_edit_that_regenerates_nothing_still_asks() {
    let (root, mut resources) = project("field_edit");
    let src = root.join("src");
    write(&src, "main.rs", "fn main() {}\n");
    write(
        &src,
        "thing.rs",
        "#[derive(Default, Reflect)]\npub struct Health {}\nimpl Component for Health {}\n",
    );
    // The state the author is in: registrations current, build current.
    register_scripts(&mut resources);
    let generated = std::fs::read_to_string(src.join("registrations.rs")).expect("generated");

    // A field. The type is the same, so the render will be too.
    write(
        &src,
        "thing.rs",
        "#[derive(Default, Reflect)]\npub struct Health {\n    pub hp: f32,\n}\n\
         impl Component for Health {}\n",
    );
    // A fingerprint the walk cannot match, which is what `src/` moving
    // looks like to the poll — without waiting on a filesystem clock.
    let sync = resources.get_mut::<ScriptSync>().expect("sync");
    sync.fingerprint = Some((Duration::ZERO, 99));
    sync.next_poll = None;

    sync_scripts_system(&mut resources);

    assert_eq!(
        std::fs::read_to_string(src.join("registrations.rs")).expect("generated"),
        generated,
        "the edit rewrote the generated file, so this no longer tests the silent case",
    );
    let sync = resources.get::<ScriptSync>().expect("sync");
    assert_eq!(
        sync.state,
        SyncState::NeedsRebuild,
        "a field was added and the editor still claimed the build was current",
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A first sighting records and acts on nothing: opening a project would
/// otherwise announce a rebuild for a build that was already correct.
#[test]
fn opening_a_project_asks_for_nothing() {
    let (root, mut resources) = project("first_sighting");
    write(&root.join("src"), "thing.rs", "pub fn tick() {}\n");

    sync_scripts_system(&mut resources);

    let sync = resources.get::<ScriptSync>().expect("sync");
    assert_eq!(sync.state, SyncState::Current);
    let _ = std::fs::remove_dir_all(&root);
}
