//! `Vec<T>` where `T` is a plain value: an ordered list of numbers, vectors or strings (#1452).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/// The field meta, the `reflect_get` arm and the `reflect_set` arm for one scalar-list field. The
/// work happens in `kooch_ecs::reflect::{scalar_list, scalar_list_from}`; this only names the type.
pub(crate) fn scalar_list(
    field_name: &syn::Ident,
    field_name_str: &str,
    set_pattern: &TokenStream2,
    element: &syn::Type,
    hidden: bool,
    field_doc: &str,
    field_group: &str,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let type_name = format!("Vec<{}>", quote!(#element));
    let meta = quote! {
        ::kooch_ecs::reflect::FieldMeta {
            name: #field_name_str,
            type_name: #type_name,
            kind: ::kooch_ecs::reflect::FieldKind::List,
            choices: &[],
            bits: &[],
            layers: false,
            layer: false,
            hidden: #hidden,
            range: None,
            shown_when: ::core::option::Option::None,
            asset_type: "",
            requires: "",
            doc: #field_doc,
            group: #field_group,
            // Empty: the items have no fields of their own, which is what makes them scalars.
            fields: &[],
        }
    };
    let get = quote! {
        #field_name_str => Some(::kooch_ecs::reflect::scalar_list(&self.#field_name)),
    };
    let set = quote! {
        #set_pattern => {
            self.#field_name = ::kooch_ecs::reflect::scalar_list_from(value, #field_name_str)?;
            Ok(())
        }
    };
    (meta, get, set)
}
