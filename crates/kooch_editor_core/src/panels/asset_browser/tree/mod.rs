//! Folder-tree model, menus and naming for the Asset Browser; `ui_tree` draws it.

mod menus;
mod model;
mod naming;
mod nav;
mod visuals;

pub(in crate::panels) use menus::{folder_menu, leaf_menu, open_action};
pub(in crate::panels) use model::{FileLeaf, FolderNode, FolderRole, PendingCreate, RenameState};
pub(in crate::panels) use naming::{create_edit, rename_edit};
pub(crate) use nav::AssetNav;
pub(in crate::panels) use nav::AssetRow;
pub(in crate::panels) use visuals::draw_drag_preview;

#[cfg(test)]
mod role_tests;
