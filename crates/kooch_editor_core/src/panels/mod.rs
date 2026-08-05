pub(crate) mod add_component_menu;
pub(crate) mod archetypes;
pub(crate) mod asset_browser;
pub(crate) mod components;
pub(crate) mod console;
pub(crate) mod game;
pub(crate) mod input_map;
pub(crate) mod inspector;
pub(crate) mod performance;
pub(crate) mod view;
pub(crate) mod world;

/// Shared harness for the per-panel widget-id stability tests (#641).
#[cfg(test)]
pub(crate) mod id_stability_probe;
