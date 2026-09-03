//! What a reload did to a project's component types, and saying it out
//! loud.
//!
//! # Why a report, and not just a log line
//!
//! The engine's standing policy is to break data rather than write
//! migrations — which is only safe while breaking is LOUD. A reload is
//! where that policy is cashed: a field dropped from a struct takes the
//! value off every entity carrying it, and a type renamed makes the
//! component disappear from them entirely. Both are the author's own
//! edit, both are usually intended, and neither produces an error.
//!
//! So the swap names what it did. Silence would be indistinguishable
//! from a reload that changed nothing.

use kooch_ecs::component::DynamicTypeRegistry;
use kooch_ecs::reflect::FieldKind;

/// What changed across a swap of the project's library.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reloaded {
    /// Types the new library declares and the old one did not.
    pub gained: Vec<String>,
    /// Types the old library declared and the new one does not.
    ///
    /// 🔴 The loud one. Instances stay parked under the old name, so the
    /// data is not gone — but nothing will draw it, and a save writes it
    /// back to a type no code claims.
    pub lost: Vec<String>,
    /// Types that survived with a different shape.
    pub changed: Vec<Changed>,
}

/// One type whose fields moved.
#[derive(Debug, PartialEq, Eq)]
pub struct Changed {
    /// Fully qualified name — the identity across the swap.
    pub type_name: String,
    /// Fields the new schema has and the old did not. These start at
    /// their default on every entity that already carried the component.
    pub added: Vec<String>,
    /// Fields the old schema had and the new does not. **The values go
    /// with them.**
    pub removed: Vec<String>,
    /// Fields that kept their name and changed kind — an `f32` that
    /// became a `Vec3`. The old value cannot be carried across.
    pub retyped: Vec<String>,
}

impl Reloaded {
    /// What the new registry says that the old one did not.
    ///
    /// Compared by name in both directions, because a rename is a loss
    /// and a gain rather than a change — the two schemas share no
    /// identity, and pretending otherwise would silently carry values
    /// into a type that never held them.
    pub fn between(before: &DynamicTypeRegistry, after: &DynamicTypeRegistry) -> Self {
        let mut report = Self::default();
        for old in before.iter() {
            match after.get(&old.type_name) {
                None => report.lost.push(old.type_name.clone()),
                Some(new) => {
                    if let Some(changed) = shape(&old.type_name, &old.fields, &new.fields) {
                        report.changed.push(changed);
                    }
                }
            }
        }
        for new in after.iter() {
            if !before.contains(&new.type_name) {
                report.gained.push(new.type_name.clone());
            }
        }
        report.gained.sort();
        report.lost.sort();
        report.changed.sort_by(|a, b| a.type_name.cmp(&b.type_name));
        report
    }

    /// Whether the two libraries declare the same thing.
    ///
    /// A rebuild that only changed a function body lands here, and it is
    /// the common case: the types are identical and there is nothing to
    /// announce.
    pub fn is_quiet(&self) -> bool {
        self.gained.is_empty() && self.lost.is_empty() && self.changed.is_empty()
    }

    /// Writes the report to the log, loudest first.
    pub fn report(&self) {
        for name in &self.lost {
            tracing::warn!(
                component = %name,
                "no longer declared — entities still carry one, and nothing will draw it",
            );
        }
        for change in &self.changed {
            if !change.removed.is_empty() {
                tracing::warn!(
                    component = %change.type_name,
                    fields = %change.removed.join(", "),
                    "fields are gone, and so are their values",
                );
            }
            if !change.retyped.is_empty() {
                tracing::warn!(
                    component = %change.type_name,
                    fields = %change.retyped.join(", "),
                    "fields changed kind — the old values could not be carried across",
                );
            }
            if !change.added.is_empty() {
                tracing::info!(
                    component = %change.type_name,
                    fields = %change.added.join(", "),
                    "new fields, starting at their default",
                );
            }
        }
        if !self.gained.is_empty() {
            tracing::info!(components = %self.gained.join(", "), "newly declared");
        }
    }
}

/// How one type's fields moved, or `None` when they did not.
fn shape(
    type_name: &str,
    before: &[kooch_ecs::component::DynamicField],
    after: &[kooch_ecs::component::DynamicField],
) -> Option<Changed> {
    let kind_of =
        |fields: &[kooch_ecs::component::DynamicField], name: &str| -> Option<FieldKind> {
            fields.iter().find(|f| f.name == name).map(|f| f.kind)
        };
    let mut changed = Changed {
        type_name: type_name.to_owned(),
        added: Vec::new(),
        removed: Vec::new(),
        retyped: Vec::new(),
    };
    for old in before {
        match kind_of(after, &old.name) {
            None => changed.removed.push(old.name.clone()),
            Some(kind) if kind != old.kind => changed.retyped.push(old.name.clone()),
            Some(_) => {}
        }
    }
    for new in after {
        if kind_of(before, &new.name).is_none() {
            changed.added.push(new.name.clone());
        }
    }
    let quiet =
        changed.added.is_empty() && changed.removed.is_empty() && changed.retyped.is_empty();
    (!quiet).then_some(changed)
}

#[cfg(test)]
mod tests;
