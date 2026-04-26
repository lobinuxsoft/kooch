//! Undo/redo system using the command pattern.
//!
//! Each undoable editor operation is wrapped in an [`EditorCommand`] that
//! captures before-state on construction. The [`UndoStack`] resource holds
//! the command history and provides `undo()`/`redo()` operations.
//!
//! Concrete commands live under [`commands`]; the parent module
//! re-exports them so callers see a flat namespace
//! (`crate::undo::SpawnCommand`, etc).

mod commands;
#[cfg(test)]
mod tests;

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::reflect::ReflectValue;

pub(crate) use commands::{
    AddComponentCommand, DespawnCommand, RemoveComponentCommand, SetFieldCommand, SpawnCommand,
};

/// A snapshot of all reflected field values for a single component on an entity.
#[derive(Debug, Clone)]
pub(crate) struct ComponentSnapshot {
    pub type_id: TypeId,
    pub fields: Vec<(String, ReflectValue)>,
}

// ---------------------------------------------------------------------------
// EditorCommand trait
// ---------------------------------------------------------------------------

/// A reversible editor operation.
pub(crate) trait EditorCommand: Send + Sync {
    /// Apply the command (first time or redo).
    fn execute(&mut self, resources: &mut Resources);
    /// Reverse the command.
    fn undo(&mut self, resources: &mut Resources);
    /// Human-readable description for UI display.
    fn description(&self) -> &str;
}

// ---------------------------------------------------------------------------
// UndoStack resource
// ---------------------------------------------------------------------------

/// History of undoable commands. Registered as a resource in the editor.
pub(crate) struct UndoStack {
    undo_stack: Vec<Box<dyn EditorCommand>>,
    redo_stack: Vec<Box<dyn EditorCommand>>,
    max_history: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history: 100,
        }
    }

    /// Executes a command and pushes it onto the undo stack.
    pub fn execute(&mut self, mut cmd: Box<dyn EditorCommand>, resources: &mut Resources) {
        cmd.execute(resources);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();

        // Trim oldest entries if over capacity.
        while self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Undoes the most recent command.
    pub fn undo(&mut self, resources: &mut Resources) {
        if let Some(mut cmd) = self.undo_stack.pop() {
            cmd.undo(resources);
            self.redo_stack.push(cmd);
        }
    }

    /// Redoes the most recently undone command.
    pub fn redo(&mut self, resources: &mut Resources) {
        if let Some(mut cmd) = self.redo_stack.pop() {
            cmd.execute(resources);
            self.undo_stack.push(cmd);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clears all history (e.g. on scene load).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Returns the description of the command that would be undone next.
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.last().map(|cmd| cmd.description())
    }

    /// Returns the description of the command that would be redone next.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|cmd| cmd.description())
    }
}

// ---------------------------------------------------------------------------
// CompoundCommand — groups multiple commands as a single undo/redo step
// ---------------------------------------------------------------------------

/// Groups multiple commands into a single atomic undo/redo operation.
pub(crate) struct CompoundCommand {
    commands: Vec<Box<dyn EditorCommand>>,
    desc: String,
}

impl CompoundCommand {
    pub fn new(desc: impl Into<String>, commands: Vec<Box<dyn EditorCommand>>) -> Self {
        Self {
            commands,
            desc: desc.into(),
        }
    }
}

impl EditorCommand for CompoundCommand {
    fn execute(&mut self, resources: &mut Resources) {
        for cmd in &mut self.commands {
            cmd.execute(resources);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Undo in reverse order.
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(resources);
        }
    }

    fn description(&self) -> &str {
        &self.desc
    }
}
