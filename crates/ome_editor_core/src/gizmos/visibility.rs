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

use std::any::TypeId;
use std::collections::HashSet;

use ome_core::resource::Resources;
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
/// [`VisualizerRegistry`]: ome_gizmos::VisualizerRegistry
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

#[cfg(test)]
mod tests {
    use super::*;

    const COLLIDER: &str = "ome_physics::components::Collider";
    const CAMERA: &str = "ome_ecs::perspective_camera::PerspectiveCamera";

    #[test]
    fn everything_draws_by_default() {
        let v = GizmoVisibility::new();
        assert!(v.draws(COLLIDER, Some("Physics")));
        assert!(v.draws(CAMERA, Some("Rendering")));
        assert!(v.draws("game::Thing", None));
        assert!(!v.has_exceptions());
    }

    /// A component with no entry draws. That is what lets a visualizer
    /// added later appear without registering anything here, and without
    /// migrating anyone's saved settings.
    #[test]
    fn an_unknown_component_draws() {
        let mut v = GizmoVisibility::new();
        v.set_category("Physics", false);
        assert!(
            v.draws("brand::New::Component", Some("Lighting")),
            "an unrelated new component was hidden"
        );
    }

    #[test]
    fn hiding_a_category_hides_only_its_own() {
        let mut v = GizmoVisibility::new();
        v.set_category("Physics", false);

        assert!(!v.draws(COLLIDER, Some("Physics")));
        assert!(v.draws(CAMERA, Some("Rendering")));
        assert!(v.has_exceptions());
    }

    #[test]
    fn hiding_one_component_leaves_its_category_alone() {
        let mut v = GizmoVisibility::new();
        v.set_component(COLLIDER, false);

        assert!(!v.draws(COLLIDER, Some("Physics")));
        assert!(
            v.draws("ome_physics::components::RigidBody", Some("Physics")),
            "hiding one component hid its whole category"
        );
        assert!(v.category_visible("Physics"));
    }

    /// The master switch hides everything and restores exactly what was
    /// set — the point of keeping it separate from the per-group state.
    #[test]
    fn the_master_switch_preserves_the_per_group_state() {
        let mut v = GizmoVisibility::new();
        v.set_category("Physics", false);

        v.enabled = false;
        assert!(!v.draws(CAMERA, Some("Rendering")));
        assert!(!v.draws(COLLIDER, Some("Physics")));

        v.enabled = true;
        assert!(v.draws(CAMERA, Some("Rendering")), "restoring lost a group");
        assert!(
            !v.draws(COLLIDER, Some("Physics")),
            "restoring forgot the hidden category"
        );
    }

    #[test]
    fn show_all_clears_every_exception() {
        let mut v = GizmoVisibility::new();
        v.set_category("Physics", false);
        v.set_component(CAMERA, false);
        v.enabled = false;

        v.show_all();

        assert!(v.draws(COLLIDER, Some("Physics")));
        assert!(v.draws(CAMERA, Some("Rendering")));
        assert!(!v.has_exceptions());
    }

    /// Choices have to survive a restart, so they round-trip through the
    /// same format the dock layout uses.
    #[test]
    fn visibility_round_trips_through_ron() {
        let mut v = GizmoVisibility::new();
        v.set_category("Physics", false);
        v.set_component(CAMERA, false);

        let text = ron::ser::to_string(&v).expect("serialise");
        let back: GizmoVisibility = ron::from_str(&text).expect("deserialise");

        assert!(!back.draws(COLLIDER, Some("Physics")));
        assert!(!back.draws(CAMERA, Some("Rendering")));
        assert!(back.draws("ome_ecs::point_light::PointLight", Some("Lighting")));
    }

    /// An older saved layout has no `enabled` field; it must read as on
    /// rather than hiding every gizmo the user had.
    #[test]
    fn an_older_saved_layout_reads_as_visible() {
        let back: GizmoVisibility =
            ron::from_str("(hidden_categories: [], hidden_components: [])").expect("deserialise");
        assert!(back.enabled, "a layout without the field hid everything");
        assert!(back.draws(COLLIDER, Some("Physics")));
    }

    #[test]
    fn grouping_sorts_categories_and_leaves_uncategorised_last() {
        let groups = group_visualizers([
            (TypeId::of::<u8>(), "game::Thing".to_owned(), None),
            (
                TypeId::of::<u16>(),
                COLLIDER.to_owned(),
                Some("Physics".to_owned()),
            ),
            (
                TypeId::of::<u32>(),
                CAMERA.to_owned(),
                Some("Rendering".to_owned()),
            ),
            (
                TypeId::of::<u64>(),
                "ome_physics::components::RigidBody".to_owned(),
                Some("Physics".to_owned()),
            ),
        ]);

        let names: Vec<Option<&str>> = groups.iter().map(|g| g.category.as_deref()).collect();
        assert_eq!(names, vec![Some("Physics"), Some("Rendering"), None]);

        // Physics holds both of its components, sorted by short name.
        let physics = &groups[0].components;
        assert_eq!(
            physics.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            vec!["Collider", "RigidBody"]
        );
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Where the choices live, next to the dock layout.
///
/// Its own file rather than a field inside `editor_layout.ron`: the layout
/// is rewritten whenever a panel moves, and folding an unrelated setting
/// into it would mean a dragged splitter and a hidden gizmo group sharing
/// one write path — and one corrupt file losing both.
pub(crate) fn visibility_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("ome").join("gizmo_visibility.ron"))
}

