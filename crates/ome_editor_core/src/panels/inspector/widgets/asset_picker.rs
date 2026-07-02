//! Typed asset-reference picker for `ReflectValue::AssetRef` fields.

use ome_core::Guid;
use ome_ecs::reflect::ReflectValue;

use super::asset::AssetCatalogEntry;

/// Renders the typed asset-reference picker for a `ReflectValue::AssetRef`
/// field. Returns `Some(new_value)` when the user picks a different
/// asset (or clears the field), otherwise `None`.
///
/// Layout follows the Unity inspector's object-field convention:
/// - Selected state shows the current asset's basename + source tag,
///   or `(None)` / `(missing: <guid>)` when unresolvable.
/// - Dropdown opens a search field at the top, then the filtered
///   list. Each row shows `display_name [source]` with the full
///   path on hover.
pub(super) fn draw_asset_picker(
    ui: &mut egui::Ui,
    current: Option<Guid>,
    asset_type: &str,
    catalog: &[AssetCatalogEntry],
) -> Option<ReflectValue> {
    let filtered: Vec<&AssetCatalogEntry> = catalog
        .iter()
        .filter(|e| e.type_name == asset_type)
        .collect();

    let current_entry = current.and_then(|g| filtered.iter().find(|e| e.guid == g).copied());

    let selected_text = match (current, current_entry) {
        (Some(_), Some(entry)) => format!("{} [{}]", entry.display_name, entry.source.label()),
        (Some(g), None) => format!("(missing: {g})"),
        (None, _) => "(None)".to_owned(),
    };

    let mut new_value: Option<ReflectValue> = None;
    let search_id = ui.id().with(("asset_picker_search", asset_type));

    let combo_response = egui::ComboBox::from_id_salt(("asset_picker", asset_type))
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            // Search box — Unity-style. Persisted in egui memory so
            // the query survives close-and-reopen of the same
            // dropdown across frames.
            let mut query: String = ui
                .ctx()
                .data(|d| d.get_temp::<String>(search_id))
                .unwrap_or_default();
            let search_resp = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .desired_width(f32::INFINITY)
                    .hint_text("\u{1f50d} Search…"),
            );
            if search_resp.changed() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(search_id, query.clone()));
            }

            ui.separator();

            // The "(None)" entry — clears the assignment. Always
            // visible, never filtered out.
            if ui.selectable_label(current.is_none(), "(None)").clicked() && current.is_some() {
                new_value = Some(ReflectValue::AssetRef {
                    guid: None,
                    asset_type: asset_type.to_owned(),
                });
            }

            if filtered.is_empty() {
                ui.weak(format!("(no {asset_type} assets registered)"));
                return;
            }

            let needle = query.trim().to_lowercase();
            let matches_query = |entry: &AssetCatalogEntry| -> bool {
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

            let mut shown = 0usize;
            for entry in &filtered {
                if !matches_query(entry) {
                    continue;
                }
                let selected = current == Some(entry.guid);
                let label = format!("{}  [{}]", entry.display_name, entry.source.label(),);
                let resp = ui
                    .selectable_label(selected, label)
                    .on_hover_text(entry.path.display().to_string());
                if !selected && resp.clicked() {
                    new_value = Some(ReflectValue::AssetRef {
                        guid: Some(entry.guid),
                        asset_type: asset_type.to_owned(),
                    });
                }
                shown += 1;
            }
            if shown == 0 {
                ui.weak("(no match)");
            }
        });
    let _ = combo_response;

    new_value
}
