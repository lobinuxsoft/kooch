use super::*;

#[test]
fn v4_is_unique_per_call() {
    let a = Guid::new_v4();
    let b = Guid::new_v4();
    assert_ne!(a, b);
}

#[test]
fn display_is_32_lowercase_hex_no_hyphens() {
    let g = Guid::from_bytes([
        0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00,
        0x00,
    ]);
    let rendered = g.to_string();
    assert_eq!(rendered, "550e8400e29b41d4a716446655440000");
    assert_eq!(rendered.len(), 32);
    assert!(
        rendered
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
}

#[test]
fn round_trip_via_string() {
    let g = Guid::new_v4();
    let s = g.to_string();
    let parsed: Guid = s.parse().expect("display form should parse");
    assert_eq!(g, parsed);
}

#[test]
fn parses_hyphenated_form_too() {
    // Unity stores no hyphens, but we accept both so users can
    // paste a UUID from any common source.
    let hyphenated = "550e8400-e29b-41d4-a716-446655440000";
    let parsed: Guid = hyphenated.parse().expect("hyphenated form should parse");
    assert_eq!(parsed.to_string(), "550e8400e29b41d4a716446655440000");
}

#[test]
fn rejects_garbage() {
    assert!("not a guid".parse::<Guid>().is_err());
    assert!("".parse::<Guid>().is_err());
}

#[test]
fn serde_round_trip_through_toml() {
    // The `.meta` sidecar serializes through serde + toml. Confirm
    // the transparent newtype renders as a plain string field.
    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Wrap {
        guid: Guid,
    }
    let original = Wrap {
        guid: Guid::new_v4(),
    };
    let text = toml::to_string(&original).expect("serializes");
    assert!(text.contains("guid = "));
    assert!(!text.contains("Guid"), "should render as plain string");
    let parsed: Wrap = toml::from_str(&text).expect("deserializes");
    assert_eq!(parsed, original);
}
