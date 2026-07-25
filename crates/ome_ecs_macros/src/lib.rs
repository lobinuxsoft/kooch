//! Derive macros for `ome_ecs`.
//!
//! Provides `#[derive(Reflect)]` to auto-generate the [`Reflect`] trait
//! implementation for component structs.
//!
//! # Supported field types
//!
//! `f32`, `f64`, `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`,
//! `bool`, `String`, `Vec2`, `Vec3`, `Vec4`, `Quat`, `Mat4`.
//!
//! # Requirements
//!
//! The struct must implement [`Default`] (used for `reflect_default()`).
//!
//! # Example
//!
//! ```ignore
//! #[derive(Default, Reflect)]
//! struct Transform {
//!     pub position: Vec3,
//!     pub rotation: Quat,
//!     pub scale: Vec3,
//! }
//! ```

mod attrs;
mod type_mapping;
mod unit_struct;
mod util;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

use crate::attrs::{
    parse_category_attr, parse_field_asset_type, parse_field_choices, parse_field_shown_when,
    parse_field_skip,
    parse_inspector_attr,
};
use crate::type_mapping::type_mapping;
use crate::unit_struct::unit_struct_impl;

/// Derives the `Reflect` trait for a named-field struct.
///
/// Generates `reflect_fields`, `reflect_get`, `reflect_set`, and
/// `reflect_default` based on the struct's fields. Each field type
/// must map to a known `FieldKind` / `ReflectValue` variant.
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Parse #[reflect(inspector = "hidden"|"read_only"|"editable")] attribute.
    let inspector_visibility = match parse_inspector_attr(&input) {
        Ok(vis) => vis,
        Err(err) => return err,
    };

    // Parse #[reflect(category = "...")] attribute.
    let category = match parse_category_attr(&input) {
        Ok(cat) => cat,
        Err(err) => return err,
    };

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            Fields::Unit => {
                // Unit struct — no fields.
                return unit_struct_impl(name, inspector_visibility.as_ref(), category.as_deref());
            }
            Fields::Unnamed(_) => {
                return syn::Error::new_spanned(
                    name,
                    "Reflect derive does not support tuple structs",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(name, "Reflect derive only supports structs")
                .to_compile_error()
                .into();
        }
    };

    let mut field_metas = Vec::new();
    let mut get_arms = Vec::new();
    let mut set_arms = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let ty = &field.ty;

        // `#[reflect(skip)]` opts the field out of the inspector +
        // get/set paths entirely. Used for handle-style fields that
        // hold opaque keys (Option<DefaultKey>, etc.) which the
        // editor inspector has no representation for.
        let skip = match parse_field_skip(field) {
            Ok(skip) => skip,
            Err(e) => return e,
        };
        if skip {
            continue;
        }

        // `#[reflect(asset = "TypeName")]` annotates an
        // `Option<Guid>` field as a typed asset reference. The
        // inspector picks it up via FieldKind::AssetRef and renders
        // a dropdown filtered by `TypeName`.
        let asset_type = match parse_field_asset_type(field) {
            Ok(opt) => opt,
            Err(e) => return e,
        };
        if let Some(asset_type) = asset_type {
            field_metas.push(quote! {
                ::ome_ecs::reflect::FieldMeta {
                    name: #field_name_str,
                    type_name: "Option<ome_core::Guid>",
                    kind: ::ome_ecs::reflect::FieldKind::AssetRef,
                    choices: &[],
                    // An asset picker has no variant to depend on yet.
                    shown_when: ::core::option::Option::None,
                    asset_type: #asset_type,
                }
            });
            get_arms.push(quote! {
                #field_name_str => Some(::ome_ecs::reflect::ReflectValue::AssetRef {
                    guid: self.#field_name,
                    asset_type: #asset_type.to_owned(),
                }),
            });
            set_arms.push(quote! {
                #field_name_str => match value {
                    ::ome_ecs::reflect::ReflectValue::AssetRef { guid, .. } => {
                        self.#field_name = guid;
                        Ok(())
                    }
                    other => Err(::ome_ecs::reflect::ReflectError::TypeMismatch {
                        field: #field_name_str.into(),
                        expected: ::ome_ecs::reflect::FieldKind::AssetRef,
                        got: other.kind(),
                    }),
                },
            });
            continue;
        }

        let Some((kind_variant, type_name_str, needs_clone)) = type_mapping(ty) else {
            return syn::Error::new_spanned(
                ty,
                format!(
                    "Reflect derive: unsupported field type `{}`. \
                     Supported: f32, f64, u8..u64, i8..i64, bool, String, \
                     Vec2, Vec3, Vec4, Quat, Mat4. \
                     Use `#[reflect(skip)]` to opt out.",
                    quote!(#ty),
                ),
            )
            .to_compile_error()
            .into();
        };

        let kind_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();
        let value_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();

        let choices_expr = match parse_field_choices(field) {
            Ok(Some(expr)) => quote! { #expr },
            Ok(None) => quote! { &[] },
            Err(e) => return e,
        };
        let shown_when_expr = match parse_field_shown_when(field) {
            Ok(Some(expr)) => quote! { ::core::option::Option::Some(&#expr) },
            Ok(None) => quote! { ::core::option::Option::None },
            Err(e) => return e,
        };

        // FieldMeta entry.
        field_metas.push(quote! {
            ::ome_ecs::reflect::FieldMeta {
                name: #field_name_str,
                type_name: #type_name_str,
                kind: ::ome_ecs::reflect::FieldKind::#kind_ident,
                choices: #choices_expr,
                shown_when: #shown_when_expr,
                asset_type: "",
            }
        });

        // reflect_get arm.
        if needs_clone {
            get_arms.push(quote! {
                #field_name_str => Some(::ome_ecs::reflect::ReflectValue::#value_ident(self.#field_name.clone())),
            });
        } else {
            get_arms.push(quote! {
                #field_name_str => Some(::ome_ecs::reflect::ReflectValue::#value_ident(self.#field_name)),
            });
        }

        // reflect_set arm.
        set_arms.push(quote! {
            #field_name_str => match value {
                ::ome_ecs::reflect::ReflectValue::#value_ident(v) => {
                    self.#field_name = v;
                    Ok(())
                }
                other => Err(::ome_ecs::reflect::ReflectError::TypeMismatch {
                    field: #field_name_str.into(),
                    expected: ::ome_ecs::reflect::FieldKind::#kind_ident,
                    got: other.kind(),
                }),
            },
        });
    }

    let field_count = field_metas.len();

    let visibility_method = inspector_visibility.map(|vis| {
        quote! {
            fn inspector_visibility() -> ::ome_ecs::reflect::InspectorVisibility {
                ::ome_ecs::reflect::InspectorVisibility::#vis
            }
        }
    });

    let category_method = category.as_deref().map(|cat| {
        quote! {
            fn category() -> Option<&'static str> {
                Some(#cat)
            }
        }
    });

    let expanded = quote! {
        impl ::ome_ecs::reflect::Reflect for #name {
            fn reflect_fields(&self) -> &'static [::ome_ecs::reflect::FieldMeta] {
                static FIELDS: &[::ome_ecs::reflect::FieldMeta] = &[
                    #(#field_metas),*
                ];
                FIELDS
            }

            fn reflect_get(&self, field: &str) -> Option<::ome_ecs::reflect::ReflectValue> {
                match field {
                    #(#get_arms)*
                    _ => None,
                }
            }

            fn reflect_set(
                &mut self,
                field: &str,
                value: ::ome_ecs::reflect::ReflectValue,
            ) -> Result<(), ::ome_ecs::reflect::ReflectError> {
                match field {
                    #(#set_arms)*
                    _ => Err(::ome_ecs::reflect::ReflectError::FieldNotFound(field.into())),
                }
            }

            fn reflect_default() -> Self {
                Self::default()
            }

            #visibility_method
            #category_method
        }
    };

    // Suppress "unused field_count" — it's used for the static array size hint.
    let _ = field_count;
    expanded.into()
}
