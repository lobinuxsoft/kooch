//! Asset Browser panel — navigates every registered asset in the
//! current project (and the shipped engine assets), grouped by type.
//!
//! Read-only by design: it answers "what assets does this project
//! contain?" so an artist can see the GUIDs, paths, and types the
//! `AssetDatabase` knows about. Assignment happens through the
//! Inspector's typed picker (`ReflectValue::AssetRef`), which consumes
//! the exact same [`AssetCatalogEntry`] slice this panel renders.
//!
//! Only *typed* entries appear — an asset with no `.meta` `asset_type`
//! (never touched by a typed `load::<T>`) is skipped upstream in
//! [`AssetCatalogEntry::collect_from_database`], because there is no
//! type to file it under.

use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

/// Content of the "Asset Browser" tab.
///
/// `catalog` is the per-frame snapshot of the `AssetDatabase` already
/// collected for the Inspector picker — this panel adds no new data
/// plumbing, it just presents the same slice grouped by type.
pub(crate) fn draw_asset_browser_content(ui: &mut egui::Ui, catalog: &[AssetCatalogEntry]) {
    let search_id = ui.id().with("asset_browser_search");
    let mut query: String = ui
        .ctx()
        .data(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();

    // Search box mirrors the inspector picker's idiom: substring match
    // over display name + full path, persisted in egui memory so the
    // query survives across frames.
    let search_resp = ui.add(
        egui::TextEdit::singleline(&mut query)
            .desired_width(f32::INFINITY)
            .hint_text(format!("{} Search assets…", icons::MAGNIFYING_GLASS)),
    );
    if search_resp.changed() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(search_id, query.clone()));
    }

    let needle = query.trim().to_lowercase();
    let matches = |entry: &AssetCatalogEntry| -> bool {
        if needle.is_empty() {
            return true;
        }
        entry.display_name.to_lowercase().contains(&needle)
            || entry
                .path
                .display()
                .to_string()
                .to_lowercase()
                .contains(&needle)
    };

    let shown = catalog.iter().filter(|e| matches(e)).count();
    ui.label(format!("{} assets ({} shown)", catalog.len(), shown,));
    ui.separator();

    if catalog.is_empty() {
        ui.weak("(no typed assets registered)");
        ui.weak("Open a project or add assets with a .meta sidecar.");
        return;
    }

    // Distinct type names, ordered by their friendly label so groups
    // land in the same slot every frame. Catalog size is a handful of
    // assets, so per-frame grouping is trivial.
    let mut types: Vec<&str> = catalog.iter().map(|e| e.type_name.as_str()).collect();
    types.sort_unstable();
    types.dedup();
    types.sort_by(|a, b| friendly_type_label(a).cmp(friendly_type_label(b)));

    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut any_visible = false;
        for type_name in types {
            // Entries of this type matching the search. The catalog is
            // pre-sorted (source, then name), so intra-group order is
            // already stable.
            let entries: Vec<&AssetCatalogEntry> = catalog
                .iter()
                .filter(|e| e.type_name == type_name && matches(e))
                .collect();
            if entries.is_empty() {
                continue;
            }
            any_visible = true;

            let header = format!(
                "{} {}  ({})",
                type_icon(type_name),
                friendly_type_label(type_name),
                entries.len(),
            );
            let id = ui.make_persistent_id(("asset_group", type_name));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    ui.label(header);
                })
                .body(|ui| {
                    for entry in entries {
                        draw_asset_row(ui, entry);
                    }
                });
        }

        if !any_visible {
            ui.weak("(no match)");
        }
    });
}

/// Renders one asset row: name + source tag, with the full path, GUID
/// and canonical type revealed on hover so devs can copy a GUID into a
/// hand-written `.ron` reference.
fn draw_asset_row(ui: &mut egui::Ui, entry: &AssetCatalogEntry) {
    ui.horizontal(|ui| {
        ui.label(&entry.display_name);
        ui.weak(format!("[{}]", entry.source.label()));
    })
    .response
    .on_hover_text(format!(
        "{}\nguid: {}\ntype: {}",
        entry.path.display(),
        entry.guid,
        entry.type_name,
    ));
}

/// Maps a canonical asset type name to a short, plural, artist-facing
/// label. Falls back to the type's last path segment for any type not
/// explicitly known, so new asset kinds show up sensibly without a
/// code change here.
fn friendly_type_label(type_name: &str) -> &str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => "Meshes",
        "ome_render::material::asset::Material" => "Materials",
        "ome_render::texture::asset::Image" => "Textures",
        other => other.rsplit("::").next().unwrap_or(other),
    }
}

/// Icon per asset type. Generic `STACK` for anything not explicitly
/// mapped.
fn type_icon(type_name: &str) -> &'static str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => icons::CUBE,
        "ome_render::material::asset::Material" => icons::FADERS,
        _ => icons::STACK,
    }
}

#[cfg(test)]
mod tests {
    use super::friendly_type_label;

    #[test]
    fn known_types_get_friendly_plural_labels() {
        assert_eq!(
            friendly_type_label("ome_render::meshlet::asset::MeshletMesh"),
            "Meshes",
        );
        assert_eq!(
            friendly_type_label("ome_render::material::asset::Material"),
            "Materials",
        );
        assert_eq!(
            friendly_type_label("ome_render::texture::asset::Image"),
            "Textures",
        );
    }

    #[test]
    fn unknown_type_falls_back_to_last_segment() {
        assert_eq!(friendly_type_label("foo::bar::Baz"), "Baz");
        assert_eq!(friendly_type_label("Bare"), "Bare");
    }
}
