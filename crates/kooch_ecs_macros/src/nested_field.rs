//! A field that is itself a reflected struct (#1452).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/// The field meta, the `reflect_get` arm and the `reflect_set` arm for one nested-struct field.
/// The work happens in `kooch_ecs::reflect::{struct_value, struct_from}`; this only names the type.
pub(crate) fn nested_field(
    field_name: &syn::Ident,
    field_name_str: &str,
    set_pattern: &TokenStream2,
    ty: &syn::Type,
    hidden: bool,
    field_doc: &str,
    field_group: &str,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let type_name = format!("{}", quote!(#ty));
    let meta = quote! {
        ::kooch_ecs::reflect::FieldMeta {
            name: #field_name_str,
            type_name: #type_name,
            kind: ::kooch_ecs::reflect::FieldKind::Nested,
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
            fields: <#ty>::REFLECT_FIELDS,
        }
    };
    let get = quote! {
        #field_name_str => Some(::kooch_ecs::reflect::struct_value(&self.#field_name)),
    };
    let set = quote! {
        #set_pattern => {
            self.#field_name = ::kooch_ecs::reflect::struct_from(value, #field_name_str)?;
            Ok(())
        }
    };
    (meta, get, set)
}
