//! A prefab's components, resolved from every place a type can be known — the remote project's
//! schema included.

use super::*;
use kooch_ecs::scene::{ComponentDescription, EntityDescription};
use kooch_remote::protocol::{ComponentSchema, FieldSchema};

const PLAYER_INPUT: &str = "roll_a_ball::registrations::components::input::PlayerInput";

/// A prefab entity holding one project component, saved before its `look` field existed.
fn player() -> EntityDescription {
    EntityDescription {
        name: "Player".to_owned(),
        components: vec![ComponentDescription {
            type_name: PLAYER_INPUT.to_owned(),
            fields: vec![(
                "movement".to_owned(),
                ReflectValue::AssetRef {
                    guid: None,
                    asset_type: "kooch_input::actions::action::Action".to_owned(),
                },
            )],
        }],
        parent_index: None,
        parent: None,
    }
}

/// What the project published over the wire for that component.
fn schema() -> Vec<ComponentSchema> {
    let reference = |name: &str| FieldSchema {
        name: name.to_owned(),
        type_name: "Option<Guid>".to_owned(),
        choices: Vec::new(),
        asset_type: "kooch_input::actions::action::Action".to_owned(),
        doc: String::new(),
    };
    vec![ComponentSchema {
        type_name: PLAYER_INPUT.to_owned(),
        fields: Some(vec![reference("movement"), reference("look")]),
        category: None,
    }]
}

/// 🔴 Over a remote session a project's component is known only from the wire: never compiled into
/// the editor, never loaded as a plugin. Asking the registry and the plugins alone showed every one
/// of the player's own components in its prefab as unknown, and none of them editable.
#[test]
fn a_remote_component_resolves() {
    let views = sorted_visible(&player(), None, None, None, Some(&schema()));
    assert!(
        views[0].resolved.is_some(),
        "the project's own component stayed unknown"
    );
}

/// Without the schema it is genuinely unknown, and says so rather than guessing.
#[test]
fn an_unknown_component_stays_unknown() {
    let views = sorted_visible(&player(), None, None, None, None);
    assert!(views[0].resolved.is_none());
}

/// A reference added after the prefab was saved has nothing in the document to click — it is shown
/// empty, ready to assign.
#[test]
fn a_new_reference_is_offered() {
    let views = sorted_visible(&player(), None, None, None, Some(&schema()));
    let names: Vec<&str> = views[0]
        .fields
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, vec!["movement", "look"]);
}
