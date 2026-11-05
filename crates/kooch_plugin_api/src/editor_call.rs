//! What a project's editor action sees and asks for: plain data, because the project and the editor
//! build the engine's crates with different features and their types need not share a layout.

/// A project function a panel's `on:click` runs in the editor.
pub type EditorHandler = fn(&mut EditorCall);

/// A project's data asset type, described for an editor that cannot name it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataAssetSchema {
    /// Fully qualified type name, what the file's `.meta` records.
    pub type_name: String,
    /// File extension, without the dot.
    pub extension: String,
    /// A fresh file's text: its fields, their kinds and the type's defaults.
    pub defaults: String,
}

/// One selected entity and its numeric fields, keyed `Type.field`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectedEntity {
    /// Packed handle (see [`pack_entity`](crate::types::pack_entity)).
    pub entity: u64,
    /// Each number and bool field as the editor last saw it; a bool is 0 or 1.
    pub numbers: Vec<(String, f32)>,
}

/// What an action asked of the editor, applied after it returns.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorRequest {
    /// Writes a number or bool field, undoable.
    SetNumber {
        /// Packed handle.
        entity: u64,
        /// `Type.field`.
        field: String,
        /// The new value; a bool is true above 0.5.
        value: f32,
    },
    /// Writes a text file.
    WriteAsset {
        /// Relative to the project's `assets/`.
        path: String,
        /// The whole file.
        text: String,
    },
}

/// One run of an action.
#[derive(Debug, Default)]
pub struct EditorCall {
    selected: Vec<SelectedEntity>,
    requests: Vec<EditorRequest>,
}

impl EditorCall {
    /// A call over `selected`; the editor builds it.
    pub fn new(selected: Vec<SelectedEntity>) -> Self {
        Self {
            selected,
            requests: Vec::new(),
        }
    }

    /// The selected entities, as packed handles.
    pub fn selected(&self) -> Vec<u64> {
        self.selected.iter().map(|own| own.entity).collect()
    }

    /// `Type.field` on a selected entity, as the editor last saw it.
    pub fn number(&self, entity: u64, field: &str) -> Option<f32> {
        let own = self.selected.iter().find(|own| own.entity == entity)?;
        let found = own.numbers.iter().find(|(name, _)| name == field)?;
        Some(found.1)
    }

    /// Asks for `Type.field` on a selected entity to become `value`.
    pub fn set_number(&mut self, entity: u64, field: &str, value: f32) {
        self.requests.push(EditorRequest::SetNumber {
            entity,
            field: field.to_owned(),
            value,
        });
    }

    /// Asks for a text file at `path` under the project's `assets/`, replacing what is there.
    pub fn write_asset(&mut self, path: &str, text: impl Into<String>) {
        self.requests.push(EditorRequest::WriteAsset {
            path: path.to_owned(),
            text: text.into(),
        });
    }

    /// What the action asked for, in order; the editor reads it after the action returns.
    pub fn take_requests(&mut self) -> Vec<EditorRequest> {
        std::mem::take(&mut self.requests)
    }
}
