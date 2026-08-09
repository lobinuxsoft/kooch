//! #759 — `.rendersettings` is creatable from the Asset Browser.
//!
//! The capability existed (#744 made it a reflected asset the Inspector
//! edits) and nothing led to it: the only way to get one was to write the
//! file by hand, and someone who did not know it existed could not find
//! out from the editor.

use std::path::Path;

use kooch_core::resource::Resources;
use kooch_render::settings::{RENDER_SETTINGS_EXTENSION, RenderSettings};

use super::{NewFileKind, create_file};

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_settings_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The file the menu writes has to be one the loader accepts. Written by
/// hand it would be a guess; this round-trips it through the real
/// serialiser and the real parser.
#[test]
fn a_new_settings_file_parses() {
    let dir = tmp("parses");
    let mut resources = Resources::new();

    create_file(&mut resources, &dir, "project", NewFileKind::RenderSettings);

    let file = dir.join(format!("project.{RENDER_SETTINGS_EXTENSION}"));
    assert!(file.is_file(), "no settings file was written");
    let parsed: RenderSettings =
        ron::from_str(&std::fs::read_to_string(&file).unwrap()).expect("the loader parses it");
    assert_eq!(
        parsed,
        RenderSettings::default(),
        "a new file does not describe the engine's own defaults, so creating \
         one would silently change how the project looks",
    );
}

/// Twice in the same folder must not overwrite the first. The menu
/// disables the entry once a project has one, but the action is reachable
/// from elsewhere and `unique_target` is what actually guarantees this.
#[test]
fn a_second_file_does_not_overwrite() {
    let dir = tmp("unique");
    let mut resources = Resources::new();

    create_file(&mut resources, &dir, "project", NewFileKind::RenderSettings);
    create_file(&mut resources, &dir, "project", NewFileKind::RenderSettings);

    assert!(
        dir.join(format!("project.{RENDER_SETTINGS_EXTENSION}"))
            .is_file()
    );
    assert!(
        dir.join(format!("project_1.{RENDER_SETTINGS_EXTENSION}"))
            .is_file(),
        "the second write landed on the first",
    );
}

/// The extension has to be the one the loader claims, or the scan never
/// adopts the file and the renderer never finds it — authored, saved and
/// inert, which is exactly the failure #744 fixed one layer down.
#[test]
fn the_extension_is_the_loaders() {
    let dir = tmp("extension");
    let mut resources = Resources::new();

    create_file(&mut resources, &dir, "look", NewFileKind::RenderSettings);

    let written = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e != "meta"))
        .expect("something was written");
    assert_eq!(
        written.extension().and_then(|e| e.to_str()),
        Some(RENDER_SETTINGS_EXTENSION),
    );
    assert_eq!(written.file_stem().and_then(|s| s.to_str()), Some("look"));
}

/// The name is taken as typed. A settings file is one per project and
/// its name is a label, so `to_snake_case` — which the Rust kinds need —
/// must not reach it.
#[test]
fn the_typed_name_is_kept() {
    let dir = tmp("verbatim");
    let mut resources = Resources::new();

    create_file(&mut resources, &dir, "My Look", NewFileKind::RenderSettings);

    assert!(
        dir.join(format!("My Look.{RENDER_SETTINGS_EXTENSION}"))
            .is_file(),
        "the name was rewritten: {:?}",
        listing(&dir),
    );
}

fn listing(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// #758 — the same wiring, so a `.buildpreset` is creatable too. Several
/// per project, unlike settings.
#[test]
fn a_new_build_preset_parses() {
    let dir = tmp("preset");
    let mut resources = Resources::new();

    create_file(
        &mut resources,
        &dir,
        "Windows release",
        NewFileKind::BuildPreset,
    );
    create_file(
        &mut resources,
        &dir,
        "Windows release",
        NewFileKind::BuildPreset,
    );

    let ext = crate::build::BUILD_PRESET_EXTENSION;
    let file = dir.join(format!("Windows release.{ext}"));
    assert!(file.is_file(), "no preset was written: {:?}", listing(&dir));
    assert!(
        dir.join(format!("Windows release_1.{ext}")).is_file(),
        "a second preset overwrote the first — presets are a list",
    );

    let parsed: crate::build::BuildPreset =
        ron::from_str(&std::fs::read_to_string(&file).unwrap()).expect("the loader parses it");
    assert_eq!(parsed, crate::build::BuildPreset::default());
}
