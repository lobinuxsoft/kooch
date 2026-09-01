//! Who a system belongs to, and what to call it on screen.

/// Which half of the build scheduled a system.
///
/// Recorded when the system is added rather than read off its name later.
/// A plugin knows which side it is on; a crate-name prefix only looks
/// like it does, and the log console already made and undid that mistake
/// with its `[game] ` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemSource {
    /// Scheduled by the engine's own plugins. Switching one off is an
    /// experiment, and can break a frame.
    Engine,
    /// Scheduled by the project — its generated registrations, or its
    /// own `main`. Switching one off is ordinary gameplay control.
    Project,
}

/// One scheduled system, as a reader sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemInfo<'a> {
    pub stage: crate::stage::Stage,
    /// The full path `type_name` gave, which is what a toggle keys on.
    pub name: &'a str,
    pub source: SystemSource,
    pub gpu: bool,
}

impl SystemInfo<'_> {
    /// The name to put in a list, without the paths and the wrapper.
    pub fn short_name(&self) -> &str {
        short_name(self.name)
    }
}

/// Trims a `type_name` down to the part a person recognises.
///
/// A project system is wrapped before it is scheduled, so what arrives is
/// `run_if_playing<a::b::read_player_input>::{{closure}}`. The real path
/// is inside the generic argument, so unwrapping is reading, not
/// plumbing — measured on the actual output, not assumed.
///
/// A genuine closure keeps its owning module: trimming
/// `assets::{{closure}}` to `{{closure}}` would name every one of them
/// the same thing.
pub fn short_name(name: &str) -> &str {
    let inner = innermost(name);
    match inner.ends_with("::{{closure}}") {
        // Keep the module above it, or every closure in the build reads
        // as the same anonymous thing.
        true => last_two(inner),
        // A plain path: the last segment is the function.
        false => inner.rsplit("::").next().unwrap_or(inner),
    }
}

/// The deepest generic argument, or the whole string when there is none.
///
/// `run_if_playing<a::b::sys>::{{closure}}` → `a::b::sys`. Wrappers nest,
/// so this recurses rather than peeling one layer.
fn innermost(name: &str) -> &str {
    let Some(open) = name.find('<') else {
        return name;
    };
    let Some(close) = name.rfind('>') else {
        return name;
    };
    if close <= open + 1 {
        return name;
    }
    innermost(&name[open + 1..close])
}

/// The last two `::` segments, joined as they were.
fn last_two(path: &str) -> &str {
    let mut parts = path.rmatch_indices("::");
    parts.next();
    match parts.next() {
        Some((at, _)) => &path[at + 2..],
        None => path,
    }
}

#[cfg(test)]
mod tests;
