//! What a loaded project declared to the editor: its actions and its data asset types.

use kooch_plugin_api::{DataAssetSchema, EditorHandler};

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

/// The data asset types a loaded project declared. Plain data: the types stay in the project.
#[derive(Default)]
pub struct DataAssetTypes {
    types: Vec<DataAssetSchema>,
}

impl DataAssetTypes {
    /// Adds a type, replacing one of the same name.
    pub fn add(&mut self, schema: DataAssetSchema) {
        self.types.retain(|own| own.type_name != schema.type_name);
        self.types.push(schema);
    }

    /// The type called `type_name`.
    pub fn get(&self, type_name: &str) -> Option<&DataAssetSchema> {
        self.types.iter().find(|own| own.type_name == type_name)
    }

    /// Every declared type.
    pub fn all(&self) -> &[DataAssetSchema] {
        &self.types
    }
}
