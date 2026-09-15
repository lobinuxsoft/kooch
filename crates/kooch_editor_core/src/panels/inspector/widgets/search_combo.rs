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
    egui::ComboBox::from_id_salt(id)
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
            egui::ScrollArea::vertical()
                .id_salt((id, "list"))
                .max_height(list_height)
                .auto_shrink([false, true])
                .show(ui, |ui| list(ui, &needle));
        })
        .response
}
