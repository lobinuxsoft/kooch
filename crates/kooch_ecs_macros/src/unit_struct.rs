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
            fn inspector_visibility() -> ::kooch_ecs::reflect::InspectorVisibility {
                ::kooch_ecs::reflect::InspectorVisibility::#vis
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
        impl ::kooch_ecs::reflect::Reflect for #name {
            fn reflect_fields(&self) -> &'static [::kooch_ecs::reflect::FieldMeta] {
                static FIELDS: &[::kooch_ecs::reflect::FieldMeta] = &[];
                FIELDS
            }

            fn reflect_get(&self, _field: &str) -> Option<::kooch_ecs::reflect::ReflectValue> {
                None
            }

            fn reflect_set(
                &mut self,
                field: &str,
                _value: ::kooch_ecs::reflect::ReflectValue,
            ) -> Result<(), ::kooch_ecs::reflect::ReflectError> {
                Err(::kooch_ecs::reflect::ReflectError::FieldNotFound(field.into()))
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
