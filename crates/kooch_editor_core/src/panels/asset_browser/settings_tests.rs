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
