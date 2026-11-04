//! A project's own panel (#1791): a `.kxml` under its `assets/editor/`, docked, its `bind:value`
//! on the entity selected, its `on:click` on the project's `src/tools/`. Edits go through
//! `EditorAction`, so they undo and reach a remote game.

use kooch_core::dynamic::{EditorCall, EditorHandlers, EditorRequest, SelectedEntity, pack_entity};
use kooch_ecs::Entity;
use kooch_ecs::component::ComponentId;
use kooch_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::state::EntityDisplayInfo;
use crate::ui_panel::UiPanel;

/// The handle an action sees for `entity`.
fn packed(entity: Entity) -> u64 {
    pack_entity(entity.index(), entity.generation())
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    panel: Option<&mut UiPanel>,
    handlers: Option<&EditorHandlers>,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    actions: &mut Vec<EditorAction>,
) {
    let Some(panel) = panel else {
        ui.weak("Loading the panel…");
        return;
    };
    let (_, gestures) = panel.show(ui);
    let held = panel.pressed();
    let target = selected
        .first()
        .and_then(|entity| entities.iter().find(|info| info.entity == *entity));
    let Some(tree) = panel.tree.as_mut() else {
        return;
    };
    let clicked = gestures.clicked.filter(|node| !tree.disabled(*node));
    if let Some(name) = clicked.and_then(|node| tree.click(node)) {
        let shown: Vec<&EntityDisplayInfo> = selected
            .iter()
            .filter_map(|entity| entities.iter().find(|info| info.entity == *entity))
            .collect();
        run(name, handlers, &shown, actions);
    }
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

/// Runs the project's action `name` and turns what it asked for into editor actions.
fn run(
    name: &str,
    handlers: Option<&EditorHandlers>,
    shown: &[&EntityDisplayInfo],
    actions: &mut Vec<EditorAction>,
) {
    let Some(handler) = handlers.and_then(|handlers| handlers.get(name)) else {
        tracing::warn!(
            name,
            "no editor action by this name in the project's src/tools/"
        );
        return;
    };
    let mut call = EditorCall::new(shown.iter().map(|info| selected_entity(info)).collect());
    // Project code inside the editor's frame: its panic stays its own.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&mut call)));
    if ran.is_err() {
        tracing::error!(name, "the editor action panicked");
        return;
    }
    for request in call.take_requests() {
        match request {
            EditorRequest::SetNumber {
                entity,
                field: path,
                value,
            } => {
                let written = shown
                    .iter()
                    .find(|info| packed(info.entity) == entity)
                    .and_then(|info| field_set(info, &path, value));
                match written {
                    Some(action) => actions.push(action),
                    None => tracing::warn!(name, path, "an editor action set no such field"),
                }
            }
            EditorRequest::WriteAsset { path, text } => {
                actions.push(EditorAction::WriteAsset { path, text })
            }
        }
    }
}

/// `info` as an action sees it: every number and bool, keyed `Type.field`.
fn selected_entity(info: &EntityDisplayInfo) -> SelectedEntity {
    let numbers = info.components.iter().flat_map(|component| {
        let fields = component.fields.values().into_iter().flatten();
        fields.filter_map(|(name, value)| {
            let number = kooch_ui::field_number(value)?;
            Some((format!("{}.{name}", component.short_name), number))
        })
    });
    SelectedEntity {
        entity: packed(info.entity),
        numbers: numbers.collect(),
    }
}

/// The edit that sets `Type.field` on `info` to `value`.
fn field_set(info: &EntityDisplayInfo, path: &str, value: f32) -> Option<EditorAction> {
    let (component, name) = path.rsplit_once('.')?;
    let (component, current) = field(info, component, name)?;
    Some(EditorAction::SetField {
        entity: info.entity,
        component,
        field: name.to_owned(),
        value: kooch_ui::field_written(current, value)?,
    })
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
