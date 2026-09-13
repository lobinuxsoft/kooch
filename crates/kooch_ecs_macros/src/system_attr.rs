//! `#[system(...)]` — where a system binds into the frame, said at the system. It expands to the
//! function unchanged; the editor's codegen reads it, and a mistyped stage is a compile error
//! rather than a silent `Update`.
//!
//! # Grammar
//!
//! ```ignore
//! #[system]                     // Update, gated by Play — the default
//! #[system(PreUpdate)]          // PreUpdate, gated by Play
//! #[system(PostUpdate, always)] // PostUpdate, runs while editing too
//! ```
//!
//! `always` is a word because no scan could infer that a system must run while the editor is
//! paused.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, Token, punctuated::Punctuated};

/// The fourteen stages, in `Stage::ALL` order — copied, since a proc-macro crate cannot depend on
/// the engine; the test keeps them honest.
pub(crate) const STAGES: [&str; 14] = [
    "Startup",
    "First",
    "Input",
    "PreUpdate",
    "Update",
    "PostUpdate",
    "GpuSync",
    "Gpu",
    "Physics",
    "PostPhysics",
    "PreRender",
    "Render",
    "PostRender",
    "Last",
];

pub(crate) fn system_impl(args: TokenStream, item: TokenStream) -> TokenStream {
    let parser = Punctuated::<Ident, Token![,]>::parse_terminated;
    let args = match syn::parse::Parser::parse(parser, args) {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };

    let mut seen_always = false;
    for (index, arg) in args.iter().enumerate() {
        let name = arg.to_string();
        if index == 0 && STAGES.contains(&name.as_str()) {
            continue;
        }
        if name == "always" && !seen_always {
            seen_always = true;
            continue;
        }
        let message = if index == 0 {
            format!(
                "`{name}` is not a stage. Expected one of: {}",
                STAGES.join(", ")
            )
        } else {
            format!("`{name}` is not a system modifier. Expected `always`")
        };
        return syn::Error::new(arg.span(), message)
            .to_compile_error()
            .into();
    }

    // The function, untouched. See the header.
    let item: proc_macro2::TokenStream = item.into();
    quote!(#item).into()
}

#[cfg(test)]
mod tests;
