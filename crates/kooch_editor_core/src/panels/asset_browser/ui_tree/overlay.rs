//! What egui still draws over the tree's rows: the menus, the name fields, the drag payload and
//! the prefab drop. Each finds its row through the panel, so the document stays the tree's alone.

use std::path::Path;

use kooch_ecs::entity::Entity;

use super::markup::Row;
use super::{Edits, Inputs};
use crate::actions::EditorAction;
use crate::drag_drop::DraggedAsset;
use crate::panels::asset_browser::tree::{
    FileLeaf, FolderNode, FolderRole, create_edit, draw_drag_preview, folder_menu, leaf_menu,
    rename_edit,
};

pub(super) struct Overlay<'a> {
    pub rows: &'a [Row],
    pub inputs: &'a Inputs<'a>,
    pub edits: Edits<'a>,
    pub actions: &'a mut Vec<EditorAction>,
    /// A row's rect on screen, while it shows.
    pub rect: &'a dyn Fn(usize) -> Option<egui::Rect>,
    /// The row under a screen point.
    pub row_at: &'a dyn Fn(egui::Pos2) -> Option<usize>,
}

impl Overlay<'_> {
    pub(super) fn tooltip(&self, response: egui::Response, hovered: Option<usize>) {
        let Some(row) = hovered.map(|index| &self.rows[index]) else {
            return;
        };
        // The badge says "this one is special"; the tooltip says what special means.
        let text = match row.main {
            true => format!("{}\nThe scene this project opens with", row.path.display()),
            false => row.path.display().to_string(),
        };
        response.on_hover_text(text);
    }

    /// The row a right-click landed on keeps its menu until the menu closes.
    pub(super) fn menu(&mut self, ui: &egui::Ui, response: &egui::Response, at: Option<usize>) {
        let id = response.id.with("row_menu");
        if response.secondary_clicked() {
            ui.data_mut(|data| match at {
                Some(index) => {
                    data.insert_temp(id, index);
                }
                None => data.remove::<usize>(id),
            });
        }
        let Some(row) = ui
            .data(|data| data.get_temp::<usize>(id))
            .and_then(|index| self.rows.get(index))
        else {
            return;
        };
        // The engine's root offers nothing: everything there is read-only.
        if row.folder && row.name.is_empty() && !row.writable {
            return;
        }
        let Self {
            inputs,
            edits,
            actions,
            ..
        } = self;
        response.context_menu(|ui| match row.folder {
            true => folder_menu(
                ui,
                &FolderNode {
                    name: row.name.clone(),
                    path: row.path.clone(),
                    folders: Default::default(),
                    files: Vec::new(),
                },
                row.writable,
                actions,
                edits.rename,
                edits.pending,
                inputs.has_settings,
                inputs.data_types,
                FolderRole::of(&row.path, inputs.project_root),
            ),
            false => leaf_menu(
                ui,
                &FileLeaf {
                    name: row.name.clone(),
                    path: row.path.clone(),
                    asset: row.asset.clone(),
                },
                row.writable,
                Path::new(""),
                actions,
                edits.rename,
                row.main,
            ),
        });
    }

    /// A typed asset dragged off its row is the payload the Inspector's slots and the scene take.
    pub(super) fn drag(&self, ui: &egui::Ui, response: &egui::Response, from: Option<usize>) {
        let id = response.id.with("row_drag");
        if let Some(index) = from
            && let Some((guid, type_name)) = &self.rows[index].asset
        {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                DraggedAsset {
                    guid: *guid,
                    type_name: type_name.clone(),
                },
            );
            ui.data_mut(|data| data.insert_temp(id, index));
        }
        let dragged = ui.data(|data| data.get_temp::<usize>(id));
        match (response.dragged(), dragged) {
            (true, Some(index)) => {
                let row = &self.rows[index];
                draw_drag_preview(ui, row.icon, &row.name);
            }
            (false, Some(_)) => ui.data_mut(|data| data.remove::<usize>(id)),
            _ => {}
        }
    }

    /// An entity dragged out of the World panel onto a project folder is saved there as a prefab.
    pub(super) fn prefab_drop(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if !egui::DragAndDrop::has_payload_of_type::<Entity>(ui.ctx())
            || !response.contains_pointer()
        {
            return;
        }
        let Some(index) = ui
            .ctx()
            .pointer_latest_pos()
            .and_then(|pos| (self.row_at)(pos))
        else {
            return;
        };
        let row = &self.rows[index];
        if !row.folder || !row.writable {
            return;
        }
        if let Some(rect) = (self.rect)(index) {
            ui.painter().rect_filled(
                rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
            );
        }
        if ui.input(|input| input.pointer.any_released())
            && let Some(entity) = egui::DragAndDrop::take_payload::<Entity>(ui.ctx())
        {
            self.actions.push(EditorAction::SavePrefab {
                entity: *entity,
                dest: Some(row.path.clone()),
                overwrite: false,
            });
        }
    }

    /// A rename edits over its row; a new file's name, under its folder's.
    pub(super) fn name_fields(&mut self, ui: &mut egui::Ui, creating: Option<usize>) {
        let renamed = self.edits.rename.as_ref().and_then(|state| {
            let index = self.rows.iter().position(|row| row.path == state.path)?;
            Some((index, (self.rect)(index)?))
        });
        if let Some((index, rect)) = renamed {
            let row = &self.rows[index];
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            rename_edit(
                &mut child,
                &row.path,
                row.folder,
                self.actions,
                self.edits.rename,
            );
        }
        if let Some(rect) = creating.and_then(|index| (self.rect)(index)) {
            let below = rect.translate(egui::vec2(0.0, rect.height()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(below));
            create_edit(&mut child, self.edits.pending, self.actions);
        }
    }
}
