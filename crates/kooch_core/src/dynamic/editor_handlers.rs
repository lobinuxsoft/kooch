//! The editor actions a loaded project declared, by name.

use kooch_plugin_api::EditorHandler;

/// 🔴 Function pointers into the project's library: empty it before the library unloads.
#[derive(Default)]
pub struct EditorHandlers {
    handlers: Vec<(Box<str>, EditorHandler)>,
}

impl EditorHandlers {
    /// Adds `name`, replacing a handler of the same name.
    pub fn add(&mut self, name: &str, handler: EditorHandler) {
        self.handlers.retain(|(own, _)| own.as_ref() != name);
        self.handlers.push((name.into(), handler));
    }

    /// The handler called `name`.
    pub fn get(&self, name: &str) -> Option<EditorHandler> {
        let found = self.handlers.iter().find(|(own, _)| own.as_ref() == name)?;
        Some(found.1)
    }
}
