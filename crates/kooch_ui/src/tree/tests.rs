use glam::Vec2;
use taffy::prelude::{length, percent};
use taffy::{AvailableSpace, FlexDirection, Size, Style};

use super::{UiNode, UiTree};
use crate::layout::canvas;

#[test]
fn a_named_node_is_queried() {
    let mut tree = UiTree::new(Style::default());
    let hud = tree.push(UiTree::ROOT, Style::default()).unwrap();
    let speed = tree.push(hud, Style::default()).unwrap();
    tree.set_name(speed, "speed");

    assert_eq!(tree.query("speed"), Some(speed));
    assert_eq!(tree.query("torque"), None);
    assert_eq!(tree.push(UiNode(9), Style::default()), None);
}

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

    let view = canvas(glam::UVec2::new(1280, 720));
    assert_eq!(view, canvas(glam::UVec2::new(1920, 1080)));
    assert!(tree.layout(screen(view)));
    let half = (view.x - 25.0) / 2.0;
    assert_eq!(tree.rect(left).unwrap().size.x, half);
    assert_eq!(tree.rect(right).unwrap().min.x, 5.0 + 10.0 + half);

    assert!(!tree.layout(screen(view)));
    tree.set_style(right, Style::default());
    assert!(tree.layout(screen(view)));
}
