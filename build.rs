//! Makes cargo notice when the pack key changes (#758).
//!
//! `src/shipped.rs` reads `KOOCH_PACK_SHARES` with `option_env!`, which
//! is baked in at compile time. Without this line cargo would happily
//! reuse a cached build of this crate, and a game rebuilt with a new key
//! would carry the previous one — or none at all, and open nothing.

fn main() {
    println!("cargo:rerun-if-env-changed=KOOCH_PACK_SHARES");
}
