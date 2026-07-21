//! Storage for components whose Rust type this binary does not know.
//!
//! A scene references components by fully-qualified type name. Whether
//! that name resolves to a real type depends on *which binary* opened
//! the scene: a project's own editor build knows its gameplay
//! components, the standalone hub never will, and neither can be fixed
//! by reflection — Rust resolves types at compile time.
//!
//! Rather than fail the load (and discard the data on the next save),
//! unresolved components are parked here verbatim and written back out
//! untouched. A scene therefore survives a round-trip through a binary
//! that only understands half of it, which is what makes it safe to
//! open any project from the hub.
//!
//! This is also the substrate the remote editor client mirrors into:
//! every project component is "unknown" to it by construction.
//!
//! Storage is SoA — entries are appended on load, scanned on save, and
//! rarely mutated in between, so parallel arrays beat a map of owned
//! rows. Type names are interned because a scene typically has few
//! distinct unknown types spread over many entities.

use crate::entity::Entity;
use crate::reflect::ReflectValue;

/// Components parked by type name because no Rust type matched.
///
/// Lives as a resource. Entries are keyed by [`Entity`], so they follow
/// the entity across a save/load cycle without needing a component slot
/// in an archetype.
#[derive(Debug, Default, Clone)]
pub struct DynamicComponents {
    /// Owning entity, one per entry.
    entities: Vec<Entity>,
    /// Index into [`Self::names`], one per entry.
    name_indices: Vec<u32>,
    /// Reflected field values, one per entry.
    fields: Vec<Vec<(String, ReflectValue)>>,
    /// Interned fully-qualified type names.
    names: Vec<String>,
}

impl DynamicComponents {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parks a component under `entity`.
    ///
    /// Replaces the fields if this entity already carries a component of
    /// the same type, so a repeated load is idempotent rather than
    /// accumulating duplicates.
    pub fn insert(&mut self, entity: Entity, type_name: &str, fields: Vec<(String, ReflectValue)>) {
        let name_index = self.intern(type_name);
        match self.position(entity, name_index) {
            Some(i) => self.fields[i] = fields,
            None => {
                self.entities.push(entity);
                self.name_indices.push(name_index);
                self.fields.push(fields);
            }
        }
    }

    /// Components parked under `entity`, as `(type_name, fields)`.
    pub fn iter_entity(
        &self,
        entity: Entity,
    ) -> impl Iterator<Item = (&str, &[(String, ReflectValue)])> {
        self.entities
            .iter()
            .enumerate()
            .filter(move |(_, e)| **e == entity)
            .map(|(i, _)| {
                (
                    self.names[self.name_indices[i] as usize].as_str(),
                    self.fields[i].as_slice(),
                )
            })
    }

    /// Every distinct type name parked in the store.
    ///
    /// Lets a name-keyed consumer (the editor's component interner) see
    /// types no local registry knows about.
    pub fn type_names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Overwrites one field of a parked component.
    ///
    /// Returns `false` when the entity has no such component, or the
    /// component has no such field — a parked component has no schema of
    /// its own, so fields are never created on the fly.
    pub fn set_field(
        &mut self,
        entity: Entity,
        type_name: &str,
        field: &str,
        value: ReflectValue,
    ) -> bool {
        let Some(name_index) = self.names.iter().position(|n| n == type_name) else {
            return false;
        };
        let Some(entry) = self.position(entity, name_index as u32) else {
            return false;
        };
        let Some((_, slot)) = self.fields[entry].iter_mut().find(|(k, _)| k == field) else {
            return false;
        };
        *slot = value;
        true
    }

    /// Drops every entry belonging to `entity`.
    pub fn remove_entity(&mut self, entity: Entity) {
        self.retain(|e| e != entity);
    }

    /// Drops entries whose entity fails `keep`. Used to prune entities
    /// that were despawned without going through the editor.
    pub fn retain_entities(&mut self, keep: impl Fn(Entity) -> bool) {
        self.retain(|e| keep(e));
    }

    /// Removes every entry, keeping interned names allocated.
    pub fn clear(&mut self) {
        self.entities.clear();
        self.name_indices.clear();
        self.fields.clear();
    }

    /// `true` when nothing is parked.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Number of parked components across all entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Interns `type_name`, returning its index.
    fn intern(&mut self, type_name: &str) -> u32 {
        match self.names.iter().position(|n| n == type_name) {
            Some(i) => i as u32,
            None => {
                self.names.push(type_name.to_owned());
                (self.names.len() - 1) as u32
            }
        }
    }

    /// Entry index for `(entity, name_index)`, if present.
    fn position(&self, entity: Entity, name_index: u32) -> Option<usize> {
        self.entities
            .iter()
            .zip(&self.name_indices)
            .position(|(e, n)| *e == entity && *n == name_index)
    }

    /// Retains entries whose entity satisfies `keep`, holding the
    /// parallel arrays in step.
    ///
    /// The mask is materialised first: `Vec::retain` cannot drive three
    /// arrays at once, and re-evaluating `keep` per array would desync
    /// them the moment it is not a pure function of the entity.
    fn retain(&mut self, keep: impl Fn(Entity) -> bool) {
        let mask: Vec<bool> = self.entities.iter().map(|e| keep(*e)).collect();
        retain_masked(&mut self.entities, &mask);
        retain_masked(&mut self.name_indices, &mask);
        retain_masked(&mut self.fields, &mask);
    }
}

/// Drops the elements of `values` whose slot in `mask` is `false`.
fn retain_masked<T>(values: &mut Vec<T>, mask: &[bool]) {
    let mut i = 0;
    values.retain(|_| {
        let keep = mask[i];
        i += 1;
        keep
    });
}
