//! [`GizmoVisibility`] — which gizmos draw, by category and by component.
//!
//! # What this replaces
//!
//! A visualizer used to draw only while its Inspector header was expanded.
//! For a camera frustum that was tolerable; for a collider it was a trap —
//! you select the body, see no outline, and conclude the gizmo is broken.
//! It also coupled display to unrelated UI state: whether a header happens
//! to be open decided whether you could see the geometry, and expanding a
//! header is a reading gesture, not a "show me the shape" gesture.
//!
//! # Why the other half is not optional
//!
//! Making everything always-visible without a way to hide it trades one
//! annoyance for a worse one: a scene with a hundred selected bodies is a
//! wall of green. So visibility is explicit, three levels deep — global,
//! per category, per component — because "hide all physics" and "hide only
//! the sensors" are both things you want.
//!
//! # Absent means visible
//!
//! Nothing is enumerated up front. A category or component with no entry
//! draws, so a visualizer added later needs no registration here and no
//! migration of anyone's saved settings. Only an explicit *off* is stored.
//!
//! Split three ways: the model of what is visible, where that survives a
//! restart, and the menu that edits it. The renderer only reads the
//! first.

mod menu;
mod persistence;
#[cfg(test)]
mod tests;

use std::any::TypeId;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Which gizmo groups are hidden.
///
/// Stores only the exceptions — see the module docs on why absent means
/// visible. Serialised alongside the dock layout, so it is a per-project
/// preference rather than something to re-set every launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GizmoVisibility {
    /// Master switch. `false` hides everything without disturbing the
    /// per-group state below, so flipping it back restores exactly what
    /// was set.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Categories the user turned off, by their reflected name
    /// (`"Physics"`, `"Rendering"`, …).
    #[serde(default)]
    hidden_categories: HashSet<String>,
    /// Individual components turned off, by full type name.
    ///
    /// Type *names* rather than `TypeId`s because this is persisted, and a
    /// `TypeId` is only meaningful inside the process that produced it.
    #[serde(default)]
    hidden_components: HashSet<String>,
}

/// `serde` default for [`GizmoVisibility::enabled`] — an older saved
/// layout with no entry means gizmos were on.
fn enabled() -> bool {
    true
}

impl GizmoVisibility {
    /// Everything visible.
    pub fn new() -> Self {
        Self {
            enabled: true,
            hidden_categories: HashSet::new(),
            hidden_components: HashSet::new(),
        }
    }

    /// Whether a component's gizmo should draw.
    ///
    /// `category` is the component's reflected category, if it has one.
    /// An uncategorised component can only be hidden individually — there
    /// is no "Uncategorised" group to switch off, because a component
    /// landing there is usually an oversight rather than a choice.
    pub fn draws(&self, type_name: &str, category: Option<&str>) -> bool {
        if !self.enabled {
            return false;
        }
        if self.hidden_components.contains(type_name) {
            return false;
        }
        match category {
            Some(category) => !self.hidden_categories.contains(category),
            None => true,
        }
    }

    /// Whether a whole category is on.
    pub fn category_visible(&self, category: &str) -> bool {
        !self.hidden_categories.contains(category)
    }

    /// Whether a component is on, ignoring its category.
    ///
    /// The panel needs this to render a checkbox that reflects the
    /// component's *own* state: a component inside a hidden category is
    /// not drawn, but its own switch is still whatever the user left it.
    pub fn component_visible(&self, type_name: &str) -> bool {
        !self.hidden_components.contains(type_name)
    }

    /// Turns a category on or off.
    pub fn set_category(&mut self, category: &str, visible: bool) {
        if visible {
            self.hidden_categories.remove(category);
        } else {
            self.hidden_categories.insert(category.to_owned());
        }
    }

    /// Turns a single component on or off.
    pub fn set_component(&mut self, type_name: &str, visible: bool) {
        if visible {
            self.hidden_components.remove(type_name);
        } else {
            self.hidden_components.insert(type_name.to_owned());
        }
    }

    /// Clears every exception — everything visible again.
    pub fn show_all(&mut self) {
        self.hidden_categories.clear();
        self.hidden_components.clear();
        self.enabled = true;
    }

    /// `true` when something is hidden, so the panel button can show that
    /// gizmos are filtered without the user opening it.
    pub fn has_exceptions(&self) -> bool {
        !self.enabled || !self.hidden_categories.is_empty() || !self.hidden_components.is_empty()
    }
}

/// One row for the Gizmos panel: a registered visualizer's component,
/// grouped under its category.
///
/// Built from the [`VisualizerRegistry`] rather than a hand-maintained
/// list, so a visualizer added later appears with no change here.
///
/// [`VisualizerRegistry`]: kooch_gizmos::VisualizerRegistry
#[derive(Debug, Clone)]
pub struct GizmoGroup {
    /// Reflected category, or `None` for a component without one.
    pub category: Option<String>,
    /// `(full type name, short name)` per component, sorted by short name.
    pub components: Vec<(String, String)>,
}

/// Groups the registered visualizers by reflected category.
///
/// Takes the registry's types plus a resolver rather than `Resources`, so
/// the grouping is testable without standing up an ECS.
pub fn group_visualizers<I>(types: I) -> Vec<GizmoGroup>
where
    I: IntoIterator<Item = (TypeId, String, Option<String>)>,
{
    let mut groups: Vec<GizmoGroup> = Vec::new();
    for (_, type_name, category) in types {
        let short = type_name
            .rsplit("::")
            .next()
            .unwrap_or(&type_name)
            .to_owned();
        let entry = (type_name, short);
        match groups.iter_mut().find(|g| g.category == category) {
            Some(group) => group.components.push(entry),
            None => groups.push(GizmoGroup {
                category,
                components: vec![entry],
            }),
        }
    }
    // Uncategorised last: it is the bucket for oversights, and it should
    // not sit above the groups people actually reach for.
    groups.sort_by(|a, b| match (&a.category, &b.category) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    for group in &mut groups {
        group.components.sort_by(|a, b| a.1.cmp(&b.1));
    }
    groups
}

pub(crate) use menu::{draw_gizmo_menu, groups_from_resources};
// `VisibilityPersistence` and `visibility_path` stay inside
// `persistence` — nothing outside this module ever named them, and the
// split made that visible.
pub(crate) use persistence::{load_visibility_system, save_visibility_system};
