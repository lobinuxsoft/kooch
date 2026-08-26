//! Inspector panel — component details for selected entities.
//!
//! Split across submodules to keep each file under the project's
//! "no monolíticos" guideline:
//! - [`multi`]: rendering when multiple entities are selected (merged view).
//! - [`single`]: rendering when a single entity is selected (full per-component).
//! - [`rotation`]: gimbal-safe Quat editing with display-mode toggle (#202, #205).
//! - [`widgets`]: per-`ReflectValue` editor widgets and choice dropdowns.

mod asset_view;
mod mass_from_colliders;
mod nav;
mod prefab_view;
pub(crate) use nav::InspectorNav;
mod multi;
mod physics_warnings;
mod rotation;
mod single;
mod widgets;

#[cfg(test)]
mod id_stability;

use std::collections::{HashMap, HashSet};

use glam::{Quat, Vec3};

use kooch_core::Guid;
use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{InspectorVisibility, ReflectValue};

use crate::actions::EditorAction;
use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::{
    ComponentDisplayInfo, EntityDisplayInfo, EulerCacheKey, ReflectedTypeInfo, RotationDisplayMode,
};

pub(crate) use asset_view::{
    AssetDetail, ImageImportInfo, MeshImportInfo, PrefabComponentView, PrefabDetail,
    PrefabEntityView, ResolvedComponent,
};
pub(crate) use widgets::{AssetCatalogEntry, AssetSource, draw_asset_picker};

/// Threshold for considering a cached Euler still in sync with the
/// underlying quaternion. Compared against `|dot(actual, reconstructed)|`
/// since `q` and `-q` represent the same rotation.
pub(super) const EULER_CACHE_EPS: f32 = 1.0e-4;

/// Context for editing the `Transform.rotation` field in either Local
/// or World display mode. The World path uses `self_global` for display
/// and `parent_global` to convert edits back to local space.
#[derive(Clone, Copy)]
pub(super) struct RotationContext {
    pub(super) mode: RotationDisplayMode,
    /// World-space rotation of the entity itself, from `GlobalTransform`.
    pub(super) self_global: Option<Quat>,
    /// Parent's world-space rotation, if the entity has a parent with
    /// a `GlobalTransform`. `None` means treat the parent as identity
    /// (root entity or parent without propagated global transform).
    pub(super) parent_global: Option<Quat>,
}

impl RotationContext {
    pub(super) fn local_only() -> Self {
        Self {
            mode: RotationDisplayMode::Local,
            self_global: None,
            parent_global: None,
        }
    }
}

/// Content of the "Inspector" tab — component details for selected entities.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_inspector_content(
    ui: &mut egui::Ui,
    focused: bool,
    nav: &mut InspectorNav,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_display_mode: &mut RotationDisplayMode,
    asset_catalog: &[AssetCatalogEntry],
    selected_asset: Option<Guid>,
    asset_detail: Option<&AssetDetail>,
) {
    if focused {
        nav.handle_keyboard(ui);
    }
    // Refilled as sections are drawn, so next frame's arrows walk exactly
    // what is on screen now.
    nav.rows.clear();

    // Asset selection takes over the Inspector — it serves both entities
    // and assets. When an asset is selected, render its view and return.
    if let Some(guid) = selected_asset
        && let Some(entry) = asset_catalog.iter().find(|e| e.guid == guid)
    {
        asset_view::draw_asset_inspector(
            ui,
            entry,
            asset_detail,
            asset_catalog,
            euler_cache,
            entities,
            reflected_types,
            actions,
        );
        return;
    }

    // Evict cache entries for entities that are no longer selected.
    euler_cache.retain(|(entity, _, _, _), _| selected.contains(entity));

    // The Local/World rotation toggle moved to the viewport toolbar
    // (panels/view.rs). It now only renders when a selected entity has
    // a `Transform` component — see `View` panel for the gating logic.

    // Whole inspector area is a drop zone for DraggedComponent. On drop,
    // the component is added to every selected entity. See #209.
    let (_, dropped) = ui.dnd_drop_zone::<DraggedComponent, ()>(egui::Frame::default(), |ui| {
        draw_inspector_body(
            ui,
            nav,
            entities,
            selected,
            reflected_types,
            actions,
            euler_cache,
            rotation_display_mode,
            asset_catalog,
        )
    });

    if let Some(payload) = dropped {
        for &entity in selected {
            actions.push(EditorAction::AddComponent {
                entity,
                component: payload.0,
            });
        }
    }
}

