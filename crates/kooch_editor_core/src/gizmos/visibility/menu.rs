//! The Gizmos dropdown: a master switch plus a checkbox per component,
//! grouped by category.

use kooch_core::resource::Resources;

use super::{GizmoGroup, GizmoVisibility, group_visualizers};

/// Draws the Gizmos dropdown: a master switch, then a checkbox per
/// category, then the components inside each.
///
/// Three levels because "hide all physics" and "hide only the colliders"
/// are both things you want, and a flat list is the part of Unity's own
/// panel people complain about.
///
/// `groups` comes from [`group_visualizers`], so the menu lists whatever is
/// registered — a visualizer added later appears here with no change.
pub(crate) fn draw_gizmo_menu(
    ui: &mut egui::Ui,
    visibility: &mut GizmoVisibility,
    groups: &[GizmoGroup],
) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut visibility.enabled, "Gizmos")
            .on_hover_text("Master switch — leaves the per-group choices below untouched");
        if ui
            .add_enabled(visibility.has_exceptions(), egui::Button::new("Reset"))
            .on_hover_text("Show everything again")
            .clicked()
        {
            visibility.show_all();
        }
    });
    ui.separator();

    // Greyed out rather than hidden while the master switch is off: the
    // per-group state still exists and is about to matter again, and
    // hiding the rows would suggest it had been lost.
    ui.add_enabled_ui(visibility.enabled, |ui| {
        if groups.is_empty() {
            ui.label("No gizmos registered");
            return;
        }
        for group in groups {
            match &group.category {
                Some(category) => {
                    let mut on = visibility.category_visible(category);
                    if ui.checkbox(&mut on, category).changed() {
                        visibility.set_category(category, on);
                    }
                    // A component inside a hidden category cannot draw, so
                    // its own switch is inert until the category is back.
                    ui.add_enabled_ui(on, |ui| {
                        component_rows(ui, visibility, &group.components);
                    });
                }
                None => {
                    ui.label("Uncategorised");
                    component_rows(ui, visibility, &group.components);
                }
            }
            ui.separator();
        }
    });
}

/// The indented per-component checkboxes under one category.
fn component_rows(
    ui: &mut egui::Ui,
    visibility: &mut GizmoVisibility,
    components: &[(String, String)],
) {
    for (type_name, short) in components {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let mut on = visibility.component_visible(type_name);
            if ui
                .checkbox(&mut on, short)
                .on_hover_text(type_name)
                .changed()
            {
                visibility.set_component(type_name, on);
            }
        });
    }
}

/// Groups the visualizers registered in `resources`, resolving each
/// component's reflected category.
pub(crate) fn groups_from_resources(resources: &Resources) -> Vec<GizmoGroup> {
    let Some(registry) = resources.get::<kooch_gizmos::VisualizerRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<kooch_ecs::component::ComponentRegistry>();
    group_visualizers(registry.registered_types().filter_map(|type_id| {
        let components = components.as_ref()?;
        // A visualizer for a component the registry has never seen has no
        // name to persist and no category to group under; skipping it is
        // better than inventing either.
        let name = components.component_name(&type_id)?;
        Some((
            type_id,
            name.to_owned(),
            components.reflect_category(&type_id).map(str::to_owned),
        ))
    }))
}
