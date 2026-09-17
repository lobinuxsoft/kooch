//! Test code for `AssetSnapshot`, in its own file.

use kooch_core::Guid;

use super::{AssetDetail, AssetSnapshot};

#[test]
fn a_stale_snapshot_is_hidden() {
    let gathered = Guid::new_v4();
    let snapshot = AssetSnapshot {
        guid: gathered,
        detail: AssetDetail::Unknown {
            type_name: "x".to_owned(),
        },
    };
    assert!(AssetSnapshot::of(Some(&snapshot), Guid::new_v4()).is_none());
    assert!(AssetSnapshot::of(Some(&snapshot), gathered).is_some());
}
