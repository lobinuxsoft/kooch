//! The row of open scenes above the entity list.
//!
//! With one scene the World panel was the scene. With several, the panel
//! has to say which scenes are open, which one new entities land in, and
//! offer a way to close one without closing the others.

use crate::actions::EditorAction;
use crate::icons;
use crate::state::SceneDisplayInfo;

/// Draws the open-scene row. Hidden when only one scene is open, since
/// there is nothing to choose between.
pub(super) fn draw_scene_bar(
    ui: &mut egui::Ui,
    scenes: &[SceneDisplayInfo],
    actions: &mut Vec<EditorAction>,
) {
    if scenes.len() < 2 {
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.label("Scenes:");
        for scene in scenes {
            // An asterisk rather than a colour: the dirty marker has to
            // survive both themes and colour-blind viewers.
            let label = if scene.dirty {
                format!("{}*", scene.name)
            } else {
                scene.name.clone()
            };

            let response =
                ui.selectable_label(scene.active, label)
                    .on_hover_text(if scene.active {
                        "Active — new entities are created in this scene"
                    } else {
                        "Click to make this the active scene"
                    });
            if response.clicked() && !scene.active {
                actions.push(EditorAction::SetActiveScene(scene.id));
            }

            if ui
                .small_button(icons::X)
                .on_hover_text(if scene.dirty {
                    "Close this scene — it has unsaved changes"
                } else {
                    "Close this scene"
                })
                .clicked()
            {
                actions.push(EditorAction::CloseScene(scene.id));
            }
        }
    });
    ui.separator();
}
