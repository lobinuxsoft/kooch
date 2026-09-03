//! Does dropping the loader actually unmap the library?
//!
//! The whole reload design turns on this. If `dlclose` is a no-op here —
//! and on glibc it often is, for a library with TLS or one another
//! library still depends on — then "unload" is a word for something that
//! did not happen, and the next load maps a *second* copy while the
//! first keeps running.
//!
//! Measured against a real `.so`, because this is not a property of our
//! code: it is a property of the dynamic loader on this machine.

#![cfg(feature = "dynamic")]

use std::path::PathBuf;

use kooch_core::dynamic::PluginLoader;

fn plugin_path() -> PathBuf {
    let mut dir = std::env::current_exe().expect("test exe path");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    #[cfg(target_os = "windows")]
    let name = "example_plugin.dll";
    #[cfg(target_os = "linux")]
    let name = "libexample_plugin.so";
    #[cfg(target_os = "macos")]
    let name = "libexample_plugin.dylib";
    dir.join(name)
}

/// How many mappings name the plugin right now.
fn mapped(needle: &str) -> usize {
    std::fs::read_to_string("/proc/self/maps")
        .expect("read maps")
        .lines()
        .filter(|line| line.contains(needle))
        .count()
}

#[test]
#[ignore = "measurement"]
fn unload_trace() {
    let path = plugin_path();
    assert!(
        path.exists(),
        "build example_plugin first: {}",
        path.display()
    );

    println!("before load     {}", mapped("example_plugin"));

    {
        let mut loader = PluginLoader::new();
        let plugin = unsafe { loader.load(&path) }.expect("load");
        println!("after load      {}", mapped("example_plugin"));
        drop(plugin);
        println!("after plugin    {}", mapped("example_plugin"));
    }
    println!("after loader    {}", mapped("example_plugin"));

    // A second generation, the way a reload would ask for one.
    {
        let mut loader = PluginLoader::new();
        let plugin = unsafe { loader.load(&path) }.expect("reload");
        println!("after reload    {}", mapped("example_plugin"));
        drop(plugin);
    }
    println!("after both      {}", mapped("example_plugin"));
}

/// The same question against a **project's** library rather than the
/// 5 MB example: `KOOCH_UNLOAD_LIB=/path/to/libgame.so`.
///
/// 🔴 Size and dependency count are exactly what make `dlclose` refuse.
/// glibc keeps a library mapped when another loaded object still depends
/// on it, and a project's `.so` links the whole engine — so the small
/// case proving clean says nothing about the real one.
///
/// Opened with `libloading` directly, not through `PluginLoader`: the
/// stamp would refuse a library built against another engine version,
/// and mapping is the only thing being measured.
#[test]
#[ignore = "measurement"]
fn project_unload_trace() {
    let Ok(path) = std::env::var("KOOCH_UNLOAD_LIB") else {
        println!("set KOOCH_UNLOAD_LIB to a project .so");
        return;
    };
    let path = PathBuf::from(path);
    let needle = path.file_name().expect("file name").to_str().expect("utf8");
    let bytes = std::fs::metadata(&path).expect("stat").len();
    println!("{needle}  {:.0} MB", bytes as f64 / 1e6);

    println!("before load     {}", mapped(needle));
    let opened = std::time::Instant::now();
    let library = unsafe { libloading::Library::new(&path) }.expect("open");
    let took = opened.elapsed();
    println!(
        "after load      {}   ({:.0} ms)",
        mapped(needle),
        took.as_secs_f64() * 1e3
    );

    let closed = std::time::Instant::now();
    drop(library);
    println!(
        "after drop      {}   ({:.0} ms)",
        mapped(needle),
        closed.elapsed().as_secs_f64() * 1e3,
    );

    let again = std::time::Instant::now();
    let library = unsafe { libloading::Library::new(&path) }.expect("reopen");
    println!(
        "after reload    {}   ({:.0} ms)",
        mapped(needle),
        again.elapsed().as_secs_f64() * 1e3
    );
    drop(library);
    println!("after both      {}", mapped(needle));
}
