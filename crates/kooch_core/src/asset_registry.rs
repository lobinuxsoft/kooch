//! Asset types register themselves, next to their own definition.
//!
//! # Why not a list
//!
//! An asset type needs two things installed before anything can load it:
//! a loader on the [`AssetServer`], and the `Assets<T>` that loader fills
//! (`load_by_guid` requires the storage to exist rather than creating
//! it). Both used to be written out by whoever assembled the app — which
//! meant the facade and the editor each kept their own copy of the same
//! list, and a type was live in one process and missing in the other.
//!
//! That is how `.inputmap` shipped: loader registered in two places,
//! storage in neither, and every load failed with `Assets<ActionMap>
//! resource missing` once per frame. It is the same shape as every entry
//! in `docs/CAPABILITIES.md` — something built, and connected by whoever
//! remembered.
//!
//! So the declaration lives beside the type:
//!
//! ```ignore
//! kooch_core::register_asset!(ActionMap, InputMapLoader);
//! ```
//!
//! [`inventory`] collects those at link time. Whatever binary links the
//! crate gets the type; nothing central lists it, so nothing central can
//! forget it. Adding an asset is one line in the crate that owns it.
//!
//! # What this does not cover
//!
//! Types from a **dynamically loaded** project plugin. `inventory`
//! collects what is linked into the binary, and a `.so` opened at runtime
//! is not. Those still register explicitly — the plugin ABI is a separate
//! path with its own versioning, and a project asset type would go
//! through it.

use crate::asset_loader::AssetServer;
use crate::resource::Resources;

/// One asset type's installation, collected at link time.
///
/// Built by [`register_asset!`](crate::register_asset), never by hand:
/// the macro is what guarantees the two function pointers agree about
/// which type they are for.
pub struct AssetTypeRegistration {
    /// `std::any::type_name::<T>()`, for diagnostics. A function rather
    /// than the string itself because `type_name` is not a `const fn`,
    /// and `inventory::submit!` builds its value in a const context.
    pub type_name: fn() -> &'static str,
    /// Puts the loader on the server, before the first scan.
    pub register_loader: fn(&mut AssetServer),
    /// Inserts the `Assets<T>` that loader fills.
    pub install_storage: fn(&mut Resources),
}

inventory::collect!(AssetTypeRegistration);

/// Every asset type linked into this binary.
pub fn registered_asset_types() -> impl Iterator<Item = &'static AssetTypeRegistration> {
    inventory::iter::<AssetTypeRegistration>()
}

/// Declares an asset type: its loader and its storage, together.
///
/// Put it next to the type, in the crate that owns it. Any binary that
/// links that crate can then load the asset with nothing added anywhere
/// else.
///
/// ```ignore
/// pub struct ActionMap { /* … */ }
/// pub struct InputMapLoader;
/// kooch_core::register_asset!(ActionMap, InputMapLoader);
/// ```
///
/// The loader expression is evaluated once per install, so it must be
/// something constructible from nothing — which every loader in the
/// engine is.
#[macro_export]
macro_rules! register_asset {
    ($ty:ty, $loader:expr) => {
        $crate::inventory::submit! {
            $crate::asset_registry::AssetTypeRegistration {
                type_name: || std::any::type_name::<$ty>(),
                register_loader: |server| {
                    server.register_loader::<$ty, _>($loader);
                },
                install_storage: |resources| {
                    resources.insert($crate::assets::Assets::<$ty>::new());
                },
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry has to be non-empty in a binary that links any crate
    /// declaring an asset — an empty inventory reads as "no asset types"
    /// and would make every check below vacuously pass.
    ///
    /// `kooch_core` declares none of its own, so this asserts the shape
    /// rather than a count: names are unique, and nothing is blank.
    #[test]
    fn every_registration_names_a_distinct_type() {
        let mut seen: Vec<&str> = Vec::new();
        for registration in registered_asset_types() {
            let name = (registration.type_name)();
            assert!(!name.is_empty(), "an asset registration has no type name");
            assert!(
                !seen.contains(&name),
                "{name} is registered twice; its loader would be installed \
                 twice and the second would win silently",
            );
            seen.push(name);
        }
    }
}