/// Inspector body — everything below the rotation-mode toggle. Split out
/// so it can be wrapped by a `dnd_drop_zone` without disturbing the
/// early-return control flow the old function used.
#[allow(clippy::too_many_arguments)]
fn draw_inspector_body(
    ui: &mut egui::Ui,
    nav: &mut InspectorNav,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    rotation_display_mode: &mut RotationDisplayMode,
    asset_catalog: &[AssetCatalogEntry],
) {
    if selected.is_empty() {
        ui.weak("No entity selected");
        return;
    }

    if selected.len() > 1 {
        multi::draw_multi_entity_inspector(
            ui,
            entities,
            selected,
            reflected_types,
            actions,
            asset_catalog,
        );
        return;
    }

    // Single entity selected — show full inspector.
    let entity = selected[0];
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        ui.weak("Entity not found (despawned?)");
        return;
    };

    let entity_name = info
        .components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.values())
        .and_then(|fields| {
            fields.iter().find_map(|(name, val)| {
                if name == "value" {
                    if let ReflectValue::String(s) = val {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                None
            })
        });

    if let Some(name) = &entity_name {
        ui.label(format!(
            "{} {}  ({}:{})",
            icons::CUBE,
            name,
            entity.index(),
            entity.generation()
        ));
    } else {
        ui.label(format!(
            "{} Entity  index: {}  generation: {}",
            icons::CUBE,
            entity.index(),
            entity.generation()
        ));
    }
    ui.separator();

    // Configurations Rapier cannot honour, said where the author is
    // looking. A log line is not a warning if nobody reads the log.
    draw_physics_warnings(ui, entity, entities);

    // Editable name field (separate from component list).
    single::draw_name_editor(ui, entity, info, actions);

    // "Add Component" dropdown.
    let existing: HashSet<ComponentId> = info.components.iter().map(|c| c.component).collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !existing.contains(&t.component))
        .collect();

    if !available.is_empty() {
        ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
            crate::panels::add_component_menu::draw_categorized(ui, &available, |component| {
                actions.push(EditorAction::AddComponent { entity, component });
            });
        });
        ui.separator();
    }

    // Filter out Hidden components for display.
    let visible_components: Vec<&ComponentDisplayInfo> = info
        .components
        .iter()
        .filter(|c| c.visibility != InspectorVisibility::Hidden)
        .collect();

    if visible_components.is_empty() {
        ui.weak("(no components)");
        return;
    }

    let rotation_ctx = RotationContext {
        mode: *rotation_display_mode,
        self_global: info.global_rotation,
        parent_global: info.parent_global_rotation,
    };

    // Named, not automatic.
    //
    // An unsalted `ScrollArea` takes its id from the parent's widget
    // counter, and what precedes this one is not a fixed number of widgets:
    // the physics warnings are zero or more, and "Add Component" is there
    // only while something is left to add. Both change as the selection
    // moves, which renamed the scroll area — and with it its bar — while it
    // stayed exactly where it was. That is the Inspector half of #641.
    egui::ScrollArea::vertical()
        .id_salt("inspector_components")
        .show(ui, |ui| {
            for comp in &visible_components {
                let is_read_only = comp.visibility == InspectorVisibility::ReadOnly;
                // Keyed on the component, *not* on the entity holding it.
                //
                // With the entity's index in the id, every widget under the
                // header was renamed the moment the selection moved — and two
                // entities carrying the same components lay out identically,
                // so the ids changed while nothing moved on screen. That is
                // #641: egui reports it as a widget whose id is unstable,
                // because from the outside that is exactly what it looks like.
                //
                // Dropping the entity also fixes what it cost: whether
                // `Transform` was expanded now survives clicking to the next
                // entity, which is what any inspector is expected to do and
                // what this one did not.
                let id = ui.make_persistent_id(format!("comp_{:?}", comp.component));
                nav.rows.push(comp.component);
                let mut section = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    true,
                );
                // The keyboard's request, applied here because this is the
                // only place the section's persistent id exists.
                if let Some(open) = nav.take_toggle_for(comp.component) {
                    section.set_open(open);
                }
                let is_cursor = nav.is_cursor(comp.component);
                let scroll_here = is_cursor && nav.scroll_to_cursor;
                section
                    .show_header(ui, |ui| {
                        let title = format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name);
                        // Sensed for clicks so the header can carry a
                        // context menu. `ui.strong` returns a hover-only
                        // response, and `context_menu` on one never fires —
                        // it attaches and silently does nothing.
                        let label = |ui: &mut egui::Ui, text: egui::RichText| {
                            ui.add(egui::Label::new(text).sense(egui::Sense::click()))
                        };
                        let title = if is_cursor {
                            // Coloured rather than boxed: a section header is
                            // already a row of furniture, and another outline
                            // would be one more line to read past.
                            label(
                                ui,
                                egui::RichText::new(title)
                                    .strong()
                                    .color(ui.visuals().selection.bg_fill),
                            )
                        } else {
                            label(ui, egui::RichText::new(title).strong())
                        };
                        if scroll_here {
                            title.scroll_to_me(Some(egui::Align::Center));
                        }
                        // Removal is always available regardless of visibility:
                        // `ReadOnly` gates field edits, not component lifecycle.
                        if ui
                            .small_button(icons::X)
                            .on_hover_text("Remove component")
                            .clicked()
                        {
                            actions.push(EditorAction::RemoveComponent {
                                entity,
                                component: comp.component,
                            });
                        }
                        if comp.short_name == "PhysicsBody" && !is_read_only {
                            draw_calculate_mass(ui, entity, comp.component, entities, actions);
                        }
                        // Only on an instance, and only for a component
                        // that came from the prefab. Reverting is what
                        // makes an override safe to have — without it an
                        // accidental drag pins that field forever.
                        //
                        // On the header rather than as a button, because
                        // it is a rare action beside two frequent ones and
                        // a third button in the row is a third thing to
                        // read past every time.
                        if info.is_prefab_instance {
                            title.context_menu(|ui| {
                                if ui
                                    .button(format!(
                                        "{} Revert {} to Prefab",
                                        icons::ARROWS_CLOCKWISE,
                                        comp.short_name,
                                    ))
                                    .clicked()
                                {
                                    actions.push(EditorAction::RevertToPrefab {
                                        entity,
                                        component: Some(comp.component),
                                    });
                                    ui.close();
                                }
                            });
                        }
                    })
                    .body(|ui| {
                        if let Some(fields) = comp.fields.values() {
                            if fields.is_empty() {
                                ui.weak("(no fields)");
                            } else if is_read_only {
                                single::draw_readonly_fields(
                                    ui,
                                    comp.component,
                                    fields,
                                    comp.field_metas,
                                );
                            } else {
                                let edits = single::draw_reflected_fields(
                                    ui,
                                    entity,
                                    Some(comp.type_id),
                                    comp.component,
                                    fields,
                                    comp.field_metas,
                                    euler_cache,
                                    rotation_ctx,
                                    asset_catalog,
                                    entities,
                                );
                                // An entity's edits go to the world.
                                for (field, value) in edits {
                                    actions.push(EditorAction::SetField {
                                        entity,
                                        component: comp.component,
                                        field,
                                        value,
                                    });
                                }
                            }
                        } else if comp.fields.is_reflectable() {
                            // Reflectable, but its values were not read
                            // for this entity — the Inspector is showing
                            // something the gather did not count as
                            // selected. That is a bug in this editor, and
                            // it says so rather than blaming the
                            // component for a schema it does have.
                            ui.weak("(values not gathered — please report)");
                        } else {
                            ui.weak("(no reflection)");
                        }
                    });
            }
        });
}

