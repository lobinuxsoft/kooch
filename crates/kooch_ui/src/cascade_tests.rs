use crate::build_tree;
use crate::source::UiSource;
use crate::style::UiStyleSheet;

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
