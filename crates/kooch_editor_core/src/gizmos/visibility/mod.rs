//! [`GizmoVisibility`] — which gizmos draw, by category and by component.

mod menu;
mod persistence;
#[cfg(test)]
mod tests;

use std::any::TypeId;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Which gizmo groups are hidden.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GizmoVisibility {
    /// Master switch. `false` hides everything without disturbing the
    /// per-group state below, so flipping it back restores exactly what
    /// was set.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// The world grid on the ground plane.
    #[serde(default = "enabled")]
    pub grid: bool,
    /// Categories the user turned off, by their reflected name
    /// (`"Physics"`, `"Rendering"`, …).
    #[serde(default)]
    hidden_categories: HashSet<String>,
    /// Individual components turned off, by full type name.
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
            grid: true,
            hidden_categories: HashSet::new(),
            hidden_components: HashSet::new(),
        }
    }

    /// Whether a component's gizmo should draw.
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

/// One row for the Gizmos panel: a registered visualizer's component, grouped under its category.
#[derive(Debug, Clone)]
pub struct GizmoGroup {
    /// Reflected category, or `None` for a component without one.
    pub category: Option<String>,
    /// `(full type name, short name)` per component, sorted by short name.
    pub components: Vec<(String, String)>,
}

/// Groups the registered visualizers by reflected category.
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
