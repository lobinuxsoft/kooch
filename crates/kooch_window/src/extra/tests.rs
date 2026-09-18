use super::*;

/// A request stays pending until the window it asked for is taken.
#[test]
fn a_request_is_pending_until_taken() {
    let mut windows = ExtraWindows::default();
    windows.request(7, WindowAttributes::default());
    assert!(windows.is_pending(7));
    assert!(!windows.is_pending(8));
    assert_eq!(windows.take_requests().len(), 1);
    assert!(!windows.is_pending(7), "no window was created for it");
}
