//! Creates a project with the current scaffold, for smoke-testing the
//! editor's plugin path without clicking through the New Project form.
//!
//! ```text
//! cargo run --example make_project --features editor -- <parent-dir>
//! ```
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let parent = std::path::Path::new(args.get(1).map(String::as_str).unwrap_or("."));
    let engine = std::env::current_dir().expect("cwd");
    let name = args.get(2).map(String::as_str).unwrap_or("HealthDemo");

    match oh_my_engine::ome_editor_core::project::create_project(name, parent, &engine) {
        Ok(path) => println!("CREATED {}", path.display()),
        Err(e) => {
            eprintln!("ERROR {e}");
            std::process::exit(1);
        }
    }
}
