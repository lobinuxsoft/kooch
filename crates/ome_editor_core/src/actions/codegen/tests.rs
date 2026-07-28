//! Tests for [`super`] — scaffold generation and project migrations.

#[cfg(test)]
mod tests {
    use super::super::{detect, ensure_features, module_name};

    /// A manifest in its own directory, the way the rest of the repo does
    /// it — there is no `tempfile` in this workspace.
    fn manifest_dir(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ome_codegen_{name}"));
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
            "oh_my_engine = { path = \"../..\", features = [\"editor\", \"physics\"] }\n",
        );

        ensure_features(&dir);

        let out = manifest_of(&dir);
        assert!(out.contains("\"gravity\""), "{out}");
        assert!(
            out.contains("\"physics\""),
            "the existing features survived: {out}",
        );
    }

    /// Running twice must not append it twice, because this runs on every
    /// script regeneration.
    #[test]
    fn adding_a_feature_is_idempotent() {
        let dir = manifest_dir(
            "idempotent",
            "oh_my_engine = { path = \"../..\", features = [\"physics\", \"gravity\"] }\n",
        );
        let before = manifest_of(&dir);

        ensure_features(&dir);
        ensure_features(&dir);

        assert_eq!(manifest_of(&dir), before);
    }

    /// A manifest that does not depend on the engine the way the scaffold
    /// writes it is somebody's own file, and is left alone.
    #[test]
    fn a_hand_written_manifest_is_untouched() {
        let original = "oh_my_engine = { git = \"…\" }\n";
        let dir = manifest_dir("hand_written", original);

        ensure_features(&dir);

        assert_eq!(manifest_of(&dir), original);
    }

    /// A project made before the library split gains everything a fresh
    /// one is scaffolded with — without this, every existing project
    /// would show none of its own components and look like it always had.
    #[test]
    fn an_old_project_gains_the_library() {
        let dir = std::env::temp_dir().join("ome_migrate_lib_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"old_game\"\n\n[dependencies]\noh_my_engine = { path = \"../..\" }\n",
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
        let dir = std::env::temp_dir().join("ome_migrate_idempotent_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"g\"\n\n[dependencies]\noh_my_engine = { path = \"../..\" }\n",
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
        let dir = std::env::temp_dir().join("ome_migrate_handwritten_test");
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

        let dir = std::env::temp_dir().join("ome_scaffold_parity_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();

        // What `create_project` writes, minus paths that need an engine root.
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"fresh\"\n\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n\n             [[bin]]\nname = \"fresh\"\npath = \"src/main.rs\"\n\n             [dependencies]\noh_my_engine = { path = \"../..\" }\n",
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
    fn module_name_flattens_nested_paths() {
        assert_eq!(module_name("player_health.rs"), "player_health");
        assert_eq!(module_name("enemies/ai.rs"), "enemies_ai");
    }

    /// The migration rewrites only the exact arm the editor generated,
    /// and leaves a hand-written or already-headless main alone.
    #[test]
    fn remote_host_migration_is_narrow() {
        use super::super::migrate_remote_host;

        let generated = "\
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(oh_my_engine::ome_remote::RemotePlugin::new());";
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
        use super::super::{SourceFile, render_registrations};
        let files = vec![SourceFile {
            rel: "movement.rs".to_owned(),
            module: "movement".to_owned(),
            components: vec![],
            systems: vec!["move_system".to_owned()],
        }];
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
}
