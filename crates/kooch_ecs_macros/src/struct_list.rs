//! `Vec<T>` where `T` is a reflected struct: an ordered list of structs (#1209).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/// The field meta, the `reflect_get` arm and the `reflect_set` arm for one struct-list field. The
/// work happens in `kooch_ecs::reflect::{list_value, list_from}`; this only names the types.
pub(crate) fn struct_list(
    field_name: &syn::Ident,
    field_name_str: &str,
    set_pattern: &TokenStream2,
    element: &syn::Type,
    bare: Option<&str>,
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
            range: None,
            shown_when: ::core::option::Option::None,
            asset_type: "",
            requires: "",
            doc: #field_doc,
            group: #field_group,
            fields: <#element>::REFLECT_FIELDS,
        }
    };
    let get = quote! {
        #field_name_str => Some(::kooch_ecs::reflect::list_value(&self.#field_name)),
    };
    let bare = match bare {
        Some(bare) => quote! { ::core::option::Option::Some(#bare) },
        None => quote! { ::core::option::Option::None },
    };
    let set = quote! {
        #set_pattern => {
            self.#field_name = ::kooch_ecs::reflect::list_from(value, #bare, #field_name_str)?;
            Ok(())
        }
    };
    (meta, get, set)
}
