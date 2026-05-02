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
use syn::{Data, DeriveInput, Fields, Lit, Meta, MetaNameValue, Type, parse_macro_input};

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

        // FieldMeta entry.
        field_metas.push(quote! {
            ::ome_ecs::reflect::FieldMeta {
                name: #field_name_str,
                type_name: #type_name_str,
                kind: ::ome_ecs::reflect::FieldKind::#kind_ident,
                choices: #choices_expr,
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

/// Generates a Reflect impl for a unit struct (no fields).
fn unit_struct_impl(
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

/// Parses `#[reflect(inspector = "hidden"|"read_only"|"editable")]` from struct attributes.
///
/// Returns `Ok(Some(ident))` with the variant name (`Hidden`, `ReadOnly`, `Editable`)
/// if the attribute is present, `Ok(None)` to use the trait default, or
/// `Err(compile_error)` for invalid values.
fn parse_inspector_attr(input: &DeriveInput) -> Result<Option<proc_macro2::Ident>, TokenStream> {
    for attr in &input.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr.parse_args_with(
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        ) {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue {
                path,
                value: syn::Expr::Lit(expr_lit),
                ..
            }) = meta
                && path.is_ident("inspector")
                && let Lit::Str(lit_str) = &expr_lit.lit
            {
                let val = lit_str.value();
                let variant_name = match val.as_str() {
                    "hidden" => "Hidden",
                    "read_only" => "ReadOnly",
                    "editable" => "Editable",
                    _ => {
                        return Err(syn::Error::new_spanned(
                            lit_str,
                            "expected \"hidden\", \"read_only\", or \"editable\"",
                        )
                        .to_compile_error()
                        .into());
                    }
                };
                return Ok(Some(proc_macro2::Ident::new(
                    variant_name,
                    proc_macro2::Span::call_site(),
                )));
            }
        }
    }
    Ok(None)
}

/// Parses `#[reflect(skip)]` on a field. Returns `true` when present,
/// `false` otherwise. Skipped fields are omitted from the FieldMeta
/// list and from the get/set match arms — opaque handle fields use
/// this to stay out of the editor inspector.
fn parse_field_skip(field: &syn::Field) -> Result<bool, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr.parse_args_with(
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        ) {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::Path(path) = meta
                && path.is_ident("skip")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Parses `#[reflect(choices = PATH)]` from a field's attributes.
///
/// Returns `Ok(Some(expr))` with the path/identifier pointing to a
/// `&'static [::ome_ecs::reflect::FieldChoice]` constant when present,
/// `Ok(None)` when absent, or `Err(compile_error)` on a parse failure.
fn parse_field_choices(
    field: &syn::Field,
) -> Result<Option<syn::Expr>, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr.parse_args_with(
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        ) {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta
                && path.is_ident("choices")
            {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

/// Parses `#[reflect(category = "...")]` from struct attributes.
///
/// Returns `Ok(Some(string))` with the category name if the attribute is
/// present, `Ok(None)` to use the trait default, or `Err(compile_error)`
/// if the value is not a string literal.
fn parse_category_attr(input: &DeriveInput) -> Result<Option<String>, TokenStream> {
    for attr in &input.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr.parse_args_with(
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        ) {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue {
                path,
                value: syn::Expr::Lit(expr_lit),
                ..
            }) = meta
                && path.is_ident("category")
                && let Lit::Str(lit_str) = &expr_lit.lit
            {
                return Ok(Some(lit_str.value()));
            }
        }
    }
    Ok(None)
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
