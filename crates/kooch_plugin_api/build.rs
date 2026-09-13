//! Bakes the exact `rustc -V -v` into the build for [`BuildStamp`]: a `Box<dyn Trait>` across a
//! dylib is sound only with one compiler, and nothing else checks it.

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
            // Building anyway beats refusing: the loader still checks the API version, and a
            // missing compiler string is recorded, not faked.
            println!("cargo:warning=could not read rustc version; plugin build stamp is weaker");
            "unknown-rustc".to_owned()
        });

    let normalised = version.replace(['\n', '\r'], "|");
    println!("cargo:rustc-env=KOOCH_RUSTC_IDENT={normalised}");
    println!("cargo:rerun-if-env-changed=RUSTC");
}
