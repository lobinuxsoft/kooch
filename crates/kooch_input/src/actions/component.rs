//! [`InputMapSource`] — which `.inputmap` a scene plays under.
//!
//! # Why a component and not a plugin argument
//!
//! `ActionsPlugin::from_asset(guid)` works and asks the wrong thing of
//! whoever writes it: a guid pasted into `main.rs`, found by opening a
//! `.meta` in a text editor. Nothing about that is authoring.
//!
//! As a component it is a field in the Inspector, filled from the same
//! typed asset picker that fills a `MeshRenderer`'s mesh — the picker
//! lists every `.inputmap` the database knows and refuses anything else.
//! It travels in the scene, so two scenes can bind differently without a
//! rebuild, and changing it in the Inspector swaps the bindings live.
//!
//! # It is read once, then left alone
//!
//! The system compares what is active against what the component asks
//! for and loads only on a mismatch. A per-frame load would re-parse a
//! file every frame to reach the same answer, and — worse — would throw
//! away the map the panel is editing.

use kooch_core::Guid;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Points at the `.inputmap` whose bindings this world uses.
///
/// One is enough: the active map is a property of the session, not of an
/// entity. Put it on whatever entity represents the game's setup — the
/// same place a scene's other one-off configuration lives.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(category = "Input")]
pub struct InputMapSource {
    /// The asset. `None` leaves whatever is already active, which is what
    /// makes an unconfigured component harmless rather than a world with
    /// no bindings at all.
    ///
    /// The string has to equal `std::any::type_name::<ActionMap>()` — the
    /// picker filters on it, and the re-exported path would match nothing.
    #[reflect(asset = "kooch_input::actions::action::ActionMap")]
    pub map: Option<Guid>,
}

impl Component for InputMapSource {}

#[cfg(test)]
mod tests {
    use super::*;
    use kooch_ecs::reflect::{FieldKind, Reflect as _};

    /// The picker filters on this exact string. A re-exported path
    /// compiles, lists nothing, and looks like an empty asset database.
    #[test]
    fn the_asset_filter_matches_the_type_it_points_at() {
        let source = InputMapSource::default();
        let field = source
            .reflect_fields()
            .iter()
            .find(|f| f.name == "map")
            .expect("map field reflected");

        assert_eq!(field.kind, FieldKind::AssetRef);
        assert_eq!(
            field.asset_type,
            std::any::type_name::<crate::actions::ActionMap>(),
            "the filter must be the asset's own type name, or the picker \
             lists nothing and reads as an empty database",
        );
    }

    /// Unset is the default, and it has to mean "leave what is active"
    /// rather than "clear the bindings" — a component added and not yet
    /// filled in must not take the controls away.
    #[test]
    fn an_unset_source_points_at_nothing() {
        assert_eq!(InputMapSource::default().map, None);
    }
}
