//! Right-click menus for folders and files. Writable roots only — the
//! engine's shipped assets are read-only from here.

use std::path::Path;

use crate::actions::{EditorAction, NewFileKind};
use crate::icons;

use super::model::{CreateKind, FileLeaf, FolderNode, FolderRole, PendingCreate, RenameState};

pub(super) fn folder_menu(
    ui: &mut egui::Ui,
    node: &FolderNode,
    writable: bool,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
    pending: &mut Option<PendingCreate>,
    has_settings: bool,
    role: FolderRole,
) {
    // Offered on folders too, and on read-only ones: opening the crate
    // that owns a folder is how you get at the code behind it, which is
    // the whole point of the entry. The handler resolves the workspace,
    // so this works the same on a project folder and an engine one.
    if ui.button("Open in IDE").clicked() {
        actions.push(EditorAction::OpenInIde {
            file: node.path.clone(),
        });
        ui.close();
    }
    if !writable {
        if ui.button("Reveal in file manager").clicked() {
            actions.push(EditorAction::RevealInFileManager {
                path: node.path.clone(),
            });
            ui.close();
        }
        return;
    }
    // "New X" starts an inline name prompt rather than creating straight
    // away, so the typed name lands in the file name and the template.
    let mut start = |kind: CreateKind| {
        *pending = Some(PendingCreate {
            parent: node.path.clone(),
            kind,
            buffer: String::new(),
            focused: false,
        });
    };
    if ui
        .button(format!("{} New Folder", icons::FOLDER_PLUS))
        .clicked()
    {
        start(CreateKind::Folder);
        ui.close();
    }

    // 🔴 Every entry below is disabled outside the tree that would
    // register it. The editor scans `assets/` for assets and `src/` for
    // scripts, so a file created anywhere else is a file nothing reads —
    // no error, no GUID, no compile, just a file. Disabled with the
    // reason beats offering an action whose result is silence.
    let mut entry = |ui: &mut egui::Ui, label: String, wants: FolderRole| {
        let refusal = role.refusal(wants);
        let resp = ui.add_enabled(refusal.is_none(), egui::Button::new(label));
        match refusal {
            Some(why) => {
                resp.on_disabled_hover_text(why);
                false
            }
            None => resp.clicked(),
        }
    };

    if entry(
        ui,
        format!("{} New Material", icons::FADERS),
        FolderRole::Assets,
    ) {
        start(CreateKind::Material);
        ui.close();
    }
    // Scripts are code and go under `src/`; a scene and an input map are
    // assets and go under `assets/`. They were one menu called
    // "New Script / Scene", which put three unrelated things behind one
    // label and made the input map hard to find precisely because it
    // belonged to none of them.
    ui.menu_button(format!("{} New Script", icons::PLUS), |ui| {
        for (label, kind) in [
            (
                "Component (Rust)",
                CreateKind::File(NewFileKind::RustComponent),
            ),
            ("System (Rust)", CreateKind::File(NewFileKind::RustSystem)),
        ] {
            if entry(ui, label.to_owned(), FolderRole::Source) {
                start(kind);
                ui.close();
            }
        }
    });
    if entry(
        ui,
        format!("{} New Scene", icons::GLOBE),
        FolderRole::Assets,
    ) {
        start(CreateKind::File(NewFileKind::Scene));
        ui.close();
    }
    if entry(
        ui,
        format!("{} New Input Action", icons::GAME_CONTROLLER),
        FolderRole::Assets,
    ) {
        start(CreateKind::File(NewFileKind::InputAction));
        ui.close();
    }
    // Settings are per project, and the renderer finds them by type: a
    // second file is read by nothing and warns where nobody looks. Shown
    // disabled rather than hidden, so a project that already has one says
    // so instead of leaving someone hunting for a menu entry that was
    // there yesterday.
    let allowed = role.refusal(FolderRole::Assets).is_none() && !has_settings;
    let settings = ui.add_enabled(
        allowed,
        egui::Button::new(format!("{} New Render Settings", icons::FADERS)),
    );
    if !allowed {
        settings.on_disabled_hover_text(
            role.refusal(FolderRole::Assets)
                .unwrap_or("This project already has one — settings are per project."),
        );
    } else if settings.clicked() {
        start(CreateKind::File(NewFileKind::RenderSettings));
        ui.close();
    }
    // The synthetic root node has an empty name; it is not itself
    // renamable / deletable (that would target the crate root).
    if !node.name.is_empty() {
        ui.separator();
        if ui.button("Rename").clicked() {
            *rename = Some(RenameState {
                path: node.path.clone(),
                buffer: node.name.clone(),
                focused: false,
            });
            ui.close();
        }
        if ui.button(format!("{} Delete", icons::TRASH)).clicked() {
            actions.push(EditorAction::DeleteFolder {
                path: node.path.clone(),
            });
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Reveal in file manager").clicked() {
        actions.push(EditorAction::RevealInFileManager {
            path: node.path.clone(),
        });
        ui.close();
    }
}

pub(super) fn leaf_menu(
    ui: &mut egui::Ui,
    leaf: &FileLeaf,
    writable: bool,
    _root: &Path,
    actions: &mut Vec<EditorAction>,
    rename: &mut Option<RenameState>,
) {
    // Prefabs only. A scene is the same format but not the same invariant:
    // it may have any number of roots, and instancing needs exactly one.
    // Offering this on a scene meant a four-root scene failed at the click
    // instead of never being offered — see `project::PREFAB_EXTENSION`.
    //
    // Instancing adds to the open scene, unlike File > Open Scene which
    // replaces it.
    if leaf
        .path
        .extension()
        .is_some_and(|ext| ext == crate::project::PREFAB_EXTENSION)
        && ui
            .button(format!("{} Instantiate into Scene", icons::PACKAGE))
            .clicked()
    {
        // Only a *registered* prefab can be instanced: the guid is what
        // both the local spawn and the wire call address it by. An
        // unregistered file has no identity yet, and the menu says nothing
        // rather than offering an action that would fail.
        if let Some((guid, _)) = &leaf.asset {
            actions.push(EditorAction::InstantiatePrefab {
                prefab: *guid,
                at: crate::viewport_pick::DropPoint::Authored,
            });
        }
        ui.close();
    }
    if ui.button("Open in IDE").clicked() {
        actions.push(EditorAction::OpenInIde {
            file: leaf.path.clone(),
        });
        ui.close();
    }
    if writable {
        ui.separator();
        if ui.button("Rename").clicked() {
            // Edit the name before the first extension; the suffix
            // (`.ron`, `.rs`, `.kooch_material.ron`, …) is re-attached on
            // commit.
            let stem = leaf.name.split('.').next().unwrap_or(&leaf.name).to_owned();
            *rename = Some(RenameState {
                path: leaf.path.clone(),
                buffer: stem,
                focused: false,
            });
            ui.close();
        }
        if ui.button(format!("{} Duplicate", icons::COPY)).clicked() {
            actions.push(EditorAction::DuplicateAsset {
                path: leaf.path.clone(),
            });
            ui.close();
        }
        if ui.button(format!("{} Delete", icons::TRASH)).clicked() {
            actions.push(EditorAction::DeleteAsset {
                path: leaf.path.clone(),
            });
            ui.close();
        }
        // Rust sources can be (re)registered as components / systems by
        // rescanning src/ — the editor detects which they are.
        if leaf.name.ends_with(".rs")
            && ui
                .button(format!("{} Register scripts", icons::PLUS))
                .clicked()
        {
            actions.push(EditorAction::RegisterScripts);
            ui.close();
        }
    }
    if let Some((guid, _)) = &leaf.asset {
        ui.separator();
        if ui.button("Copy GUID").clicked() {
            ui.ctx().copy_text(guid.to_string());
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Reveal in file manager").clicked() {
        actions.push(EditorAction::RevealInFileManager {
            path: leaf.path.clone(),
        });
        ui.close();
    }
}