/// Reads the saved choices. Missing file or unparseable content both mean
/// "everything visible" — a preference is not worth failing a launch over.
pub(crate) fn load() -> GizmoVisibility {
    let Some(path) = visibility_path() else {
        return GizmoVisibility::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(
                target: "ome_editor_core::gizmos::visibility",
                path = %path.display(),
                error = %e,
                "unreadable gizmo visibility file; showing everything",
            );
            GizmoVisibility::new()
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    target: "ome_editor_core::gizmos::visibility",
                    path = %path.display(),
                    error = %e,
                    "could not read gizmo visibility",
                );
            }
            GizmoVisibility::new()
        }
    }
}

/// Startup system: pulls the saved choices into `Resources`.
pub(crate) fn load_visibility_system(resources: &mut Resources) {
    let loaded = load();
    resources.insert(VisibilityPersistence {
        last_serialized: ron::ser::to_string(&loaded).ok(),
    });
    resources.insert(loaded);
}

/// Cache of what is already on disk, so a steady-state frame writes
/// nothing. Same shape as `LayoutPersistence`.
#[derive(Default)]
pub(crate) struct VisibilityPersistence {
    last_serialized: Option<String>,
}

/// End-of-frame system: writes the choices when they actually changed.
pub(crate) fn save_visibility_system(resources: &mut Resources) {
    let Some(serialized) = resources
        .get::<GizmoVisibility>()
        .and_then(|v| ron::ser::to_string(v).ok())
    else {
        return;
    };
    let Some(persist) = resources.get_mut::<VisibilityPersistence>() else {
        return;
    };
    if persist.last_serialized.as_deref() == Some(serialized.as_str()) {
        return;
    }
    persist.last_serialized = Some(serialized.clone());

    let Some(path) = visibility_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "ome_editor_core::gizmos::visibility",
            error = %e,
            "could not create the config directory",
        );
        return;
    }
    if let Err(e) = std::fs::write(&path, serialized) {
        tracing::warn!(
            target: "ome_editor_core::gizmos::visibility",
            path = %path.display(),
            error = %e,
            "could not save gizmo visibility",
        );
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

/// Draws the Gizmos dropdown: a master switch, then a checkbox per
/// category, then the components inside each.
///
/// Three levels because "hide all physics" and "hide only the colliders"
/// are both things you want, and a flat list is the part of Unity's own
/// panel people complain about.
///
/// `groups` comes from [`group_visualizers`], so the menu lists whatever is
/// registered — a visualizer added later appears here with no change.
pub(crate) fn draw_gizmo_menu(
    ui: &mut egui::Ui,
    visibility: &mut GizmoVisibility,
    groups: &[GizmoGroup],
) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut visibility.enabled, "Gizmos")
            .on_hover_text("Master switch — leaves the per-group choices below untouched");
        if ui
            .add_enabled(visibility.has_exceptions(), egui::Button::new("Reset"))
            .on_hover_text("Show everything again")
            .clicked()
        {
            visibility.show_all();
        }
    });
    ui.separator();

    // Greyed out rather than hidden while the master switch is off: the
    // per-group state still exists and is about to matter again, and
    // hiding the rows would suggest it had been lost.
    ui.add_enabled_ui(visibility.enabled, |ui| {
        if groups.is_empty() {
            ui.label("No gizmos registered");
            return;
        }
        for group in groups {
            match &group.category {
                Some(category) => {
                    let mut on = visibility.category_visible(category);
                    if ui.checkbox(&mut on, category).changed() {
                        visibility.set_category(category, on);
                    }
                    // A component inside a hidden category cannot draw, so
                    // its own switch is inert until the category is back.
                    ui.add_enabled_ui(on, |ui| {
                        component_rows(ui, visibility, &group.components);
                    });
                }
                None => {
                    ui.label("Uncategorised");
                    component_rows(ui, visibility, &group.components);
                }
            }
            ui.separator();
        }
    });
}

/// The indented per-component checkboxes under one category.
fn component_rows(
    ui: &mut egui::Ui,
    visibility: &mut GizmoVisibility,
    components: &[(String, String)],
) {
    for (type_name, short) in components {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let mut on = visibility.component_visible(type_name);
            if ui
                .checkbox(&mut on, short)
                .on_hover_text(type_name)
                .changed()
            {
                visibility.set_component(type_name, on);
            }
        });
    }
}

/// Groups the visualizers registered in `resources`, resolving each
/// component's reflected category.
pub(crate) fn groups_from_resources(resources: &Resources) -> Vec<GizmoGroup> {
    let Some(registry) = resources.get::<ome_gizmos::VisualizerRegistry>() else {
        return Vec::new();
    };
    let components = resources.get::<ome_ecs::component::ComponentRegistry>();
    group_visualizers(registry.registered_types().filter_map(|type_id| {
        let components = components.as_ref()?;
        // A visualizer for a component the registry has never seen has no
        // name to persist and no category to group under; skipping it is
        // better than inventing either.
        let name = components.component_name(&type_id)?;
        Some((
            type_id,
            name.to_owned(),
            components.reflect_category(&type_id).map(str::to_owned),
        ))
    }))
}
