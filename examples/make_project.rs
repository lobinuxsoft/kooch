//! Creates a project with the current scaffold, for smoke-testing the
//! editor's plugin path without clicking through the New Project form.
//!
//! ```text
//! cargo run --example make_project --features editor -- <parent-dir> [name] [engine-root]
//! ```
//!
//! `engine-root` defaults to the current directory, which means the
//! engine's own tree and therefore the development path: the manifest
//! points at the live clone and nothing is copied.
//!
//! Pass a **packaged** engine (`dist/engine`, from `package_editor`) to
//! exercise what an installed editor does instead — vendor the engine
//! into the project and write a relative path (#754). That is the only
//! way to reach that branch without clicking through the New Project
//! form of a GUI.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let parent = std::path::Path::new(args.get(1).map(String::as_str).unwrap_or("."));
    let engine = args
        .get(3)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let name = args.get(2).map(String::as_str).unwrap_or("HealthDemo");

    match kooch::kooch_editor_core::project::create_project(name, parent, &engine) {
        Ok(path) => println!("CREATED {}", path.display()),
        Err(e) => {
            eprintln!("ERROR {e}");
            std::process::exit(1);
        }
    }
}
