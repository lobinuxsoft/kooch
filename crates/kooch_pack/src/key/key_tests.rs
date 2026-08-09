use super::*;

#[test]
fn a_key_round_trips_through_hex() {
    let key = PackKey::generate();
    assert_eq!(PackKey::parse(&key.to_hex()), Some(key));
}

#[test]
fn hex_is_64_lowercase_characters() {
    let hex = PackKey::generate().to_hex();
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
    );
}

/// It arrives pasted, and a paste brings whatever was around it.
#[test]
fn pasted_whitespace_is_ignored() {
    let key = PackKey::generate();
    let messy = format!("  {}\n", key.to_hex());
    assert_eq!(PackKey::parse(&messy), Some(key));
}

#[test]
fn a_malformed_key_is_none() {
    assert_eq!(PackKey::parse(""), None);
    assert_eq!(PackKey::parse("abc"), None);
    assert_eq!(PackKey::parse(&"z".repeat(64)), None);
    assert_eq!(PackKey::parse(&"a".repeat(63)), None);
    assert_eq!(PackKey::parse(&"a".repeat(65)), None);
}

/// Two calls must not agree, or every project ships under one key.
#[test]
fn generated_keys_differ() {
    assert_ne!(PackKey::generate(), PackKey::generate());
}

/// 🔴 One key, two jobs is how a public value ends up related to a
/// secret one: the tag sits in the clear at byte 0 of every pack, and the
/// data key encrypts. Domain separation is what keeps the first from
/// saying anything about the second.
#[test]
fn the_tag_and_the_data_key_differ() {
    let key = PackKey::generate();

    assert_ne!(&key.data_key()[..8], &key.tag()[..]);
    assert_ne!(key.data_key(), *key.bytes_for_split());
}

/// Derivation has to be a function of the key, or a pack written today
/// does not open tomorrow.
#[test]
fn derivation_is_deterministic() {
    let key = PackKey::generate();
    let same = PackKey::parse(&key.to_hex()).unwrap();

    assert_eq!(key.tag(), same.tag());
    assert_eq!(key.data_key(), same.data_key());
}

/// And a function of *that* key: two projects must not share a tag, or
/// the tag identifies the engine rather than the pack.
#[test]
fn another_key_gives_another_tag() {
    assert_ne!(PackKey::generate().tag(), PackKey::generate().tag());
}

/// 🔴 A key that reaches a log reaches wherever logs go — a bug report, a
/// screenshot of a terminal, CI output kept for a year. The derive would
/// have put it in every `{:?}` of every struct holding one.
#[test]
fn a_key_never_prints_itself() {
    let key = PackKey::generate();
    let hex = key.to_hex();

    assert!(!format!("{key:?}").contains(&hex));
    assert!(!format!("{key}").contains(&hex));
    assert!(!format!("{:?}", Some(key.clone())).contains(&hex));
}
