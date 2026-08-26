//! Per-`ReflectValue` editor widgets and choice helpers.
//!
//! [`draw_value_widget`] is the giant `match` over `ReflectValue`
//! variants; [`choices::draw_choice_dropdown`] covers integer fields with
//! `FieldChoice` hints (used internally); [`draw_readonly_value`] is the
//! non-interactive counterpart shared by both single- and multi-entity
//! rendering paths.

mod asset;
mod asset_picker;
mod choices;
mod entity_picker;
mod value_widget;

pub(crate) use self::asset::{AssetCatalogEntry, AssetSource};
pub(crate) use self::asset_picker::draw_asset_picker;
pub(super) use self::choices::{bits_for, choices_for, draw_readonly_value, requires_for};
pub(super) use self::value_widget::{FieldContext, draw_value_widget};
