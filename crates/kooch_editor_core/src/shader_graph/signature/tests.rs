use super::functions;
use crate::shader_graph::Node;

#[test]
fn a_helper_is_no_node() {
    let source = "fn helper(a: f32) -> f32 { return a; }\n/// Doubles.\nfn twice(a: f32) -> f32 { return a; }\n";
    assert_eq!(functions("a.wgsl", source).len(), 1);
}

#[test]
fn a_default_reaches_its_pin() {
    let source =
        "/// x\nfn wave(\n    uv: vec2<f32>,\n    speed: f32, // @default(2)\n) -> f32 {\n";
    let (node, _) = functions("a.wgsl", source).remove(0);
    let Node::Function { inputs, .. } = node else {
        panic!("not a function");
    };
    assert_eq!(inputs[1].default, [2.0; 4]);
    assert_eq!(inputs[0].width, 2);
}

#[test]
fn a_foreign_type_is_skipped() {
    let source = "/// x\nfn pick(t: texture_2d<f32>) -> f32 { return 0.0; }\n";
    assert!(functions("a.wgsl", source).is_empty());
}
