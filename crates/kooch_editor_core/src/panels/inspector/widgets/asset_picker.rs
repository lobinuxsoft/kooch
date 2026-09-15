//! Typed asset-reference picker for `ReflectValue::AssetRef` fields.

use kooch_core::Guid;
use kooch_ecs::reflect::ReflectValue;

use super::asset::AssetCatalogEntry;
use crate::drag_drop::DraggedAsset;

/// Renders the typed asset-reference picker for a `ReflectValue::AssetRef` field. Returns
/// `Some(new_value)` when the user picks a different asset (or clears the field), otherwise `None`.
pub(crate) fn draw_asset_picker(
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

    let slot = super::search_combo::search_combo(
        ui,
        ("asset_picker", asset_type),
        selected_text,
        |ui, needle| {
            // "(None)" clears the assignment, and is never filtered out.
            if ui.selectable_label(current.is_none(), "(None)").clicked() {
                if current.is_some() {
                    new_value = Some(ReflectValue::AssetRef {
                        guid: None,
                        asset_type: asset_type.to_owned(),
                    });
                }
                ui.close();
            }

            if filtered.is_empty() {
                ui.weak(format!("(no {asset_type} assets registered)"));
                return;
            }

            let matches_query = |entry: &AssetCatalogEntry| -> bool {
                needle.is_empty()
                    || entry.display_name.to_lowercase().contains(needle)
                    || entry
                        .path
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(needle)
            };

            let mut shown = 0usize;
            for entry in filtered.iter().filter(|e| matches_query(e)) {
                let selected = current == Some(entry.guid);
                let label = format!("{}  [{}]", entry.display_name, entry.source.label());
                let resp = ui
                    .selectable_label(selected, label)
                    .on_hover_text(entry.path.display().to_string());
                if resp.clicked() {
                    if !selected {
                        new_value = Some(ReflectValue::AssetRef {
                            guid: Some(entry.guid),
                            asset_type: asset_type.to_owned(),
                        });
                    }
                    ui.close();
                }
                shown += 1;
            }
            if shown == 0 {
                ui.weak("(no match)");
            }
        },
    );
    // Drop target: an asset dragged out of the Asset Browser. Only this slot's own type is
    // accepted, so a mesh dragged over a material field neither highlights nor assigns.
    if let Some(hovered) = slot.dnd_hover_payload::<DraggedAsset>()
        && hovered.type_name == asset_type
    {
        ui.painter().rect_filled(
            slot.rect,
            2.0,
            egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
        );
        if let Some(released) = slot.dnd_release_payload::<DraggedAsset>()
            && current != Some(released.guid)
        {
            new_value = Some(ReflectValue::AssetRef {
                guid: Some(released.guid),
                asset_type: asset_type.to_owned(),
            });
        }
    }

    new_value
}
