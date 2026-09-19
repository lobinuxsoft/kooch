//! A row's right-click menu.

use super::*;

pub(super) fn handle_context_menu(
    resp: &egui::Response,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    pinned: &mut HashSet<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    clipboard_has_entities: bool,
    actions: &mut Vec<EditorAction>,
) {
    resp.context_menu(|ui| {
        // 🔴 Without this the menu takes egui's default and every label wraps — "Duplicate" one
        // character per line. It was always too narrow; adding entries that say their chord is what
        // made it impossible to miss. Same idiom as the View and Game panels' menus.
        ui.set_min_width(240.0);

        // Ensure the right-clicked entity is selected.
        if !selected.contains(&info.entity) {
            selected.clear();
            selected.push(info.entity);
        }

        // Pinning is what makes a gizmo answerable without keeping the entity selected — you pin
        // the camera you are aiming, then go move the thing it follows.
        let all_pinned = selected.iter().all(|e| pinned.contains(e));
        let pin_label = match (all_pinned, selected.len()) {
            (true, 1) => format!("{} Unpin gizmos", icons::EYE),
            (true, n) => format!("{} Unpin gizmos ({n})", icons::EYE),
            (false, 1) => format!("{} Pin gizmos", icons::EYE),
            (false, n) => format!("{} Pin gizmos ({n})", icons::EYE),
        };
        if ui
            .button(pin_label)
            .on_hover_text("Keep this entity's gizmos drawn while something else is selected")
            .clicked()
        {
            for entity in selected.iter() {
                if all_pinned {
                    pinned.remove(entity);
                } else {
                    pinned.insert(*entity);
                }
            }
            ui.close();
        }
        ui.separator();

        // The clipboard three, read out of the same table the keyboard reads. They live here rather
        // than on a toolbar because the pointer is what names the selection and the place — and two
        // lists of the same commands is one list that drifts.
        for chord in [
            crate::shortcuts::EditChord::Duplicate,
            crate::shortcuts::EditChord::Copy,
            crate::shortcuts::EditChord::Paste,
        ] {
            let enabled = match chord {
                crate::shortcuts::EditChord::Paste => clipboard_has_entities,
                _ => true,
            };
            let icon = match chord {
                crate::shortcuts::EditChord::Paste => icons::PACKAGE,
                _ => icons::COPY,
            };
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(format!("{icon} {}", chord.label()))
                        .shortcut_text(chord.chord()),
                )
                .on_hover_text(chord.tooltip())
                .clicked()
            {
                // 🔴 Paste is rebuilt with THIS entity's scene rather than taken from the table. A
                // menu opened on a row names a place; the chord that fills the table has no pointer
                // and so names the active scene.
                match (chord, info.scene) {
                    (crate::shortcuts::EditChord::Paste, Some(scene)) => {
                        actions.push(EditorAction::PasteEntities {
                            into: crate::actions::SpawnTarget::Scene(scene),
                        });
                    }
                    _ => actions.extend(crate::shortcuts::actions_for(
                        chord,
                        selected,
                        Some(&crate::history::Document::World),
                    )),
                }
                ui.close();
            }
        }
        ui.separator();

        // Two destinations, because they are two different intents and guessing between them is how
        // an entity ends up somewhere the user has to go find it.
        ui.menu_button("New Child", |ui| {
            super::spawn_entries(
                ui,
                actions,
                crate::actions::SpawnTarget::ChildOf(info.entity),
            );
        });
        // 🔴 Offered even with no scene, where it used to be hidden — and
        // hiding it left `New Child` as the only spawn a row had, so an
        // entity in no scene could only ever be nested under (#1033).
        let (label, root) = root_target(info.scene);
        ui.menu_button(label, |ui| {
            super::spawn_entries(ui, actions, root);
        });
        ui.separator();

        let count = selected.len();
        let label = if count == 1 {
            format!("{} Despawn", icons::TRASH)
        } else {
            format!("{} Despawn {} entities", icons::TRASH, count)
        };

        if ui.button(label).clicked() {
            for entity in selected.drain(..) {
                actions.push(EditorAction::Despawn(entity));
            }
            ui.close();
        }

        // Only on an instance. Reverting is the operation that makes an override safe to have:
        // without it an accidental gizmo drag detaches that transform from the prefab forever, and
        // the only way back is deleting the instance and placing a new one.
        if selected.len() == 1 && info.is_prefab_instance {
            let entity = selected[0];
            if ui
                .button(format!("{} Revert to Prefab", icons::ARROWS_CLOCKWISE))
                .on_hover_text("Drop this instance's changes and follow the prefab again")
                .clicked()
            {
                actions.push(EditorAction::RevertToPrefab {
                    entity,
                    component: None,
                });
                ui.close();
            }
        }

        // One entity only. A prefab is one tree with one root — see `SceneDocument::root_index` —
        // so N selected entities are either N prefabs or one thing that is not a tree, and neither
        // is what this menu item means.
        if selected.len() == 1 {
            let entity = selected[0];
            // The same glyph the asset tree shows for a prefab file, since
            // that is what this produces.
            if ui
                .button(format!("{} Save as Prefab", icons::PACKAGE))
                .on_hover_text("Write this entity and its children to a scene file in assets/")
                .clicked()
            {
                actions.push(EditorAction::SavePrefab {
                    entity,
                    dest: None,
                    overwrite: false,
                });
                ui.close();
            }
        }

        // Add Component submenu (only for single entity).
        if selected.len() == 1 {
            let entity = selected[0];
            let existing: HashSet<ComponentId> = entities
                .iter()
                .find(|e| e.entity == entity)
                .map(|e| e.components.iter().map(|c| c.component).collect())
                .unwrap_or_default();

            let available: Vec<&ReflectedTypeInfo> = reflected_types
                .iter()
                .filter(|t| !existing.contains(&t.component))
                .collect();

            if !available.is_empty() {
                ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
                    crate::panels::add_component_menu::draw_categorized(
                        ui,
                        &available,
                        |component| {
                            actions.push(EditorAction::AddComponent { entity, component });
                        },
                    );
                });
            }
        } else if selected.len() > 1 {
            // Multi-select: add component to all selected.
            let all: Vec<&ReflectedTypeInfo> = reflected_types.iter().collect();
            ui.menu_button(format!("{} Add Component to all", icons::PLUS), |ui| {
                crate::panels::add_component_menu::draw_categorized(ui, &all, |component| {
                    for &entity in selected.iter() {
                        actions.push(EditorAction::AddComponent { entity, component });
                    }
                });
            });

            // Multi-select: remove shared component from all selected.
            // Collect components present in ALL selected entities.
            let selected_infos: Vec<&EntityDisplayInfo> = entities
                .iter()
                .filter(|e| selected.contains(&e.entity))
                .collect();

            if !selected_infos.is_empty() {
                let first = selected_infos[0];
                let mut shared: Vec<(ComponentId, std::borrow::Cow<'static, str>)> = first
                    .components
                    .iter()
                    .filter(|c| {
                        selected_infos[1..].iter().all(|info| {
                            info.components.iter().any(|ic| ic.component == c.component)
                        })
                    })
                    .map(|c| (c.component, c.short_name.clone()))
                    .collect();
                shared.sort_by(|a, b| a.1.cmp(&b.1));

                if !shared.is_empty() {
                    ui.menu_button(
                        format!("{} Remove Component from all", icons::MINUS),
                        |ui| {
                            for (component, name) in &shared {
                                if ui.selectable_label(false, name.as_ref()).clicked() {
                                    for &entity in selected.iter() {
                                        actions.push(EditorAction::RemoveComponent {
                                            entity,
                                            component: *component,
                                        });
                                    }
                                    ui.close();
                                }
                            }
                        },
                    );
                }
            }
        }
    });
}
