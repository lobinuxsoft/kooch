//! Reparenting, the IDE command, the power profile, and cancelling a
//! launch — the handlers that do not belong to a larger subsystem.

use kooch_core::power::PowerProfile;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;

use crate::project_state::ProjectState;

pub(super) fn handle_reparent(
    resources: &mut Resources,
    entity: Entity,
    new_parent: Option<Entity>,
) {
    // Moved to kooch_ecs::hierarchy (#595): the server has to be able to
    // perform this too, and while it lived here remote mode had no way to
    // reparent at all.
    kooch_ecs::hierarchy::reparent(resources, entity, new_parent);
}

pub(super) fn handle_cancel_launch(resources: &mut Resources) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.kill_launcher();
    }
}

pub(super) fn handle_set_ide_command(resources: &mut Resources, command: Option<String>) {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.editor_config.ide_command = command;
        if let Err(e) = ps.editor_config.save() {
            tracing::warn!(error = %e, "failed to save editor config");
        }
    }
}

pub(super) fn handle_set_power_profile(resources: &mut Resources, profile: PowerProfile) {
    if let Some(slot) = resources.get_mut::<PowerProfile>() {
        if *slot != profile {
            tracing::info!(
                from = slot.as_str(),
                to = profile.as_str(),
                "power profile changed"
            );
            *slot = profile;
        }
    } else {
        resources.insert(profile);
    }
}
