use kooch_ecs::reflect::ReflectValue;

use super::merged;

/// The type moved on after the file was written: its shape wins, the file's values survive.
#[test]
fn the_type_shapes_an_old_file() {
    let defaults = vec![
        ("kept".to_owned(), ReflectValue::F32(1.0)),
        ("added".to_owned(), ReflectValue::F32(2.0)),
        ("retyped".to_owned(), ReflectValue::F32(3.0)),
    ];
    let file = vec![
        ("gone".to_owned(), ReflectValue::F32(7.0)),
        ("kept".to_owned(), ReflectValue::F32(8.0)),
        ("retyped".to_owned(), ReflectValue::Bool(true)),
    ];

    let fields = merged(defaults, file);

    assert_eq!(
        fields,
        vec![
            ("kept".to_owned(), ReflectValue::F32(8.0)),
            ("added".to_owned(), ReflectValue::F32(2.0)),
            ("retyped".to_owned(), ReflectValue::F32(3.0)),
        ]
    );
}
