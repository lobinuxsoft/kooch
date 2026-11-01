//! A dropdown whose search box stays above its scrolling list.

/// Draws a combo box with a search box as its fixed header and `list` scrolling below it. `list`
/// receives the lowercased, trimmed query and calls `ui.close()` when it picks something.
///
/// 🔴 `CloseOnClickOutside`: egui's default closes the popup on any click, the search box's included.
pub(crate) fn search_combo(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    selected_text: String,
    list: impl FnOnce(&mut egui::Ui, &str),
) -> egui::Response {
    let id = egui::Id::new(&id_salt);
    let search_id = ui.id().with((id, "search"));
    let list_height = ui.spacing().combo_height;
    let response = egui::ComboBox::from_id_salt(id)
        .selected_text(selected_text)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        // The list scrolls on its own; the popup never does, so the header cannot scroll away.
        .height(f32::INFINITY)
        .show_ui(ui, |ui| {
            let mut query: String = ui
                .ctx()
                .data(|d| d.get_temp::<String>(search_id))
                .unwrap_or_default();
            let search = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .desired_width(f32::INFINITY)
                    .hint_text("\u{1f50d} Search…"),
            );
            if search.changed() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(search_id, query.clone()));
            }
            ui.separator();
            let needle = query.trim().to_lowercase();
            // 🔴 The unfiltered list's height, held while filtering: a popup that shrinks to its
            // matches keeps that size after the query is cleared.
            let height_id = search_id.with("height");
            let held = ui.ctx().data(|d| d.get_temp::<f32>(height_id));
            let shown = egui::ScrollArea::vertical()
                .id_salt((id, "list"))
                .max_height(list_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_height(held.unwrap_or(0.0).min(list_height));
                    list(ui, &needle);
                });
            if needle.is_empty() {
                let height = shown.content_size.y.min(list_height);
                ui.ctx().data_mut(|d| d.insert_temp(height_id, height));
            }
        })
        .response;
    // Every opening starts unfiltered, so the first frame it draws is the one that sizes it.
    if !egui::ComboBox::is_open(ui.ctx(), response.id) {
        ui.ctx().data_mut(|d| d.remove::<String>(search_id));
    }
    response
}
