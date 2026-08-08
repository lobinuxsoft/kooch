use super::super::render::{colliding_names, module_path, render_registrations};
use super::super::{SourceFile, detect, ensure_features};

/// A discovered source file, for the render tests.
fn source(rel: &str, components: &[&str], systems: &[&str]) -> SourceFile {
    SourceFile {
        rel: rel.to_owned(),
        module: module_path(rel),
        components: components.iter().map(|s| (*s).to_owned()).collect(),
        systems: systems.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// A manifest in its own directory, the way the rest of the repo does
/// it — there is no `tempfile` in this workspace.
fn manifest_dir(name: &str, contents: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_codegen_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join("Cargo.toml"), contents).expect("write");
    dir
}

fn manifest_of(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("Cargo.toml")).expect("read")
}

/// A project scaffolded before `gravity` existed compiles a host with
/// no gravity system, so a `PointGravity` is authorable, mirrors, draws
/// its gizmo and pulls on nothing — which reads as "physics is broken".
#[test]
fn an_older_project_gains_the_gravity_feature() {
    let dir = manifest_dir(
        "older",
        "kooch = { path = \"../..\", features = [\"editor\", \"physics\"] }\n",
    );

    ensure_features(&dir);

    let out = manifest_of(&dir);
    assert!(out.contains("\"gravity\""), "{out}");
    assert!(
        out.contains("\"physics\""),
        "the existing features survived: {out}",
    );
}

/// Running twice must not append anything twice, because this runs on
/// every script regeneration.
///
/// Compares the second pass against the first rather than against the
/// original: the point is that repetition is a no-op, not which
/// features `ADDED` currently holds — pinning the list here made this
/// fail every time a feature was added, which is noise, not a signal.
#[test]
fn adding_a_feature_is_idempotent() {
    let dir = manifest_dir(
        "idempotent",
        "kooch = { path = \"../..\", features = [\"physics\"] }\n",
    );

    ensure_features(&dir);
    let after_first = manifest_of(&dir);

    ensure_features(&dir);
    assert_eq!(manifest_of(&dir), after_first, "a second pass changed it");

    // And it did do something the first time, or the test proves nothing.
    assert!(
        after_first.matches("gravity").count() == 1,
        "expected exactly one gravity: {after_first}"
    );
}

/// A manifest that does not depend on the engine the way the scaffold
/// writes it is somebody's own file, and is left alone.
#[test]
fn a_hand_written_manifest_is_untouched() {
    let original = "kooch = { git = \"…\" }\n";
    let dir = manifest_dir("hand_written", original);

    ensure_features(&dir);

    assert_eq!(manifest_of(&dir), original);
}

