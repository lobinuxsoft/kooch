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
/// field must be an `Option<kooch_core::Guid>` and is exposed to the
/// inspector as a typed asset reference (renderered as a dropdown
/// picker filtered by `TypeName`).
pub(crate) fn parse_field_asset_type(field: &syn::Field) -> Result<Option<String>, TokenStream> {
    parse_field_string(field, "asset")
}

/// Parses `#[reflect(requires = "ComponentName")]` on an entity-reference
/// field: the short name of a component the target has to carry.
///
/// The inspector filters its picker by it and refuses a drop that does not
/// satisfy it. A `Joint` body without a `PhysicsBody` is not a body, and a
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
/// `&'static [::kooch_ecs::reflect::FieldChoice]` constant when present,
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
/// `&'static [::kooch_ecs::reflect::FieldChoice]` constant naming each bit,
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
/// `::kooch_ecs::reflect::FieldCondition` constant when present,
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

/// Parses `#[reflect(group = "...")]` from a field's attributes.
///
/// The heading the Inspector draws the field under. Consecutive fields
/// sharing a group form one section — see [`FieldMeta::group`] for why
/// this is a label rather than a nested struct.
///
/// [`FieldMeta::group`]: ::kooch_ecs::reflect::FieldMeta::group
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

/// Collects a field's doc comment into a single string for the
/// Inspector tooltip (#737).
///
/// Doc comments desugar to `#[doc = "..."]` attributes before a proc
/// macro ever sees them, so this is a plain attribute walk — there is
/// nothing to ask the compiler for.
///
/// Rust puts a leading space after `///`, which would indent every line
/// of the tooltip, so it is stripped. Everything else is left alone:
/// markdown, links and code fences render as text, which is worse than
/// rendered markdown and much better than no explanation at all.
///
/// Returns `""` for a field with no doc comment. The Inspector shows no
/// tooltip rather than an empty box.
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
