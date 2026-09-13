//! `#[reflect(...)]` attribute parsing for both struct-level and field-level
//! annotations consumed by the `Reflect` derive macro.

use proc_macro::TokenStream;
use syn::{DeriveInput, Lit, Meta, MetaNameValue};

/// Parses the struct's `#[reflect(inspector = ...)]` (hidden, read_only, editable):
/// `Ok(Some(variant))`, `Ok(None)` for the default, or a compile error.
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

/// Parses `#[reflect(asset = ...)]` on an `Option<kooch_core::Guid>` field: a typed asset
/// reference, picked from a dropdown filtered by that type.
pub(crate) fn parse_field_asset_type(field: &syn::Field) -> Result<Option<String>, TokenStream> {
    parse_field_string(field, "asset")
}

/// Parses `#[reflect(requires = ...)]` on an entity-reference field: the component its target must
/// carry, used to filter and refuse picks.
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

/// Parses `#[reflect(skip)]`: the field is left out of `FieldMeta` and get/set, for opaque handles
/// the Inspector cannot show.
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

/// Parses `#[reflect(choices = PATH)]`, a `&'static [FieldChoice]` constant: `Ok(Some(path))`,
/// `Ok(None)`, or a compile error.
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

/// Parses `#[reflect(bits = PATH)]`, a `&'static [FieldChoice]` naming each bit: `Ok(Some(path))`,
/// `Ok(None)`, or a compile error.
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

/// Parses `#[reflect(shown_when = PATH)]`, a `FieldCondition` constant: `Ok(Some(path))`,
/// `Ok(None)`, or a compile error.
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

/// Parses `#[reflect(range = PATH)]`, a `FieldRange` constant — named, like `shown_when`, so the
/// bounds have one place to change.
pub(crate) fn parse_field_range(field: &syn::Field) -> Result<Option<syn::Expr>, TokenStream> {
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
                && path.is_ident("range")
            {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

/// Parses `#[reflect(group = ...)]`: the Inspector heading the field is drawn under; consecutive
/// fields sharing one form a section.
pub(crate) fn parse_field_group(field: &syn::Field) -> Result<Option<String>, TokenStream> {
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
                && path.is_ident("group")
                && let Lit::Str(lit_str) = &expr_lit.lit
            {
                return Ok(Some(lit_str.value()));
            }
        }
    }
    Ok(None)
}

/// Parses the struct's `#[reflect(category = ...)]`: `Ok(Some(name))`, `Ok(None)` for the default,
/// or a compile error for a non-string.
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

/// Collects a field's `#[doc]` attributes into its Inspector tooltip (#737), stripping the leading
/// space Rust adds; empty when there is none.
pub(crate) fn parse_field_doc(field: &syn::Field) -> String {
    let mut lines: Vec<String> = Vec::new();
    for attr in &field.attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(MetaNameValue {
            value: syn::Expr::Lit(expr_lit),
            ..
        }) = &attr.meta
            && let Lit::Str(lit_str) = &expr_lit.lit
        {
            let mut line = lit_str.value();
            // `/// text` reaches here as `" text"`. Left in, every line
            // of every tooltip would be indented by one space.
            if line.starts_with(' ') {
                line.remove(0);
            }
            lines.push(line);
        }
    }
    // Trailing blank lines come from a doc comment ending in `///`,
    // which is common above a `#[reflect(...)]` attribute and would
    // render as empty space at the bottom of the tooltip.
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}
