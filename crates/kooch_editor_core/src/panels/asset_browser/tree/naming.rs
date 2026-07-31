//! Inline rename and create-new, and the validation both share.

use std::path::Path;

use crate::actions::{EditorAction, NewFileKind};

use super::model::{CreateKind, PendingCreate, RenameState};

pub(super) fn rename_edit(
    ui: &mut egui::Ui,
    path: &Path,
    is_folder: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    let Some(state) = rename.as_mut() else {
        return;
    };
    let resp = ui.add(egui::TextEdit::singleline(&mut state.buffer).desired_width(f32::INFINITY));
    if !state.focused {
        resp.request_focus();
        state.focused = true;
    }
    let buffer = state.buffer.clone();

    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !escape && !resp.lost_focus() {
        return;
    }
    if resp.lost_focus() && enter && !escape {
        if let Some(new_name) = commit_name(path, &buffer, is_folder) {
            actions.push(if is_folder {
                EditorAction::RenameFolder {
                    path: path.to_path_buf(),
                    new_name,
                }
            } else {
                EditorAction::RenameAsset {
                    path: path.to_path_buf(),
                    new_name,
                }
            });
        }
    }
    *rename = None;
}

/// Builds the new file/folder name from the edited stem, re-attaching a
/// file's extension suffix. Returns `None` for an empty edit.
pub(super) fn commit_name(path: &Path, buffer: &str, is_folder: bool) -> Option<String> {
    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_folder {
        return Some(trimmed.to_owned());
    }
    let filename = path.file_name()?.to_str()?;
    let suffix = filename.find('.').map(|i| &filename[i..]).unwrap_or("");
    Some(format!("{trimmed}{suffix}"))
}

/// Renders the inline creation name field. Commits on Enter (non-empty),
/// cancels on Escape / focus loss.
pub(super) fn create_edit(
    ui: &mut egui::Ui,
    pending: &mut Option<PendingCreate>,
    actions: &mut Vec<EditorAction>,
) {
    let Some(p) = pending.as_mut() else {
        return;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(&mut p.buffer)
            .desired_width(f32::INFINITY)
            .hint_text(create_hint(p.kind)),
    );
    if !p.focused {
        resp.request_focus();
        p.focused = true;
    }
    let name = p.buffer.trim().to_owned();
    let kind = p.kind;
    let parent = p.parent.clone();

    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if !escape && !resp.lost_focus() {
        return;
    }
    if resp.lost_focus() && enter && !escape && !name.is_empty() {
        // A new component / system auto-registers so it works without a
        // manual step; the CreateFile above runs first (writes the file),
        // then RegisterScripts rescans src/ and picks it up.
        let auto_register = matches!(
            kind,
            CreateKind::File(NewFileKind::RustComponent | NewFileKind::RustSystem)
        );
        actions.push(match kind {
            CreateKind::Folder => EditorAction::CreateFolder { parent, name },
            CreateKind::Material => EditorAction::CreateMaterial {
                folder: parent,
                name,
            },
            CreateKind::File(kind) => EditorAction::CreateFile {
                folder: parent,
                name,
                kind,
            },
        });
        if auto_register {
            actions.push(EditorAction::RegisterScripts);
        }
    }
    *pending = None;
}

pub(super) fn create_hint(kind: CreateKind) -> &'static str {
    match kind {
        CreateKind::Folder => "Folder name…",
        CreateKind::Material => "Material name…",
        CreateKind::File(NewFileKind::RustComponent) => "Component name (e.g. Health)…",
        CreateKind::File(NewFileKind::RustSystem) => "System name (e.g. Movement)…",
        CreateKind::File(NewFileKind::Scene) => "Scene name…",
    }
}
