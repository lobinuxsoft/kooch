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
