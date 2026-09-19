//! Typed parameter nodes: old files opening typed, their editors, textures, scalars and constants.

use super::*;

/// 🔴 Graphs written before #1170 carry `Param { width, color }` in their file. They must open as the
/// typed nodes they mean — or every shader authored until then opens with its parameters gone.
#[test]
fn an_old_param_opens_typed() {
    let migrate = |width, color| {
        Node::Param {
            name: "p".to_owned(),
            width,
            color,
            default: [0.25; 4],
        }
        .migrated()
    };
    assert!(matches!(
        migrate(1, false),
        Node::Float {
            default: 0.25,
            range: None,
            ..
        }
    ));
    assert!(matches!(migrate(3, true), Node::Vector { width: 3, .. }));
    assert!(matches!(migrate(4, true), Node::Color { .. }));
    assert!(matches!(migrate(4, false), Node::Vector { width: 4, .. }));
}

/// And through the real door: an old file's graph comes back out of `extract` already typed.
#[test]
fn an_old_file_extracts_typed() {
    let mut graph = Graph::new();
    graph.insert_node(
        Pos2::ZERO,
        Node::Param {
            name: "tint".to_owned(),
            width: 4,
            color: true,
            default: [1.0; 4],
        },
    );
    graph.insert_node(Pos2::ZERO, Node::surface_output());
    let source = generate(&graph).unwrap();

    let read = extract(&source).expect("the graph");

    assert!(read.nodes().any(|n| matches!(n, Node::Color { .. })));
    assert!(!read.nodes().any(|n| matches!(n, Node::Param { .. })));
}

/// Each typed node reaches the engine as the kind the Inspector draws: a slider, a stepped slider, a
/// picker.
#[test]
fn typed_params_keep_their_editors() {
    use kooch_render::material::ParamKind;

    let mut graph = Graph::new();
    for node in [
        Node::Float {
            name: "amount".to_owned(),
            default: 0.5,
            range: Some([0.0, 2.0]),
        },
        Node::Int {
            name: "sides".to_owned(),
            default: 6.0,
            range: Some([3.0, 12.0]),
        },
        Node::Color {
            name: "tint".to_owned(),
            default: [1.0; 4],
        },
        Node::Vector {
            name: "offset".to_owned(),
            width: 2,
            default: [0.0; 4],
        },
        Node::Output,
    ] {
        graph.insert_node(Pos2::ZERO, node);
    }

    let shader = Shader::parse(&generate(&graph).unwrap()).unwrap();
    let kind = |name: &str| {
        let param = shader.params.iter().find(|p| p.name == name).unwrap();
        (param.kind, param.range)
    };

    assert_eq!(kind("amount"), (ParamKind::Float, Some([0.0, 2.0])));
    assert_eq!(kind("sides"), (ParamKind::Int, Some([3.0, 12.0])));
    assert_eq!(kind("tint"), (ParamKind::Color, None));
    assert_eq!(kind("offset"), (ParamKind::Vec2, None));
}

/// 🔴 Every graph written before #1170 has texture nodes with no `preview` field at all. It has to
/// default rather than fail, or `extract` returns nothing and the file opens in the IDE instead.
#[test]
fn a_texture_without_preview_opens() {
    let source = generate(&tinted_texture()).unwrap();
    let old = source.replace(",preview:None", "");
    assert_ne!(
        old, source,
        "the fixture no longer writes the field this test strips"
    );

    let read = extract(&old).expect("an old graph still opens");

    assert!(
        read.nodes()
            .any(|n| matches!(n, Node::Texture { preview: None, .. }))
    );
}

/// Old graphs carry `Constant([f32; 4])`. All four components were always live, so it opens as a
/// four-wide constant vector with every value kept (#1170).
#[test]
fn an_old_constant_opens_typed() {
    assert!(matches!(
        Node::Constant([1.0, 2.0, 3.0, 4.0]).migrated(),
        Node::ConstVector {
            width: 4,
            value: [1.0, 2.0, 3.0, 4.0]
        }
    ));
}

/// 🔴 A scalar fills every channel: `colour × float` scaled only red while it read `(v, 0, 0, 0)`.
#[test]
fn a_scalar_fills_every_channel() {
    let emitted = |node: Node| {
        let mut graph = Graph::new();
        let scalar = graph.insert_node(Pos2::ZERO, node);
        let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
        graph.connect(
            OutPinId {
                node: scalar,
                output: 0,
            },
            InPinId {
                node: output,
                input: 0,
            },
        );
        generate(&graph).unwrap()
    };
    assert!(emitted(Node::ConstFloat(0.5)).contains("vec4<f32>(0.5, 0.5, 0.5, 0.5)"));
    let float = Node::Float {
        name: "levels".to_owned(),
        default: 31.0,
        range: None,
    };
    assert!(emitted(float).contains("vec4<f32>(f32(p.levels))"));
}

/// A constant writes only the components its type has, and an int writes a whole number.
#[test]
fn a_constant_writes_its_type() {
    let emitted = |node: Node| {
        let mut graph = Graph::new();
        let constant = graph.insert_node(Pos2::ZERO, node);
        let output = graph.insert_node(Pos2::ZERO, Node::surface_output());
        graph.connect(
            OutPinId {
                node: constant,
                output: 0,
            },
            InPinId {
                node: output,
                input: 0,
            },
        );
        generate(&graph).unwrap()
    };

    assert!(emitted(Node::ConstInt(2.6)).contains("vec4<f32>(3.0, 3.0, 3.0, 3.0)"));
    assert!(
        emitted(Node::ConstVector {
            width: 2,
            value: [1.0, 2.0, 9.0, 9.0]
        })
        .contains("vec4<f32>(1.0, 2.0, 0.0, 0.0)"),
        "a two-wide vector leaked the components it does not have",
    );
}
