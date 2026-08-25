//! `#[system(...)]` — where a system binds into the frame, said at the
//! system.
//!
//! # Why the macro emits nothing
//!
//! The editor generates `registrations.rs` by SCANNING `src/`, and until
//! now that scan had exactly one policy: every system it found went to
//! `Stage::Update` wrapped in `run_if_playing`. The scanner did not lack
//! power — it lacked anything to read. This attribute is that.
//!
//! So the expansion is the function, unchanged. Nothing is generated,
//! nothing is wrapped, and a build with the attribute is byte-identical
//! to a build without it. Remove it and the code still compiles; the
//! system just goes back to the default binding.
//!
//! # Why it is not merely a comment
//!
//! Because it VALIDATES. `#[system(PostUpdte)]` is a typo, and the whole
//! failure mode this project keeps meeting is the one that logs nothing:
//! a comment with that typo would leave the system in `Update` forever
//! and never say so. The stage name is checked against the fourteen here,
//! at compile time, and a wrong one is a compile error naming the
//! alternatives.
//!
//! # Grammar
//!
//! ```ignore
//! #[system]                     // Update, gated by Play — the default
//! #[system(PreUpdate)]          // PreUpdate, gated by Play
//! #[system(PostUpdate, always)] // PostUpdate, runs while editing too
//! ```
//!
//! `always` is the one thing a scanner could never infer, which is the
//! reason it is a word rather than a convention: a system that must run
//! while the editor is paused — a gizmo, an overlay, a streaming pump —
//! is indistinguishable from a gameplay one by any amount of looking at
//! its name or its body.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, Token, punctuated::Punctuated};

/// The fourteen, in the order [`Stage::ALL`] runs them.
///
/// Duplicated from `kooch_core::stage` rather than imported: a
/// proc-macro crate compiles for the HOST and cannot depend on the
/// engine it generates code for. The test below is what keeps the two
/// lists honest.
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
