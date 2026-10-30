//! `.kooch_style` (#280, syntax in #1693): a CSS subset parsed on Servo's `cssparser`. Layout values
//! go through taffy's own CSS parser; this module owns the rule grammar and the mapping.

mod declaration;
mod resolved;
mod sheet;
mod tokens;

pub use declaration::{Axis, Declaration, Edge};
pub use resolved::ResolvedStyle;
pub use sheet::{
    Compound, STATE_ACTIVE, STATE_CHECKED, STATE_DISABLED, STATE_FOCUS, STATE_HOVER, STATE_ROOT,
    Selector, StyleRule, StyleWarning, UiStyleSheet,
};

#[cfg(test)]
mod tests;

/// Extension the asset database routes to [`UiStyleLoader`].
pub const STYLE_EXTENSION: &str = "kooch_style";

/// Reads a `.kooch_style` file into a [`UiStyleSheet`], logging what it skipped.
#[derive(Debug, Default, Clone, Copy)]
pub struct UiStyleLoader;

impl kooch_core::asset_loader::AssetLoader<UiStyleSheet> for UiStyleLoader {
    fn extensions(&self) -> &[&'static str] {
        &[STYLE_EXTENSION]
    }

    fn load(
        &self,
        bytes: &[u8],
        ctx: &mut kooch_core::asset_loader::LoadContext<'_>,
    ) -> kooch_core::asset_loader::AssetResult<UiStyleSheet> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| kooch_core::asset_loader::AssetError::Loader(Box::new(e)))?;
        let sheet = UiStyleSheet::parse(text);
        warn_skipped(ctx.path, &sheet);
        Ok(sheet)
    }
}

kooch_core::register_asset!(UiStyleSheet, UiStyleLoader);

pub(crate) fn warn_skipped(path: &std::path::Path, sheet: &UiStyleSheet) {
    for warning in &sheet.warnings {
        tracing::warn!(
            "{}:{}:{}: {}",
            path.display(),
            warning.line,
            warning.column,
            warning.message
        );
    }
}
