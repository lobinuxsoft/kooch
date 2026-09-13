//! Who a system belongs to, and what to call it on screen.

/// Which half of the build scheduled a system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemSource {
    /// Scheduled by the engine's own plugins. Switching one off is an
    /// experiment, and can break a frame.
    Engine,
    /// Scheduled by the project — its generated registrations, or its
    /// own `main`. Switching one off is ordinary gameplay control.
    Project,
}

/// What a toggle addresses a system by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SystemKey {
    pub name: String,
    pub nth: u32,
}

impl SystemKey {
    /// The key for the first system with this name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: canonical(&name.into()).to_owned(),
            nth: 0,
        }
    }

    /// The key for the `nth` system sharing a name, counting from zero.
    pub fn nth(name: impl Into<String>, nth: u32) -> Self {
        Self {
            nth,
            ..Self::new(name)
        }
    }
}

impl From<&str> for SystemKey {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

/// The name a key is built from, with any wrapper taken off.
pub fn canonical(name: &str) -> &str {
    innermost(name)
}

/// One scheduled system, as a reader sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemInfo<'a> {
    pub stage: crate::stage::Stage,
    /// The full path `type_name` gave.
    pub name: &'a str,
    /// What a toggle addresses it by.
    pub key: &'a SystemKey,
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
pub(super) fn innermost(name: &str) -> &str {
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
