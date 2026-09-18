//! `Vec<Option<Guid>>` with `#[reflect(asset = ...)]`: an ordered list of asset references (#1201).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/// The field meta, the `reflect_get` arm and the `reflect_set` arm for one asset-list field.
///
/// 🔴 `set` also takes a single `AssetRef`, as a one-item list: that is what a field written before
/// it became a list holds on disk, and an alias alone would still refuse it.
pub(crate) fn asset_list(
    field_name: &syn::Ident,
    field_name_str: &str,
    set_pattern: &TokenStream2,
    asset_type: &str,
    field_doc: &str,
    field_group: &str,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let meta = quote! {
        ::kooch_ecs::reflect::FieldMeta {
            name: #field_name_str,
            type_name: "Vec<Option<kooch_core::Guid>>",
            kind: ::kooch_ecs::reflect::FieldKind::List,
            choices: &[],
            bits: &[],
            range: None,
            shown_when: ::core::option::Option::None,
            asset_type: #asset_type,
            requires: "",
            doc: #field_doc,
            group: #field_group,
            fields: &[],
        }
    };
    let get = quote! {
        #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::List {
            items: self
                .#field_name
                .iter()
                .map(|guid| ::kooch_ecs::reflect::ReflectValue::AssetRef {
                    guid: *guid,
                    asset_type: #asset_type.to_owned(),
                })
                .collect(),
            element: ::std::boxed::Box::new(::kooch_ecs::reflect::ReflectValue::AssetRef {
                guid: None,
                asset_type: #asset_type.to_owned(),
            }),
        }),
    };
    let set = quote! {
        #set_pattern => match value {
            ::kooch_ecs::reflect::ReflectValue::List { items, .. } => {
                let mut guids = ::std::vec::Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        ::kooch_ecs::reflect::ReflectValue::AssetRef { guid, .. } => guids.push(guid),
                        other => {
                            return Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                                field: #field_name_str.into(),
                                expected: ::kooch_ecs::reflect::FieldKind::AssetRef,
                                got: other.kind(),
                            });
                        }
                    }
                }
                self.#field_name = guids;
                Ok(())
            }
            ::kooch_ecs::reflect::ReflectValue::AssetRef { guid, .. } => {
                self.#field_name = guid.into_iter().map(Some).collect();
                Ok(())
            }
            other => Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                field: #field_name_str.into(),
                expected: ::kooch_ecs::reflect::FieldKind::List,
                got: other.kind(),
            }),
        },
    };
    (meta, get, set)
}
