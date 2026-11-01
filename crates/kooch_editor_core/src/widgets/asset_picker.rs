//! The editor's asset picker: a search box over the catalog's assets of one type, "(None)" to
//! clear, and an asset dragged from the Asset Browser dropped on it. Every asset field uses it.

use kooch_core::Guid;

use crate::drag_drop::DraggedAsset;
use crate::panels::inspector::AssetCatalogEntry;

/// `Some(new)` when the user picks another asset; `Some(None)` clears it. `skip` hides one asset,
/// like the file a template picker sits in.
pub(crate) fn asset_picker(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: Option<Guid>,
    asset_type: &str,
    catalog: &[AssetCatalogEntry],
    skip: Option<Guid>,
) -> Option<Option<Guid>> {
    let filtered: Vec<&AssetCatalogEntry> = catalog
        .iter()
        .filter(|entry| entry.type_name == asset_type && Some(entry.guid) != skip)
        .collect();
    let current_entry = current.and_then(|guid| filtered.iter().find(|e| e.guid == guid).copied());
    let selected_text = match (current, current_entry) {
        (Some(_), Some(entry)) => format!("{} [{}]", entry.display_name, entry.source.label()),
        (Some(guid), None) => format!("(missing: {guid})"),
        (None, _) => "(None)".to_owned(),
    };

    let mut picked = None;
    let slot = super::search_combo(ui, id_salt, selected_text, |ui, needle| {
        // "(None)" clears the assignment, and is never filtered out.
        if ui.selectable_label(current.is_none(), "(None)").clicked() {
            if current.is_some() {
                picked = Some(None);
            }
            ui.close();
        }
        if filtered.is_empty() {
            ui.weak(format!("(no {asset_type} assets registered)"));
            return;
        }
        let matches = |entry: &AssetCatalogEntry| {
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
        for entry in filtered.iter().filter(|entry| matches(entry)) {
            let selected = current == Some(entry.guid);
            let label = format!("{}  [{}]", entry.display_name, entry.source.label());
            let response = ui
                .selectable_label(selected, label)
                .on_hover_text(entry.path.display().to_string());
            if response.clicked() {
                if !selected {
                    picked = Some(Some(entry.guid));
                }
                ui.close();
            }
            shown += 1;
        }
        if shown == 0 {
            ui.weak("(no match)");
        }
    });
    // Only this slot's own type is accepted, so a mesh dragged over a material field neither
    // highlights nor assigns.
    if let Some(hovered) = slot.dnd_hover_payload::<DraggedAsset>()
        && hovered.type_name == asset_type
        && Some(hovered.guid) != skip
    {
        ui.painter().rect_filled(
            slot.rect,
            2.0,
            egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
        );
        if let Some(released) = slot.dnd_release_payload::<DraggedAsset>()
            && current != Some(released.guid)
        {
            picked = Some(Some(released.guid));
        }
    }
    picked
}
