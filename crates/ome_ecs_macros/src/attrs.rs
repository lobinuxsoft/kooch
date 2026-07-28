//! `#[reflect(...)]` attribute parsing for both struct-level and field-level
//! annotations consumed by the `Reflect` derive macro.

use proc_macro::TokenStream;
use syn::{DeriveInput, Lit, Meta, MetaNameValue};

/// Parses `#[reflect(inspector = "hidden"|"read_only"|"editable")]` from struct attributes.
///
/// Returns `Ok(Some(ident))` with the variant name (`Hidden`, `ReadOnly`, `Editable`)
/// if the attribute is present, `Ok(None)` to use the trait default, or
/// `Err(compile_error)` for invalid values.
pub(crate) fn parse_inspector_attr(
    input: &DeriveInput,
) -> Result<Option<proc_macro2::Ident>, TokenStream> {
    for attr in &input.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
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

/// Parses `#[reflect(asset = "TypeName")]` on a field. The annotated
/// field must be an `Option<ome_core::Guid>` and is exposed to the
/// inspector as a typed asset reference (renderered as a dropdown
/// picker filtered by `TypeName`).
pub(crate) fn parse_field_asset_type(field: &syn::Field) -> Result<Option<String>, TokenStream> {
    parse_field_string(field, "asset")
}

/// Parses `#[reflect(requires = "ComponentName")]` on an entity-reference
/// field: the short name of a component the target has to carry.
///
/// The inspector filters its picker by it and refuses a drop that does not
/// satisfy it. A `Joint` body without a `RigidBody` is not a body, and a
/// reference accepted but inert is indistinguishable from a broken one.
pub(crate) fn parse_field_requires(field: &syn::Field) -> Result<Option<String>, TokenStream> {
    parse_field_string(field, "requires")
}

/// The shared shape of `#[reflect(<key> = "...")]` on a field.
fn parse_field_string(field: &syn::Field, key: &str) -> Result<Option<String>, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue {
                path,
                value: syn::Expr::Lit(expr_lit),
                ..
            }) = meta
                && path.is_ident(key)
                && let Lit::Str(lit_str) = &expr_lit.lit
            {
                return Ok(Some(lit_str.value()));
            }
        }
    }
    Ok(None)
}

/// Parses `#[reflect(skip)]` on a field. Returns `true` when present,
/// `false` otherwise. Skipped fields are omitted from the FieldMeta
/// list and from the get/set match arms — opaque handle fields use
/// this to stay out of the editor inspector.
pub(crate) fn parse_field_skip(field: &syn::Field) -> Result<bool, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
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
pub(crate) fn parse_field_choices(field: &syn::Field) -> Result<Option<syn::Expr>, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
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

/// Parses `#[reflect(bits = PATH)]` from a field's attributes.
///
/// Returns `Ok(Some(expr))` with the path pointing to a
/// `&'static [::ome_ecs::reflect::FieldChoice]` constant naming each bit,
/// `Ok(None)` when absent, or `Err(compile_error)` on a parse failure.
pub(crate) fn parse_field_bits(field: &syn::Field) -> Result<Option<syn::Expr>, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta
                && path.is_ident("bits")
            {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

/// Parses `#[reflect(shown_when = PATH)]` from a field's attributes.
///
/// Returns `Ok(Some(expr))` with the path/identifier pointing to a
/// `::ome_ecs::reflect::FieldCondition` constant when present,
/// `Ok(None)` when absent, or `Err(compile_error)` on a parse failure.
pub(crate) fn parse_field_shown_when(field: &syn::Field) -> Result<Option<syn::Expr>, TokenStream> {
    for attr in &field.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            Ok(n) => n,
            Err(e) => return Err(e.to_compile_error().into()),
        };
        for meta in nested {
            if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta
                && path.is_ident("shown_when")
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
pub(crate) fn parse_category_attr(input: &DeriveInput) -> Result<Option<String>, TokenStream> {
    for attr in &input.attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let nested = match attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
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
