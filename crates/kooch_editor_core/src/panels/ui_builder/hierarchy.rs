//! The document's elements as the file nests them; a `<use>` is one row, as Unity shows a template.
//! Rows drag to reorder and take what the Library drops; the context menu edits the element.

use kooch_ui::markup::Place;

use super::drop::{self, Dragged};
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::UiBuilder;

/// Points per nesting level.
const INDENT: f32 = 12.0;

enum Edit {
    Rename(u32, String),
    Duplicate(u32),
    Remove(u32),
}

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, catalog: &[AssetCatalogEntry]) {
    ui.strong("Hierarchy");
    ui.separator();
    let rename_id = ui.make_persistent_id("ui_builder_rename");
    let mut renaming: Option<(u32, String)> = ui.data(|data| data.get_temp(rename_id));
    let dragged = drop::current(ui.ctx());
    let accent = ui.visuals().selection.stroke.color;
    let mut edit = None;
    let mut landing = None;
    egui::ScrollArea::vertical()
        .id_salt("ui_builder_hierarchy")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in builder.outline() {
                let origin = row.origin;
                let row_rect = ui
                    .horizontal(|ui| {
                        ui.add_space(row.depth as f32 * INDENT);
                        if let Some((at, text)) = renaming.as_mut().filter(|(at, _)| *at == origin)
                        {
                            let field = ui.text_edit_singleline(text);
                            if !field.has_focus() && !field.lost_focus() {
                                field.request_focus();
                            }
                            if field.lost_focus() {
                                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                                    edit = Some(Edit::Rename(*at, text.trim().to_owned()));
                                }
                                renaming = None;
                            }
                            return;
                        }
                        let response = ui
                            .selectable_label(builder.selected == Some(origin), label(&row))
                            .interact(egui::Sense::drag());
                        if response.clicked() {
                            builder.selected = Some(origin);
                        }
                        if origin == 0 {
                            return;
                        }
                        if response.drag_started() {
                            egui::DragAndDrop::set_payload(ui.ctx(), Dragged::Row(origin));
                        }
                        if response.double_clicked() {
                            renaming = Some((origin, row.name.clone().unwrap_or_default()));
                        }
                        response.context_menu(|ui| {
                            if ui.button("Rename").clicked() {
                                renaming = Some((origin, row.name.clone().unwrap_or_default()));
                            }
                            if ui.button("Duplicate    Ctrl+D").clicked() {
                                edit = Some(Edit::Duplicate(origin));
                            }
                            if ui.button("Delete       Del").clicked() {
                                edit = Some(Edit::Remove(origin));
                            }
                        });
                    })
                    .response
                    .rect;
                let row_rect = row_rect
                    .with_max_x(ui.max_rect().max.x)
                    .intersect(ui.clip_rect());
                if dragged.is_some()
                    && let Some(pointer) = ui.ctx().pointer_latest_pos()
                    && row_rect.contains(pointer)
                {
                    let fraction = (pointer.y - row_rect.top()) / row_rect.height().max(1.0);
                    let at = drop::place_on(builder, origin, fraction);
                    drop::mark(ui.painter(), row_rect, at, accent);
                    landing = Some(at);
                }
            }
            // Below the last row: the end of the document.
            let rest = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            if dragged.is_some()
                && landing.is_none()
                && ui
                    .ctx()
                    .pointer_latest_pos()
                    .is_some_and(|p| rest.contains(p))
            {
                let line = rest.with_max_y(rest.top() + 1.0);
                drop::mark(ui.painter(), line, Place::After(0), accent);
                landing = Some(Place::Inside(0));
            }
        });
    ui.data_mut(|data| match renaming {
        Some(renaming) => {
            data.insert_temp(rename_id, renaming);
        }
        None => data.remove::<(u32, String)>(rename_id),
    });
    if let Some(at) = landing
        && let Some(dragged) = drop::released(ui.ctx())
    {
        drop::apply(builder, dragged, at, catalog);
    }
    shortcuts(ui, builder, &mut edit);
    match edit {
        Some(Edit::Rename(origin, name)) => builder.set_attribute(origin, "name", &name),
        Some(Edit::Duplicate(origin)) => builder.duplicate(origin),
        Some(Edit::Remove(origin)) => builder.remove(origin),
        None => {}
    }
}

/// Only while the pointer is over the hierarchy and nothing types: Delete in a text field is text.
fn shortcuts(ui: &egui::Ui, builder: &UiBuilder, edit: &mut Option<Edit>) {
    let Some(origin) = builder.selected.filter(|origin| *origin != 0) else {
        return;
    };
    if !ui.ui_contains_pointer() || ui.memory(|memory| memory.focused().is_some()) {
        return;
    }
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::D) {
            *edit = Some(Edit::Duplicate(origin));
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
            *edit = Some(Edit::Remove(origin));
        }
    });
}

fn label(row: &kooch_ui::markup::Outline) -> String {
    let mut label = format!("<{}>", row.tag);
    if let Some(name) = &row.name {
        label.push_str(&format!(" #{name}"));
    }
    if let Some(classes) = &row.classes {
        for class in classes.split_whitespace() {
            label.push_str(&format!(" .{class}"));
        }
    }
    label
}
