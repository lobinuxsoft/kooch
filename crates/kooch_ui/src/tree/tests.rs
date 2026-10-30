use taffy::Style;

use super::{UiNode, UiTree};

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
