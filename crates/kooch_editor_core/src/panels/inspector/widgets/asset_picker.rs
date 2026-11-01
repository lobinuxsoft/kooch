//! Typed asset-reference picker for `ReflectValue::AssetRef` fields.

use kooch_core::Guid;
use kooch_ecs::reflect::ReflectValue;

use super::asset::AssetCatalogEntry;

/// The editor's [`asset_picker`](crate::widgets::asset_picker) on a reflected field. Returns
/// `Some(new_value)` when the user picks a different asset (or clears the field), otherwise `None`.
pub(crate) fn draw_asset_picker(
    ui: &mut egui::Ui,
    current: Option<Guid>,
    asset_type: &str,
    catalog: &[AssetCatalogEntry],
) -> Option<ReflectValue> {
    crate::widgets::asset_picker(
        ui,
        ("asset_picker", asset_type),
        current,
        asset_type,
        catalog,
        None,
    )
    .map(|guid| ReflectValue::AssetRef {
        guid,
        asset_type: asset_type.to_owned(),
    })
}
