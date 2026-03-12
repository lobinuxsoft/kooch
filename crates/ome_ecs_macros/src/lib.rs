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

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type, parse_macro_input};

/// Derives the `Reflect` trait for a named-field struct.
///
/// Generates `reflect_fields`, `reflect_get`, `reflect_set`, and
/// `reflect_default` based on the struct's fields. Each field type
/// must map to a known `FieldKind` / `ReflectValue` variant.
#[proc_macro_derive(Reflect)]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            Fields::Unit => {
                // Unit struct — no fields.
                return unit_struct_impl(name);
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

        let Some((kind_variant, type_name_str, needs_clone)) = type_mapping(ty) else {
            return syn::Error::new_spanned(
                ty,
                format!(
                    "Reflect derive: unsupported field type `{}`. \
                     Supported: f32, f64, u8..u64, i8..i64, bool, String, \
                     Vec2, Vec3, Vec4, Quat, Mat4.",
                    quote!(#ty),
                ),
            )
            .to_compile_error()
            .into();
        };

        let kind_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();
        let value_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();

        // FieldMeta entry.
        field_metas.push(quote! {
            ::ome_ecs::reflect::FieldMeta {
                name: #field_name_str,
                type_name: #type_name_str,
                kind: ::ome_ecs::reflect::FieldKind::#kind_ident,
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
        }
    };

    // Suppress "unused field_count" — it's used for the static array size hint.
    let _ = field_count;
    expanded.into()
}

/// Generates a Reflect impl for a unit struct (no fields).
fn unit_struct_impl(name: &syn::Ident) -> TokenStream {
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
        }
    };
    expanded.into()
}

/// Maps a Rust type to (FieldKind variant name, type_name string, needs_clone).
fn type_mapping(ty: &Type) -> Option<(&'static str, &'static str, bool)> {
    let ident = last_type_segment(ty)?;
    match ident.as_str() {
        "f32" => Some(("F32", "f32", false)),
        "f64" => Some(("F64", "f64", false)),
        "u8" => Some(("U8", "u8", false)),
        "u16" => Some(("U16", "u16", false)),
        "u32" => Some(("U32", "u32", false)),
        "u64" => Some(("U64", "u64", false)),
        "i8" => Some(("I8", "i8", false)),
        "i16" => Some(("I16", "i16", false)),
        "i32" => Some(("I32", "i32", false)),
        "i64" => Some(("I64", "i64", false)),
        "bool" => Some(("Bool", "bool", false)),
        "String" => Some(("String", "String", true)),
        "Vec2" => Some(("Vec2", "Vec2", false)),
        "Vec3" => Some(("Vec3", "Vec3", false)),
        "Vec4" => Some(("Vec4", "Vec4", false)),
        "Quat" => Some(("Quat", "Quat", false)),
        "Mat4" => Some(("Mat4", "Mat4", false)),
        _ => None,
    }
}

/// Extracts the last path segment identifier from a type.
/// e.g. `glam::Vec3` → "Vec3", `f32` → "f32".
fn last_type_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => {
            type_path.path.segments.last().map(|s| s.ident.to_string())
        }
        _ => None,
    }
}
