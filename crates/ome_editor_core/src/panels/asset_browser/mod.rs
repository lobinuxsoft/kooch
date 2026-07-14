//! Asset Browser panel — navigates every registered asset in the
//! current project (and the shipped engine assets), grouped by type.
//!
//! The list surfaces the same [`AssetCatalogEntry`] slice the Inspector
//! picker consumes. Selecting a row records the selection on the editor
//! overlay; the **Inspector** panel then renders that asset's import
//! settings / editable parameters — the Inspector serves both entities
//! and assets. See [`crate::panels::inspector::asset_view`].
//!
//! Only *typed* entries appear — an asset with no `.meta` `asset_type`
//! (never touched by a typed `load::<T>`) is skipped upstream in
//! [`AssetCatalogEntry::collect_from_database`], because there is no
//! type to file it under.

use ome_core::Guid;

use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;

/// Content of the "Asset Browser" tab.
///
/// `catalog` is the per-frame `AssetDatabase` snapshot; `selected_asset`
/// is the panel's selection (owned by the editor overlay so the render
/// system can pre-resolve the asset's data for the Inspector).
pub(crate) fn draw_asset_browser_content(
    ui: &mut egui::Ui,
    catalog: &[AssetCatalogEntry],
    selected_asset: &mut Option<Guid>,
) {
    let search_id = ui.id().with("asset_browser_search");
    let mut query: String = ui
        .ctx()
        .data(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();

    let search_resp = ui.add(
        egui::TextEdit::singleline(&mut query)
            .id(search_id)
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
    ui.label(format!("{} assets ({} shown)", catalog.len(), shown));
    ui.separator();

    if catalog.is_empty() {
        ui.weak("(no typed assets registered)");
        ui.weak("Open a project or add assets with a .meta sidecar.");
        return;
    }

    // Distinct type names, ordered by friendly label so groups land in
    // the same slot every frame. Catalog size is a handful of assets, so
    // per-frame grouping is trivial.
    let mut types: Vec<&str> = catalog.iter().map(|e| e.type_name.as_str()).collect();
    types.sort_unstable();
    types.dedup();
    types.sort_by(|a, b| friendly_type_label(a).cmp(friendly_type_label(b)));

    // Clicking a row selects it; clicking the selected row clears it.
    let mut toggled: Option<Guid> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut any_visible = false;
        for type_name in types {
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
                        if draw_asset_row(ui, entry, *selected_asset == Some(entry.guid)) {
                            toggled = Some(entry.guid);
                        }
                    }
                });
        }

        if !any_visible {
            ui.weak("(no match)");
        }
    });

    if let Some(guid) = toggled {
        *selected_asset = if *selected_asset == Some(guid) {
            None
        } else {
            Some(guid)
        };
    }
}

/// Renders one selectable asset row. Returns `true` when clicked.
fn draw_asset_row(ui: &mut egui::Ui, entry: &AssetCatalogEntry, selected: bool) -> bool {
    let label = format!("{}  [{}]", entry.display_name, entry.source.label());
    ui.selectable_label(selected, label)
        .on_hover_text(format!(
            "{}\nguid: {}\ntype: {}",
            entry.path.display(),
            entry.guid,
            entry.type_name,
        ))
        .clicked()
}

/// Maps a canonical asset type name to a short, plural, artist-facing
/// label. Falls back to the type's last path segment for any unknown
/// type, so new asset kinds show up sensibly without a code change here.
fn friendly_type_label(type_name: &str) -> &str {
    match type_name {
        "ome_render::meshlet::asset::MeshletMesh" => "Meshes",
        "ome_render::material::asset::Material" => "Materials",
        "ome_render::texture::asset::Image" => "Textures",
        other => other.rsplit("::").next().unwrap_or(other),
    }
}

/// Icon per asset type. Generic `STACK` for anything not explicitly mapped.
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
