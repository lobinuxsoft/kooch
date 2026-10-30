use crate::build_tree;
use crate::source::UiSource;
use crate::style::UiStyleSheet;

const MENU_UI: &str = include_str!("../tests/data/menu.kooch_ui");
const MENU_STYLE: &str = include_str!("../tests/data/menu.kooch_style");

#[test]
fn the_menu_sample_builds() {
    let source = UiSource::parse(MENU_UI).expect("the sample parses");
    assert_eq!(source.style.as_deref(), Some("menu.kooch_style"));
    let sheet = UiStyleSheet::parse(MENU_STYLE);
    let tree = build_tree(&source, Some(&sheet));
    let menu = tree.query("menu").expect("the menu panel");
    let surface = [16.0 / 255.0, 16.0 / 255.0, 20.0 / 255.0, 0.85];
    assert_eq!(tree.paint(menu).map(|p| p.background), Some(surface));
    assert!(tree.query("volume").is_some());
    // A template, until phase 5.
    assert!(tree.query("resume").is_none());
}

#[test]
fn specificity_and_inheritance_resolve() {
    let source = UiSource::parse(
        r#"<ui><panel name="p"><label name="a" class="b">x</label><label name="c">y</label></panel></ui>"#,
    )
    .expect("parses");
    let sheet = UiStyleSheet::parse(
        "#a { color: red; } .b { color: blue; } #p { color: yellow; } panel label.b { color: aqua; }",
    );
    let tree = build_tree(&source, Some(&sheet));
    let text = |name| tree.query(name).and_then(|n| tree.paint(n)).map(|p| p.text);
    // `#a` (1,0,0) over the later `panel label.b` (0,1,2).
    assert_eq!(text("a"), Some([1.0, 0.0, 0.0, 1.0]));
    // No rule of its own: the panel's.
    assert_eq!(text("c"), Some([1.0, 1.0, 0.0, 1.0]));
}
