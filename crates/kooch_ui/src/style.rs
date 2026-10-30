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
