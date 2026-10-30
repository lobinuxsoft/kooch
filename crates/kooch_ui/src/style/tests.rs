use taffy::{FlexDirection, LengthPercentage};

use super::*;

const MENU: &str = include_str!("../../tests/data/menu.kooch_style");

fn resolve(rule: &StyleRule) -> ResolvedStyle {
    let mut style = ResolvedStyle::default();
    for declaration in &rule.declarations {
        style.apply(declaration);
    }
    style
}

#[test]
fn the_menu_sample_resolves() {
    let sheet = UiStyleSheet::parse(MENU);
    assert!(sheet.warnings.is_empty(), "{:#?}", sheet.warnings);
    let card = sheet
        .rules
        .iter()
        .find(|rule| rule.selectors[0].0[0].classes.first().map(AsRef::as_ref) == Some("card"))
        .expect("the .card rule");
    let style = resolve(card);
    assert_eq!(style.layout.flex_direction, FlexDirection::Column);
    assert_eq!(style.layout.gap.width, LengthPercentage::length(12.0));
    assert_eq!(style.layout.padding.left, LengthPercentage::length(24.0));
    assert_eq!(style.layout.border.top, LengthPercentage::length(3.0));
    // `var(--accent)`, from a `:root` the rule reads through.
    assert_eq!(style.paint.border, [1.0, 138.0 / 255.0, 31.0 / 255.0, 1.0]);
    assert_eq!(style.paint.radius, 18.0);

    let danger = sheet
        .rules
        .iter()
        .find(|rule| rule.selectors[0].0.len() == 2)
        .expect("the descendant rule");
    assert_eq!(danger.selectors[0].0[0].classes.len(), 2);
    assert_eq!(danger.selectors[0].0[1].name.as_deref(), Some("label"));
}

#[test]
fn an_unknown_property_only_warns() {
    let sheet = UiStyleSheet::parse(".a {\n  colour: red;\n  font-size: 20px;\n}");
    assert_eq!(sheet.warnings.len(), 1, "{:#?}", sheet.warnings);
    assert_eq!(sheet.warnings[0].line, 2);
    assert_eq!(
        sheet.rules[0].declarations,
        vec![Declaration::FontSize(20.0)]
    );
}
