//! Captures the compiler identity into the build, for [`BuildStamp`].
//!
//! Passing a `Box<dyn Trait>` across a dynamic library boundary is only
//! sound when both sides were built by the same compiler: Rust does not
//! guarantee vtable layout between versions. Nothing checks that at run
//! time unless we make it, so the exact `rustc -V -v` output is baked in
//! and compared at load.
//!
//! No dependencies: this shells out to the compiler cargo already told
//! us to use.

use std::process::Command;

fn main() {
    // `-v` includes the commit hash and host triple, so a nightly and a
    // stable of the same version number do not collide, and neither do
    // two hosts.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let version = Command::new(rustc)
        .args(["-V", "-v"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_else(|| {
            // Refusing to build would be worse than a stamp that only
            // covers the API version: the loader still rejects
            // mismatched API versions, and a missing compiler string is
            // recorded as such rather than silently faked.
            println!("cargo:warning=could not read rustc version; plugin build stamp is weaker");
            "unknown-rustc".to_owned()
        });

    let normalised = version.replace(['\n', '\r'], "|");
    println!("cargo:rustc-env=OME_RUSTC_IDENT={normalised}");
    println!("cargo:rerun-if-env-changed=RUSTC");
}
