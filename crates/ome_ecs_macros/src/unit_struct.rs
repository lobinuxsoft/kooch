//! `Reflect` derive expansion for unit structs (no fields).

use proc_macro::TokenStream;
use quote::quote;

/// Generates a Reflect impl for a unit struct (no fields).
pub(crate) fn unit_struct_impl(
    name: &syn::Ident,
    inspector_visibility: Option<&proc_macro2::Ident>,
    category: Option<&str>,
) -> TokenStream {
    let visibility_method = inspector_visibility.map(|vis| {
        quote! {
            fn inspector_visibility() -> ::ome_ecs::reflect::InspectorVisibility {
                ::ome_ecs::reflect::InspectorVisibility::#vis
            }
        }
    });

    let category_method = category.map(|cat| {
        quote! {
            fn category() -> Option<&'static str> {
                Some(#cat)
            }
        }
    });

    let expanded = quote! {
        impl ::ome_ecs::reflect::Reflect for #name {
            fn reflect_fields(&self) -> &'static [::ome_ecs::reflect::FieldMeta] {
                static FIELDS: &[::ome_ecs::reflect::FieldMeta] = &[];
                FIELDS
            }

            fn reflect_get(&self, _field: &str) -> Option<::ome_ecs::reflect::ReflectValue> {
                None
            }

            fn reflect_set(
                &mut self,
                field: &str,
                _value: ::ome_ecs::reflect::ReflectValue,
            ) -> Result<(), ::ome_ecs::reflect::ReflectError> {
                Err(::ome_ecs::reflect::ReflectError::FieldNotFound(field.into()))
            }

            fn reflect_default() -> Self {
                Self::default()
            }

            #visibility_method
            #category_method
        }
    };
    expanded.into()
}
