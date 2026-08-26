use super::*;

#[test]
fn set_and_get() {
    let mut pd = PluginData::new();
    pd.set("key", b"value");

    assert_eq!(pd.get("key"), Some(b"value".as_slice()));
    assert_eq!(pd.get("missing"), None);
}

#[test]
fn overwrite() {
    let mut pd = PluginData::new();
    pd.set("k", b"first");
    pd.set("k", b"second");

    assert_eq!(pd.get("k"), Some(b"second".as_slice()));
    assert_eq!(pd.len(), 1);
}

#[test]
fn remove() {
    let mut pd = PluginData::new();
    pd.set("k", b"data");

    let removed = pd.remove("k");
    assert_eq!(removed, Some(b"data".to_vec()));
    assert!(pd.is_empty());
}

#[test]
fn empty_data() {
    let mut pd = PluginData::new();
    pd.set("empty", b"");

    assert_eq!(pd.get("empty"), Some(b"".as_slice()));
}
