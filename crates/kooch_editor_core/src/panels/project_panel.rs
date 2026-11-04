//! A project's own panel (#1791): a `.kxml` under its `assets/editor/`, docked, its `bind:value`
//! on the entity selected. Edits go through `EditorAction`, so they undo and reach a remote game.

use kooch_ecs::Entity;
use kooch_ecs::component::ComponentId;
use kooch_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::state::EntityDisplayInfo;
use crate::ui_panel::UiPanel;

pub(crate) fn draw(
    ui: &mut egui::Ui,
    panel: Option<&mut UiPanel>,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    actions: &mut Vec<EditorAction>,
) {
    let Some(panel) = panel else {
        ui.weak("Loading the panel…");
        return;
    };
    panel.show(ui);
    let held = panel.pressed();
    let target = selected
        .first()
        .and_then(|entity| entities.iter().find(|info| info.entity == *entity));
    let Some(tree) = panel.tree.as_mut() else {
        return;
    };
    for bound in tree.take_bindings() {
        let found = target.and_then(|info| field(info, &bound.component, &bound.field));
        // Disabled while there is nothing to edit; enabled again unless the file disables it.
        let disabled = found.is_none() || tree.attribute(bound.node, "disabled") == Some("true");
        if tree.disabled(bound.node) != disabled {
            tree.set_disabled(bound.node, disabled);
        }
        let Some((component, value)) = found else {
            continue;
        };
        let entity = target.map(|info| info.entity).expect("found on the target");
        if bound.touched {
            if let Some(value) = kooch_ui::field_written(value, bound.value) {
                actions.push(EditorAction::SetField {
                    entity,
                    component,
                    field: bound.field.to_string(),
                    value,
                });
            }
        // The snapshot is a round trip behind a drag: the hand keeps the control until it lets go.
        } else if held != Some(bound.node)
            && let Some(number) = kooch_ui::field_number(value)
        {
            tree.set_value(bound.node, number);
        }
    }
}

/// `Type.field` on one entity, `Type` short or full.
fn field<'a>(
    info: &'a EntityDisplayInfo,
    component: &str,
    name: &str,
) -> Option<(ComponentId, &'a ReflectValue)> {
    let short = component.rsplit("::").next().unwrap_or(component);
    let found = info.components.iter().find(|c| c.short_name == short)?;
    let value = found
        .fields
        .values()?
        .iter()
        .find(|(own, _)| own == name)
        .map(|(_, value)| value)?;
    Some((found.component, value))
}
