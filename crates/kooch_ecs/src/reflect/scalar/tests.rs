use glam::Vec3;

use super::{scalar_list, scalar_list_from};
use crate::reflect::{FieldKind, ReflectValue};

/// 🔴 A mesh is the reason this exists: positions and indices are lists of plain values, and a
/// round trip that loses one is a block that loads with holes in it.
#[test]
fn a_scalar_list_round_trips() {
    let points = vec![Vec3::X, Vec3::new(1.0, 2.0, 3.0)];
    let read: Vec<Vec3> = scalar_list_from(scalar_list(&points), "positions").expect("Vec3 list");
    assert_eq!(read, points);

    let indices = vec![0_u32, 3, 2, 1];
    let read: Vec<u32> = scalar_list_from(scalar_list(&indices), "corners").expect("u32 list");
    assert_eq!(read, indices);
}

/// An empty list still says what it holds, or the Inspector's add button has nothing to add.
#[test]
fn an_empty_list_names_its_element() {
    let ReflectValue::List { items, element } = scalar_list::<u32>(&[]) else {
        panic!("not a list");
    };
    assert!(items.is_empty());
    assert_eq!(element.kind(), FieldKind::U32);
}

/// The wrong kind of item is refused rather than silently dropped: a list that quietly loses the
/// values it could not read is a mesh that loads short.
#[test]
fn a_mistyped_item_is_refused() {
    let mixed = ReflectValue::List {
        items: vec![ReflectValue::U32(1), ReflectValue::Bool(true)],
        element: Box::new(ReflectValue::U32(0)),
    };
    assert!(scalar_list_from::<u32>(mixed, "corners").is_err());
}

/// 🔴 Through the derive, which is the half the runtime test above cannot reach: before #1452 a
/// `Vec<u32>` field was a compile error, and that is what forced a block's geometry into an asset.
#[test]
fn the_derive_carries_a_scalar_list() {
    use crate::reflect::Reflect;

    #[derive(Debug, Default, Clone, PartialEq, crate::Reflect)]
    struct Shell {
        positions: Vec<Vec3>,
        #[reflect(hidden)]
        corners: Vec<u32>,
    }

    let held = Shell {
        positions: vec![Vec3::X, Vec3::Y],
        corners: vec![0, 1, 2],
    };
    let mut read = Shell::default();
    for field in ["positions", "corners"] {
        let value = held.reflect_get(field).expect("the field reflects");
        read.reflect_set(field, value).expect("and reads back");
    }
    assert_eq!(read, held);

    let metas = held.reflect_fields();
    assert_eq!(metas[0].kind, FieldKind::List);
    assert_eq!(metas[0].type_name, "Vec<Vec3>");
    // Nobody edits five thousand vertices by hand, so geometry hides from the Inspector.
    assert!(metas[1].hidden, "a hidden list stays hidden");
}
