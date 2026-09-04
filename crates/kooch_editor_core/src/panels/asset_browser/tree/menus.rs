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
    // A shape on its own, for a block meant to be shared. The usual way
    // in is the World panel's Spawn menu, which makes the asset and the
    // entity together — a block is normally authored per entity.
    if entry(
        ui,
        format!("{} New Block Mesh", icons::CUBE),
        FolderRole::Assets,
    ) {
        start(CreateKind::File(NewFileKind::BlockMesh));
        ui.close();
    }
    // Several per project, unlike settings: "Windows release" and
    // "Linux debug" are two presets rather than one with a switch, which
    // is the whole reason Godot's export presets are a list.
    if entry(
        ui,
        format!("{} New Build Preset", icons::PACKAGE),
        FolderRole::Assets,
    ) {
        start(CreateKind::File(NewFileKind::BuildPreset));
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
    is_main_scene: bool,
) {
    ui.set_min_width(240.0);
    // Opening comes first: it is what a scene is FOR, and it was the one
    // thing this menu could not do. Reaching a scene meant File > Open
    // Scene and navigating to the file already under the pointer.
    //
    // 🔴 Keyed on the extension, not on `offers_main_scene`, which also
    // asks whether the folder is writable. Opening a read-only scene --
    // one vendored with the engine -- is a perfectly ordinary thing to
    // want; setting it as the project's main scene is not.
    if is_scene(&leaf.path) {
        if ui
            .button(format!("{} Open Scene", icons::FOLDER_OPEN))
            .on_hover_text("Replace what is open with this scene")
            .clicked()
        {
            actions.push(EditorAction::OpenScene {
                path: Some(leaf.path.clone()),
            });
            ui.close();
        }
        if ui
            .button(format!("{} Open Additive", icons::PLUS))
            .on_hover_text("Open this scene beside the ones already open")
            .clicked()
        {
            actions.push(EditorAction::OpenSceneAdditive {
                path: Some(leaf.path.clone()),
            });
            ui.close();
        }
        ui.separator();
    }
    if offers_main_scene(&leaf.path, writable) {
        // Offered as disabled rather than hidden on the scene that
        // already is the main one: a menu that changes shape depending on
        // a state nothing else displays is how you end up right-clicking
        // three scenes to find out which is which. The badge in the tree
        // says which; this says it again where the question was asked.
        let entry = egui::Button::new(format!("{} Set as Main Scene", icons::GLOBE));
        let resp = ui.add_enabled(!is_main_scene, entry);
        if is_main_scene {
            resp.on_disabled_hover_text("Already the scene this project opens with");
        } else if resp.clicked() {
            actions.push(EditorAction::SetMainScene {
                path: leaf.path.clone(),
            });
            ui.close();
        }
        ui.separator();
    }
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
            // (`.material`, `.rs`, `.block`, …) is re-attached on
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

/// Whether "Set as Main Scene" belongs on this file's menu (#808).
///
/// Scenes only, and only under a writable root. A `.material` is not
/// something a game can open with, and the engine's shipped assets are
/// somebody else's — pointing a project's manifest at one would store a
/// path outside the project, which the handler refuses anyway.
///
/// 🔴 A prefab is the same format with a different extension and must
/// **not** qualify: it carries exactly one root entity, so a game opening
/// one would start with a single object and no camera. The extension is
/// the only thing separating them — see `PREFAB_EXTENSION`.
pub(super) fn offers_main_scene(path: &Path, writable: bool) -> bool {
    writable && is_scene(path)
}

/// Whether this file is a scene rather than a prefab.
///
/// The extension is the only thing separating them: same format, and a
/// prefab carries exactly one root.
pub(super) fn is_scene(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == crate::project::SCENE_EXTENSION)
}

#[cfg(test)]
mod main_scene_entry_tests;
