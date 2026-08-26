use super::*;

#[test]
fn a_split_key_reassembles() {
    let key = PackKey::generate();
    assert_eq!(SplitKey::split(&key).assemble(), key);
}

#[test]
fn shares_round_trip_through_hex() {
    let key = PackKey::generate();
    let split = SplitKey::split(&key);

    let hex = split.to_hex();
    assert_eq!(SplitKey::parse(&hex).unwrap().assemble(), key);
}

/// 🔴 The property that makes this arithmetic rather than obfuscation:
/// every share but the last is random and the last is the key XOR'd with
/// them, so **no single share carries any information about the key**.
/// It is a one-time pad, and that part is not a matter of effort.
#[test]
fn no_single_share_is_the_key() {
    let key = PackKey::generate();
    let split = SplitKey::split(&key);
    let master = key.to_hex();

    for share in split.to_hex() {
        assert_ne!(share, master, "a share is the key itself");
    }
}

/// The whole point: a shipped binary has no thirty-two contiguous bytes
/// that are the key, so the entropy scan that finds one in fifty
/// milliseconds finds nothing.
#[test]
fn the_key_bytes_are_nowhere_in_the_shares() {
    let key = PackKey::generate();
    let split = SplitKey::split(&key);
    let wanted = key.to_hex();

    let all: String = split.to_hex().join("");
    assert!(!all.contains(&wanted));
}

/// Two builds of one project must not ship the same bytes, or the shares
/// become a fingerprint that identifies the key across releases.
#[test]
fn splitting_twice_gives_different_shares() {
    let key = PackKey::generate();

    assert_ne!(
        SplitKey::split(&key).to_hex(),
        SplitKey::split(&key).to_hex()
    );
}

/// Same key, different shares, and both still open the same pack.
#[test]
fn different_shares_assemble_to_one_key() {
    let key = PackKey::generate();

    assert_eq!(
        SplitKey::split(&key).assemble(),
        SplitKey::split(&key).assemble(),
    );
}

#[test]
fn the_wrong_number_of_shares_is_none() {
    let key = PackKey::generate();
    let hex = SplitKey::split(&key).to_hex().to_vec();

    assert!(SplitKey::parse(&hex[..SHARES - 1]).is_none());
    assert!(SplitKey::parse(&[]).is_none());
}

#[test]
fn a_malformed_share_is_none() {
    let mut hex = SplitKey::split(&PackKey::generate()).to_hex().to_vec();
    hex[1] = "not hex".to_owned();

    assert!(SplitKey::parse(&hex).is_none());
}

/// Same reasoning as `PackKey`: shares are key material, and `{:?}` is
/// how key material reaches a log without anyone deciding to put it
/// there.
#[test]
fn shares_never_print_themselves() {
    let split = SplitKey::split(&PackKey::generate());
    let printed = format!("{split:?}");

    for share in split.to_hex() {
        assert!(!printed.contains(&share));
    }
}
