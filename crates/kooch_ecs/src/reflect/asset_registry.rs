//! Reflected asset types, collected at link time (#744).
//!
//! Lets the editor read and write the fields of an asset it has never
//! heard of, given only the `type_name` the asset database recorded.
//!
//! # Why this is a second registry
//!
//! [`kooch_core::asset_registry`] already collects asset types — their
//! loader and their storage. It cannot collect this: `Reflect` lives
//! here, in `kooch_ecs`, which is **above** `kooch_core`, so the lower
//! crate has no name for the trait. Widening the existing registration
//! would mean inverting the layering.
//!
//! Two registries, one macro. [`register_reflected_asset!`] submits to
//! both so an asset type cannot end up in one and not the other — which
//! is precisely the failure `.inputmap` shipped with, and the reason
//! `kooch_core`'s registry exists at all.
//!
//! # What it is for
//!
//! Before this, the editor's asset Inspector was a `match` on
//! `type_name`: `Material`, `MeshletMesh`, `Image` and prefabs each had a
//! hand-written gather function, a variant and a renderer, and any other
//! type displayed as "no import settings for X". A new asset type cost
//! three edits inside the editor, and the day nobody made them the asset
//! existed and could not be edited.
//!
//! An asset registered here is editable with no editor changes at all,
//! now and for every type after it.

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::assets::{Asset, Handle};
use kooch_core::resource::Resources;

use crate::reflect::{FieldMeta, Reflect, ReflectValue};

/// One reflected asset type's editor bridge, collected at link time.
pub struct ReflectedAssetRegistration {
    /// `std::any::type_name::<T>()` — matched against what the asset
    /// database recorded for a file.
    pub type_name: fn() -> &'static str,
    /// Field layout, for labels, widgets and tooltips.
    pub field_metas: fn() -> &'static [FieldMeta],
    /// Current values, loading the asset on demand.
    ///
    /// Takes `&mut Resources` because resolving a guid means going
    /// through the `AssetServer`, which loads if it has to. A read that
    /// cannot load is a read that shows nothing for an asset the user
    /// just clicked.
    pub read: fn(&mut Resources, Guid) -> Option<Vec<(String, ReflectValue)>>,
    /// Writes one field back. `false` when the asset is not loaded or
    /// the field does not exist or rejected the value.
    pub write: fn(&mut Resources, Guid, &str, ReflectValue) -> bool,
}

kooch_core::inventory::collect!(ReflectedAssetRegistration);

/// Every reflected asset type linked into this binary.
pub fn reflected_asset_types() -> impl Iterator<Item = &'static ReflectedAssetRegistration> {
    kooch_core::inventory::iter::<ReflectedAssetRegistration>()
}

/// The registration for `type_name`, if this binary has one.
pub fn reflected_asset(type_name: &str) -> Option<&'static ReflectedAssetRegistration> {
    reflected_asset_types().find(|r| (r.type_name)() == type_name)
}

/// Resolves a guid to a handle, loading if needed.
///
/// Public because the macro's generated closures call it from whatever
/// crate owns the asset type.
#[doc(hidden)]
pub fn load_handle<T: Asset>(resources: &mut Resources, guid: Guid) -> Option<Handle<T>> {
    // `AssetServer` is taken out because loading needs `&mut Resources`
    // for the storage it fills, and it cannot be borrowed from the same
    // map at the same time.
    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<T>(guid, resources).ok();
    resources.insert(server);
    handle
}

/// Reads every field of a loaded asset. Used by the macro.
#[doc(hidden)]
pub fn read_reflected<T: Asset + Reflect>(
    resources: &mut Resources,
    guid: Guid,
) -> Option<Vec<(String, ReflectValue)>> {
    let handle = load_handle::<T>(resources, guid)?;
    let asset = resources.get::<Assets<T>>()?.get(handle)?;
    Some(
        asset
            .reflect_fields()
            .iter()
            .filter_map(|meta| {
                asset
                    .reflect_get(meta.name)
                    .map(|value| (meta.name.to_owned(), value))
            })
            .collect(),
    )
}

/// Writes one field of a loaded asset. Used by the macro.
#[doc(hidden)]
pub fn write_reflected<T: Asset + Reflect>(
    resources: &mut Resources,
    guid: Guid,
    field: &str,
    value: ReflectValue,
) -> bool {
    let Some(handle) = load_handle::<T>(resources, guid) else {
        return false;
    };
    let Some(assets) = resources.get_mut::<Assets<T>>() else {
        return false;
    };
    let Some(asset) = assets.get_mut(handle) else {
        return false;
    };
    asset.reflect_set(field, value).is_ok()
}

/// Declares an asset type that the editor can edit by reflection.
///
/// Does everything [`kooch_core::register_asset!`] does, and adds the
/// editor bridge. Use this instead of `register_asset!` for any asset
/// whose fields a person should be able to change.
///
/// ```ignore
/// #[derive(Reflect, Serialize, Deserialize)]
/// pub struct RenderSettings { /* … */ }
/// pub struct RenderSettingsLoader;
/// kooch_ecs::register_reflected_asset!(RenderSettings, RenderSettingsLoader);
/// ```
///
/// One macro rather than two calls, because two calls is a thing to
/// forget half of, and half of it is an asset that loads and cannot be
/// edited.
#[macro_export]
macro_rules! register_reflected_asset {
    ($ty:ty, $loader:expr) => {
        ::kooch_core::register_asset!($ty, $loader);
        ::kooch_core::inventory::submit! {
            $crate::reflect::asset_registry::ReflectedAssetRegistration {
                type_name: || std::any::type_name::<$ty>(),
                field_metas: || {
                    <$ty as $crate::reflect::Reflect>::reflect_default().reflect_fields()
                },
                read: |resources, guid| {
                    $crate::reflect::asset_registry::read_reflected::<$ty>(resources, guid)
                },
                write: |resources, guid, field, value| {
                    $crate::reflect::asset_registry::write_reflected::<$ty>(
                        resources, guid, field, value,
                    )
                },
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever is registered must be named distinctly, or a lookup by
    /// type name returns whichever the linker happened to order first.
    #[test]
    fn registrations_name_distinct_types() {
        let mut seen: Vec<&str> = Vec::new();
        for registration in reflected_asset_types() {
            let name = (registration.type_name)();
            assert!(!name.is_empty());
            assert!(!seen.contains(&name), "{name} is registered twice");
            seen.push(name);
        }
    }

    /// An unknown type resolves to nothing rather than to the first
    /// registration — the editor falls back to its "no settings" label,
    /// which is honest, instead of editing the wrong asset's fields.
    #[test]
    fn an_unregistered_type_resolves_to_nothing() {
        assert!(reflected_asset("not::a::real::Asset").is_none());
    }
}
