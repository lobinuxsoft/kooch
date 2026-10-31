use glam::Vec2;
use taffy::prelude::{length, percent};
use taffy::{AvailableSpace, FlexDirection, Size, Style};

use super::UiTree;
use crate::layout::canvas;
use crate::{GlyphAtlas, UiFonts};

fn screen(view: Vec2) -> Size<AvailableSpace> {
    Size {
        width: AvailableSpace::Definite(view.x),
        height: AvailableSpace::Definite(view.y),
    }
}

/// Absolute positions: the second half sits past the first, inside two padded parents.
#[test]
fn a_row_splits_the_canvas() {
    let mut tree = UiTree::new(Style {
        size: Size {
            width: percent(1.0),
            height: percent(1.0),
        },
        padding: taffy::Rect {
            left: length(5.0),
            right: length(0.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Style::default()
    });
    let row = tree
        .push(
            UiTree::ROOT,
            Style {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                padding: taffy::Rect {
                    left: length(10.0),
                    right: length(10.0),
                    top: length(0.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
        )
        .unwrap();
    let grow = Style {
        flex_grow: 1.0,
        ..Style::default()
    };
    let left = tree.push(row, grow.clone()).unwrap();
    let right = tree.push(row, grow).unwrap();

    let mut fonts = UiFonts::default();
    let view = canvas(glam::UVec2::new(1280, 720));
    assert_eq!(view, canvas(glam::UVec2::new(1920, 1080)));
    assert!(tree.layout(screen(view), &mut fonts));
    let half = (view.x - 25.0) / 2.0;
    assert_eq!(tree.rect(left).unwrap().size.x, half);
    assert_eq!(tree.rect(right).unwrap().min.x, 5.0 + 10.0 + half);

    assert!(!tree.layout(screen(view), &mut fonts));
    tree.set_style(right, Style::default());
    assert!(tree.layout(screen(view), &mut fonts));
}

/// Accents, ñ and a dash: the text is measured, pushes its sibling, and its glyphs reach the atlas
/// once per scale.
#[test]
fn text_is_measured_and_rasterized() {
    let mut fonts = UiFonts::default();
    let mut atlas = GlyphAtlas::default();
    let mut tree = UiTree::new(Style {
        flex_direction: FlexDirection::Row,
        ..Style::default()
    });
    let label = tree.push(UiTree::ROOT, Style::default()).unwrap();
    tree.set_text(label, "Velocidad máxima: 12,5 m/s — ñandú", 32.0);
    let after = tree
        .push(
            UiTree::ROOT,
            Style {
                size: Size {
                    width: length(50.0),
                    height: length(50.0),
                },
                ..Style::default()
            },
        )
        .unwrap();

    tree.layout(screen(canvas(glam::UVec2::new(1920, 1080))), &mut fonts);
    let width = tree.rect(label).unwrap().size.x;
    assert!(width > 300.0, "{width}");
    assert_eq!(tree.rect(after).unwrap().min.x, width);

    assert!(tree.rasterize(1.0, &mut fonts, &mut atlas));
    let glyphs = tree.glyphs().len();
    assert!(glyphs > 25, "{glyphs}");
    assert!(atlas.take_dirty().is_some());
    assert!(!tree.rasterize(1.0, &mut fonts, &mut atlas));

    let small = tree.glyphs()[0].size;
    assert!(tree.rasterize(2.0, &mut fonts, &mut atlas));
    assert!(tree.glyphs()[0].size[1] > small[1] * 1.8);
}

/// Fira Code's `=>` is a ligature: the shaper swaps in glyphs a lone `=` and `>` never use.
#[test]
fn fira_code_shapes_ligatures() {
    let mut fonts = UiFonts::default();
    let ids = |content: &str, fonts: &mut UiFonts| {
        let mut text = crate::text::Text {
            content: content.into(),
            size: 32.0,
            family: Some("Fira Code".into()),
            layout: None,
            origin: Vec2::ZERO,
        };
        let layout = text.shaped(fonts);
        crate::text::measure(layout, None);
        let mut ids = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                    ids.extend(run.glyphs().map(|glyph| glyph.id));
                }
            }
        }
        ids
    };
    let joined = ids("=>", &mut fonts);
    let apart = ids("= >", &mut fonts);
    assert_eq!(joined.len(), 2);
    assert_ne!(joined[1], apart[2]);
}