/// A project made before the library split gains everything a fresh
/// one is scaffolded with — without this, every existing project
/// would show none of its own components and look like it always had.
#[test]
fn an_old_project_gains_the_library() {
    let dir = std::env::temp_dir().join("kooch_migrate_lib_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"old_game\"\n\n[dependencies]\nkooch = { path = \"../..\" }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rs"),
        "mod registrations;\nfn main() {}\n",
    )
    .unwrap();

    super::super::migrate_to_library(&dir, "old_game");

    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[lib]"), "no lib target: {manifest}");
    assert!(manifest.contains("\"dylib\""), "not a dylib: {manifest}");
    assert!(
        manifest.contains("[[bin]]"),
        "declaring a lib stops cargo inferring the bin: {manifest}"
    );
    assert!(dir.join("src").join("lib.rs").exists(), "no lib.rs");

    let main = std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap();
    assert!(
        main.contains("use old_game::registrations;"),
        "main.rs still owns the module: {main}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Running it twice must change nothing — it runs on every script
/// registration, not once.
#[test]
fn the_migration_is_idempotent() {
    let dir = std::env::temp_dir().join("kooch_migrate_idempotent_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"g\"\n\n[dependencies]\nkooch = { path = \"../..\" }\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "mod registrations;\n").unwrap();

    super::super::migrate_to_library(&dir, "g");
    let once_manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let once_main = std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap();

    super::super::migrate_to_library(&dir, "g");
    assert_eq!(
        once_manifest,
        std::fs::read_to_string(dir.join("Cargo.toml")).unwrap()
    );
    assert_eq!(
        once_main,
        std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Somebody else's manifest is not ours to rewrite.
#[test]
fn a_hand_written_manifest_is_left_alone_by_the_migration() {
    let dir = std::env::temp_dir().join("kooch_migrate_handwritten_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let original = "[package]\nname = \"mine\"\n";
    std::fs::write(dir.join("Cargo.toml"), original).unwrap();

    super::super::migrate_to_library(&dir, "mine");

    assert_eq!(
        std::fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
        original
    );
    assert!(!dir.join("src").join("lib.rs").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The parity lock.** A freshly scaffolded project must already
/// satisfy the migration: if the scaffold grows something the
/// migration does not add, existing projects silently diverge and
/// nothing notices. This is the test that was missing when `--remote`
/// opened a second window for weeks.
#[test]
fn the_scaffold_already_satisfies_the_migration() {
    use crate::project::{generate_lib_rs, generate_main_rs};

    let dir = std::env::temp_dir().join("kooch_scaffold_parity_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();

    // What `create_project` writes, minus paths that need an engine root.
    std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"fresh\"\n\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n\n             [[bin]]\nname = \"fresh\"\npath = \"src/main.rs\"\n\n             [dependencies]\nkooch = { path = \"../..\" }\n",
        )
        .unwrap();
    std::fs::write(dir.join("src").join("main.rs"), generate_main_rs("fresh")).unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), generate_lib_rs("fresh")).unwrap();

    let before_manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let before_main = std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap();
    let before_lib = std::fs::read_to_string(dir.join("src").join("lib.rs")).unwrap();

    super::super::migrate_to_library(&dir, "fresh");

    assert_eq!(
        before_manifest,
        std::fs::read_to_string(dir.join("Cargo.toml")).unwrap(),
        "the scaffold produces a manifest the migration still wants to change"
    );
    assert_eq!(
        before_main,
        std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap(),
        "the scaffold produces a main.rs the migration still wants to change"
    );
    assert_eq!(
        before_lib,
        std::fs::read_to_string(dir.join("src").join("lib.rs")).unwrap()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn detects_component_and_system() {
    let src = "\
#[derive(Default, Reflect)]
pub struct Health {}
impl Component for Health {}

pub fn movement(resources: &mut Resources) {}
";
    let (components, systems) = detect(src);
    assert_eq!(components, vec!["Health".to_owned()]);
    assert_eq!(systems, vec!["movement".to_owned()]);
}

#[test]
fn a_folder_becomes_a_module_rather_than_a_prefix() {
    assert_eq!(module_path("player_health.rs"), "player_health");
    assert_eq!(module_path("enemies/ai.rs"), "enemies::ai");
    assert_eq!(module_path("a/b/c.rs"), "a::b::c");
}

/// The layout the generated file has to produce, verified by
/// compiling it in a real project before it was ever generated.
///
/// The `#[path]` on a container is the bare directory name: Rust
/// resolves it against the directory of `registrations.rs` and
/// *replaces* the path rather than appending to it, so neither `../`
/// nor a `registrations/` prefix belongs here. Children then resolve
/// against their container and only name their own file.
#[test]
fn folders_are_emitted_as_nested_modules() {
    let files = vec![
        source("components/movement.rs", &["GroundMovement"], &[]),
        source("player.rs", &["PlayerController"], &[]),
        source("systems/movement.rs", &[], &["apply_movement"]),
    ];
    let out = render_registrations(&files);

    assert!(
        out.contains("#[path = \"components\"]\npub mod components {\n"),
        "container must carry the bare directory name, got:\n{out}"
    );
    assert!(
        out.contains("    #[path = \"movement.rs\"]\n    pub mod movement;\n"),
        "a child names only its own file, and must be reachable from \
             outside its container, got:\n{out}"
    );
    assert!(
        !out.contains("components_movement"),
        "the flattened name survived, got:\n{out}"
    );
    assert!(
        out.contains("register_cpu_reflected::<components::movement::GroundMovement>()"),
        "registrations must use the nested path, got:\n{out}"
    );
    assert!(
        out.contains("run_if_playing(systems::movement::apply_movement)"),
        "systems must use the nested path, got:\n{out}"
    );
    // A file directly under `src/` keeps its plain declaration.
    assert!(
        out.contains("#[path = \"player.rs\"]\npub mod player;\n"),
        "a root-level file should not be wrapped, got:\n{out}"
    );
}

/// Every block that opens has to close, at any depth, or the
/// generated file is not even parseable.
#[test]
fn nested_folders_open_and_close_their_blocks() {
    let files = vec![
        source("a/b/deep.rs", &["Deep"], &[]),
        source("a/shallow.rs", &["Shallow"], &[]),
        source("z.rs", &["Z"], &[]),
    ];
    let out = render_registrations(&files);
    let modules = out.split("/// Editor-managed").next().expect("header");
    assert_eq!(
        modules.matches('{').count(),
        modules.matches('}').count(),
        "unbalanced module blocks:\n{modules}"
    );
    assert!(
        out.contains("register_cpu_reflected::<a::b::deep::Deep>()"),
        "got:\n{out}"
    );
    // `a/b/` closes before `a/shallow.rs`, which stays inside `a`.
    assert!(
        out.contains("register_cpu_reflected::<a::shallow::Shallow>()"),
        "got:\n{out}"
    );
}

/// A directory can be called anything the filesystem allows; a module
/// cannot. The `#[path]` keeps the real name either way.
#[test]
fn a_folder_named_like_a_keyword_still_compiles() {
    let out = render_registrations(&[source("move/dash.rs", &["Dash"], &[])]);
    assert!(
        out.contains("#[path = \"move\"]\npub mod r#move {"),
        "a keyword directory needs a raw identifier, got:\n{out}"
    );
    assert!(
        out.contains("register_cpu_reflected::<r#move::dash::Dash>()"),
        "got:\n{out}"
    );
    // `crate` cannot be a raw identifier at all.
    assert_eq!(module_path("crate/thing.rs"), "crate_::thing");
}

/// `src/components.rs` beside `src/components/` would be one `mod`
/// declared twice. Nothing can express it, so it gets reported.
#[test]
fn a_file_and_a_folder_of_the_same_name_are_reported() {
    let files = vec![
        source("components.rs", &["A"], &[]),
        source("components/movement.rs", &["B"], &[]),
    ];
    assert_eq!(colliding_names(&files), vec!["components".to_owned()]);
    // The ordinary layout is not a collision.
    let fine = vec![
        source("components/movement.rs", &["A"], &[]),
        source("systems/movement.rs", &[], &["s"]),
    ];
    assert!(colliding_names(&fine).is_empty());
}

/// The generated file used to open with
/// `#![allow(unused_imports, unused_variables, dead_code)]`. That is an
/// inner attribute on the `registrations` module, and every project
/// script is mounted inside it via `#[path]` — so it silenced those
/// three lints across the user's whole project, for good.
#[test]
fn the_generated_file_does_not_silence_lints_for_the_whole_project() {
    let out = render_registrations(&[source("components/movement.rs", &["GroundMovement"], &[])]);
    assert!(
        !out.contains("#!["),
        "an inner attribute here applies to every script in the project, got:\n{out}"
    );
}

/// …which is only safe if the generated code is itself warning-free.
/// With no components, `declare_components` never mentions its
/// parameter, and an unused parameter is exactly the warning the
/// blanket allow used to hide.
#[test]
fn a_project_with_no_components_still_uses_its_parameter() {
    let out = render_registrations(&[source("systems/movement.rs", &[], &["apply_movement"])]);
    assert!(
        out.contains("let _ = engine;"),
        "an unused parameter would warn in the user's project, got:\n{out}"
    );
    assert!(
        !out.contains("use kooch::kooch_ecs::component::plugin_bridge::declare_component;"),
        "the import would be unused too, got:\n{out}"
    );
}

/// The migration rewrites only the exact arm the editor generated,
/// and leaves a hand-written or already-headless main alone.
#[test]
fn remote_host_migration_is_narrow() {
    use super::super::migrate_remote_host;

    let generated = "\
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());";
    assert!(migrate_remote_host(generated).contains("RemoteHostPlugins"));

    // The game arm uses DefaultPlugins too and must survive.
    let game = "\
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: true });";
    assert_eq!(migrate_remote_host(game), game);

    // Already migrated: unchanged, and not migrated twice.
    let migrated = migrate_remote_host(generated);
    assert_eq!(migrate_remote_host(&migrated), migrated);
}

/// Gameplay systems must be registered unconditionally and wrapped
/// in the runtime gate — registering them only when `run_systems` is
/// set would make Play require a rebuild, which is the whole point
/// of the gate.
#[test]
fn generated_plugin_wraps_systems_in_the_runtime_gate() {
    let files = vec![source("movement.rs", &[], &["move_system"])];
    let out = render_registrations(&files);
    assert!(out.contains("pub run_systems: bool"));
    assert!(
        out.contains("insert_resource(Playing(self.run_systems))"),
        "run_systems must seed the gate, not branch on it"
    );
    assert!(
        out.contains("add_system(Stage::Update, run_if_playing(movement::move_system))"),
        "system must be registered wrapped, got:\n{out}"
    );
    assert!(
        !out.contains("if self.run_systems"),
        "compile-time branch survived"
    );
}

/// The regression that broke three real projects: `ensure_main_wired`
/// re-added `mod registrations;` beside the `use` the migration had
/// just written, declaring the name twice so the project stopped
/// compiling. The remote host then died on launch and the editor
/// reported only "remote project exited".
#[test]
fn a_duplicated_registrations_module_is_cleaned_up() {
    let dir = std::env::temp_dir().join("kooch_double_mod_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"g\"\n\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n\n             [dependencies]\nkooch = { path = \"../..\" }\n",
        )
        .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "pub mod registrations;\n").unwrap();
    // Both lines, as the two passes left it.
    std::fs::write(
        dir.join("src").join("main.rs"),
        "mod registrations;\n\nuse kooch::prelude::*;\n\nuse g::registrations;\n\nfn main() {}\n",
    )
    .unwrap();

    super::super::migrate_to_library(&dir, "g");

    let main = std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap();
    assert!(
        !main.contains("mod registrations;"),
        "the stray module declaration survived: {main}"
    );
    assert!(
        main.contains("use g::registrations;"),
        "the library import was lost: {main}"
    );
    assert!(main.contains("fn main()"), "body was mangled: {main}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// And the other end: once the `use` is there, rewiring must not put
/// the module back.
#[test]
fn wiring_does_not_re_add_a_module_that_moved_to_the_library() {
    let dir = std::env::temp_dir().join("kooch_no_readd_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let main = "use kooch::prelude::*;\n\nuse g::registrations;\n\n                    fn main() {\n    let mut app = App::new();\n                        app.add_plugins(DefaultPlugins);\n                        app.add_plugin(registrations::ProjectRegistrations { run_systems: true });\n}\n";
    std::fs::write(dir.join("src").join("main.rs"), main).unwrap();

    let mut resources = kooch_core::resource::Resources::new();
    super::super::ensure_main_wired(&dir, &mut resources);

    let after = std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap();
    assert!(
        !after.contains("mod registrations;"),
        "the module was re-added beside the library import: {after}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
