//! #759 — what decides whether the menu offers a `.rendersettings`.

use super::*;
use crate::panels::inspector::AssetSource;

fn entry(path: &str, type_name: &str) -> AssetCatalogEntry {
    AssetCatalogEntry {
        guid: Guid::new_v4(),
        path: PathBuf::from(path),
        display_name: path.to_owned(),
        source: AssetSource::Project,
        type_name: type_name.to_owned(),
    }
}

fn settings_type() -> &'static str {
    std::any::type_name::<kooch_render::settings::RenderSettings>()
}

#[test]
fn an_empty_project_may_author_one() {
    assert!(!has_render_settings(&[], Some(Path::new("/proj"))));
}

#[test]
fn a_project_settings_is_found() {
    let catalog = [entry("/proj/look.rendersettings", settings_type())];
    assert!(has_render_settings(&catalog, Some(Path::new("/proj"))));
}

/// 🔴 The engine ships assets of its own, and one of those must not stop
/// a project from authoring its settings — that would leave the entry
/// permanently disabled in every project, which reads as a broken menu.
#[test]
fn an_engine_settings_does_not_count() {
    let catalog = [entry("/engine/default.rendersettings", settings_type())];
    assert!(!has_render_settings(&catalog, Some(Path::new("/proj"))));
}

/// By type, not by extension: the renderer finds this asset by type, so
/// a file the catalog never typed is one it will not read either. Asking
/// the same question keeps the menu and the effect in agreement.
#[test]
fn an_untyped_file_does_not_count() {
    let catalog = [entry("/proj/look.rendersettings", "some::OtherThing")];
    assert!(!has_render_settings(&catalog, Some(Path::new("/proj"))));
}

/// No project open is not "a project with none": there is nowhere to
/// write, and the engine root is read-only.
#[test]
fn no_project_means_nothing_found() {
    let catalog = [entry("/engine/default.rendersettings", settings_type())];
    assert!(!has_render_settings(&catalog, None));
}

/// 🔴 A dropped file used to land in whatever folder was selected — or in
/// the project root when none was, beside `Cargo.toml`, where nothing
/// registers it and no build carries it.
///
/// The same rule the "New …" menu enforces (#765), applied to the other
/// way a file enters a project. A rule that holds for one entrance and
/// not the other is not a rule.
#[test]
fn imports_always_land_under_assets() {
    let proj = Path::new("/proj");
    let assets = proj.join("assets");

    // Nothing selected: not the project root any more.
    assert_eq!(import_destination(None, Some(proj)), Some(assets.clone()));
    // Selected somewhere that is not assets: still assets.
    assert_eq!(
        import_destination(Some(&proj.join("src")), Some(proj)),
        Some(assets.clone()),
    );
    assert_eq!(
        import_destination(Some(proj), Some(proj)),
        Some(assets.clone())
    );
    // Selected inside assets: that folder, so dropping into a subfolder
    // still works.
    let props = assets.join("props");
    assert_eq!(import_destination(Some(&props), Some(proj)), Some(props));
}

/// No project, nowhere to import: the engine's tree is read-only.
#[test]
fn no_project_means_no_import() {
    assert_eq!(import_destination(Some(Path::new("/x")), None), None);
}
