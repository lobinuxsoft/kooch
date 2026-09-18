use super::SnarlStyle;

#[test]
const fn snarl_style_is_send_sync() {
    const fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<SnarlStyle>();
}