/// Draws any physics warnings that apply to `entity`.
///
/// The **Calculate mass** button on a `PhysicsBody` header.
///
/// Writes `density × collider volume` into `mass`, once. It emits an
/// ordinary `SetField`, which means it is undoable like any other edit and
/// works unchanged against a remote project — the volume is computed from
/// the display snapshot, so no new message crosses the wire.
///
/// Disabled rather than hidden when there is nothing to measure: a button
/// that is not there reads as a feature that does not exist, while a greyed
/// one with a reason on hover says what to do next.
fn draw_calculate_mass(
    ui: &mut egui::Ui,
    entity: Entity,
    component: ComponentId,
    entities: &[EntityDisplayInfo],
    actions: &mut Vec<EditorAction>,
) {
    let mass = mass_from_colliders::mass_from_colliders(entity, entities);
    let button = ui.add_enabled(mass.is_some(), egui::Button::new("Calculate mass").small());
    let button = match mass {
        Some(mass) => button.on_hover_text(format!(
            "Set mass to {mass:.3} kg — this body's collider volume times its density. \
             Writes the value, so resizing a collider later will not change it.",
        )),
        None => button.on_disabled_hover_text(
            "Needs a Collider on this entity or on one of its children. Descendants \
             that carry their own PhysicsBody are separate bodies and do not count.",
        ),
    };
    if button.clicked()
        && let Some(mass) = mass
    {
        actions.push(EditorAction::SetField {
            entity,
            component,
            field: "mass".to_owned(),
            value: kooch_ecs::reflect::ReflectValue::F32(mass),
        });
    }
}

/// Amber rather than red: the scene still runs, and the configuration is
/// legal — it just does not do what the author probably expects. The same
/// colour the Transform readout already uses for shear.
fn draw_physics_warnings(ui: &mut egui::Ui, entity: Entity, entities: &[EntityDisplayInfo]) {
    let warnings = physics_warnings::warnings_for(entity, entities);
    if warnings.is_empty() {
        return;
    }
    for warning in warnings {
        let message = warning.message();
        // The full explanation on hover; one line in the panel, so a
        // second warning is still visible without scrolling.
        ui.horizontal(|ui| {
            ui.colored_label(egui::Color32::from_rgb(240, 180, 40), "\u{26a0}");
            ui.add(
                egui::Label::new(
                    egui::RichText::new(warning.summary())
                        .color(egui::Color32::from_rgb(240, 180, 40)),
                )
                .truncate(),
            )
            .on_hover_text(message);
        });
    }
    ui.separator();
}
