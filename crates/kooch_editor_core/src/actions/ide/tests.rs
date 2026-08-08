use super::*;

/// The real file from a Homebrew VSCodium install, which is what
/// started this: the binary is not on the PATH and the desktop entry
/// is the only place its location is written down.
const HOMEBREW_CODIUM: &str = "\
[Desktop Entry]
Name=VSCodium
Exec=/home/linuxbrew/.linuxbrew/bin/codium %F
Type=Application

[Desktop Action new-empty-window]
Name=New Empty Window
Exec=/home/linuxbrew/.linuxbrew/bin/codium --new-window %F
";

#[test]
fn the_exec_line_gives_the_full_path_the_path_did_not_have() {
    let command = parse_exec(HOMEBREW_CODIUM).expect("an Exec line");
    assert_eq!(command.program, "/home/linuxbrew/.linuxbrew/bin/codium");
    assert!(
        command.args.is_empty(),
        "the %F placeholder must be dropped"
    );
}

/// A desktop action's `Exec` opens something other than what
/// double-clicking does — here, an empty window.
#[test]
fn a_desktop_action_is_not_mistaken_for_the_entry() {
    let command = parse_exec(HOMEBREW_CODIUM).expect("an Exec line");
    assert!(
        !command.args.iter().any(|arg| arg == "--new-window"),
        "took the Exec from [Desktop Action], which opens an empty window"
    );
}

#[test]
fn a_flatpak_exec_keeps_the_arguments_it_needs() {
    let command = parse_exec(
        "[Desktop Entry]\nExec=/usr/bin/flatpak run --branch=stable com.vscodium.codium %U\n",
    )
    .expect("an Exec line");
    assert_eq!(command.program, "/usr/bin/flatpak");
    assert_eq!(
        command.args,
        vec!["run", "--branch=stable", "com.vscodium.codium"]
    );
}

#[test]
fn a_flatpak_vscodium_still_understands_goto() {
    let command = parse_exec("[Desktop Entry]\nExec=flatpak run com.vscodium.codium %U\n").unwrap();
    assert!(command.understands_goto());
}

#[test]
fn a_full_path_to_codium_understands_goto() {
    let command = IdeCommand::parse("/home/linuxbrew/.linuxbrew/bin/codium").unwrap();
    assert!(
        command.understands_goto(),
        "the flag is decided by the program name, not by the directory it sits in"
    );
}

/// A plain text editor does not know `-g`, and would create a file
/// called `-g` if handed one.
#[test]
fn a_generic_editor_does_not_claim_goto() {
    assert!(!IdeCommand::parse("kate").unwrap().understands_goto());
    assert!(
        !IdeCommand::parse("gnome-text-editor")
            .unwrap()
            .understands_goto()
    );
}

/// A real entry, and the one that caught this: the spec allows
/// quoting and Antigravity uses it.
#[test]
fn a_quoted_exec_yields_a_runnable_path() {
    let command = parse_exec(
        "[Desktop Entry]\nExec=\"/home/me/.local/share/antigravity/antigravity-ide\" %F\n",
    )
    .expect("an Exec line");
    assert_eq!(
        command.program, "/home/me/.local/share/antigravity/antigravity-ide",
        "the quotes must go, or the OS looks for a program called '\"/home/…'"
    );
}

/// What someone does when they copy the path out of a `.desktop`.
#[test]
fn a_hand_typed_quoted_path_is_still_runnable() {
    let command =
        IdeCommand::parse("\"/home/me/.local/share/antigravity/antigravity-ide\"").unwrap();
    assert_eq!(
        command.program, "/home/me/.local/share/antigravity/antigravity-ide",
        "quotes typed into Settings must not reach the OS"
    );
}

#[test]
fn a_file_with_no_exec_yields_nothing() {
    assert!(parse_exec("[Desktop Entry]\nName=Nothing\n").is_none());
}
