//! What the selected vcam's components put on the Game view this frame (#1402).
//!
//! 🔴 **A point only goes on the picture if the picture can hold it.** The arm's length, an orbital
//! radius and a deoccluder's pull are DEPTH: projected from the camera they land on the eye, or on
//! the target, and say nothing. Those become a line of text. What becomes a mark is what the lens
//! actually separates — where the target sits against where the rig means to hold it.
//!
//! 🔴 **Projected from the camera that DREW the picture, never from the selected vcam.** They are
//! not the same eye: a vcam you are inspecting may not be the one winning the election — a
//! `priority` of -1 against 0 — and during a handover the camera is between two of them and is
//! neither. Projecting from the vcam put every mark somewhere the picture does not agree with,
//! which is the bug this overlay shipped with. `CameraStack::read` is the same call
//! `viewport/game.rs` renders the panel through, so the two cannot drift apart again.
//!
//! Collected with `&Resources` and drawn later, so nothing here touches the world.

use glam::{Vec2, Vec3};
use kooch_camera::framing::Lens;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;

use crate::panels::game::rig_overlay::{Mark, MarkKind, RigView, ZoneKind, Zones};

/// Puts a world point on the vcam's screen, or `None` when it is behind the lens.
struct Screen {
    at: Vec3,
    right: Vec3,
    above: Vec3,
    forward: Vec3,
    lens: Lens,
}

impl Screen {
    fn of(&self, point: Vec3) -> Option<Vec2> {
        let offset = point - self.at;
        let depth = offset.dot(self.forward);
        if depth <= 0.0 {
            return None;
        }
        let span = self.lens.span(depth);
        Some(Vec2::new(
            offset.dot(self.right) / span.x,
            offset.dot(self.above) / span.y,
        ))
    }

    /// A world length at the depth `point` sits at, as a fraction of the screen's width. What turns
    /// a metre cap into a ring somebody can see.
    fn across(&self, point: Vec3, metres: f32) -> Option<f32> {
        let depth = (point - self.at).dot(self.forward);
        (depth > 0.0).then(|| metres / self.lens.span(depth).x)
    }
}

/// The aspect the panel last rendered at.
fn aspect(resources: &kooch_core::resource::Resources) -> f32 {
    resources
        .get::<kooch_ecs::ViewAspect>()
        .copied()
        .unwrap_or_default()
        .0
}

/// The rig of the selected vcam, and of every pinned one.
///
/// 🔴 Pinned entities draw beside the selection in the Edit view, and an overlay that ignored them
/// made pinning useless for the one view where a rig is actually judged.
pub(super) fn selected_rig(
    resources: &kooch_core::resource::Resources,
    selected: &[Entity],
    pinned: &[Entity],
) -> Option<RigView> {
    // Read once, not per vcam: it is a query, and every vcam is seen through the same eye.
    let camera = kooch_render::CameraStack::read::<
        kooch_ecs::query::filter::Without<crate::editor_camera::markers::EditorCamera>,
    >(resources)
    .base?;
    let (_, rotation, at) = camera.world_matrix.to_scale_rotation_translation();
    let screen = Screen {
        at,
        right: rotation * Vec3::X,
        above: rotation * Vec3::Y,
        forward: rotation * -Vec3::Z,
        // Its own lens too: during a blend this holds the interpolated field of view, which is what
        // the frame was actually drawn with (#1254).
        lens: Lens::new(camera.fov_y_rad.to_degrees(), aspect(resources)),
    };

    let registry = resources.get::<ComponentRegistry>()?;
    let mut view = RigView::default();
    // Selected first, then pinned minus whatever is both — drawing one twice doubles every line.
    let drawn = selected
        .iter()
        .copied()
        .chain(pinned.iter().copied().filter(|e| !selected.contains(e)));
    for entity in drawn {
        let Some(mut one) = one_rig(resources, registry, &screen, entity) else {
            continue;
        };
        // 🔴 Named, because with a pin there are several and an unlabelled line belongs to nobody.
        if let Some(name) = registry
            .get_cpu::<kooch_ecs::Name>()
            .and_then(|names| names.get(entity))
        {
            for note in &mut one.notes {
                *note = format!("{}: {note}", name.value);
            }
        }
        view.zones.extend(one.zones);
        view.marks.extend(one.marks);
        view.notes.extend(one.notes);
    }
    (!view.is_empty()).then_some(view)
}

/// One vcam's rig, or nothing when `entity` is not a vcam.
fn one_rig(
    resources: &kooch_core::resource::Resources,
    registry: &ComponentRegistry,
    screen: &Screen,
    entity: Entity,
) -> Option<RigView> {
    // 🔴 Not gated on `RotationComposer` any more. It was, so a vcam without one drew nothing at
    // all — and a half-built rig is exactly when an author needs to see what it has (#1402).
    let vcam = registry
        .get_cpu::<kooch_camera::VirtualCamera>()?
        .get(entity)?;

    let mut view = RigView::default();
    let target = super::camera_target_point(resources, entity);
    let lead = resources
        .get::<kooch_camera::RigMemory>()
        .and_then(|memory| memory.leads.of(entity))
        .map(|lead| lead.offset())
        .unwrap_or(Vec3::ZERO);

    group(&mut view, registry, screen, vcam.group, target);
    composers(&mut view, registry, entity, screen, target, lead);
    bodies(
        &mut view, registry, entity, &screen, target, resources, vcam,
    );
    aims(&mut view, registry, entity);
    arm(&mut view, resources, entity);

    (!view.is_empty()).then_some(view)
}

/// The target, its group's members, and the point they resolve to.
fn group(
    view: &mut RigView,
    registry: &ComponentRegistry,
    screen: &Screen,
    group: u32,
    target: Option<Vec3>,
) {
    let Some(target) = target else {
        view.notes
            .push("no target: nothing carries this vcam's group".to_owned());
        return;
    };
    let Some(centre) = screen.of(target) else {
        view.notes
            .push("the target is behind the camera".to_owned());
        return;
    };

    let members: Vec<Vec3> = registry
        .get_cpu::<kooch_camera::CameraTarget>()
        .map(|targets| {
            let globals = registry.get_cpu::<GlobalTransform>();
            targets
                .iter()
                .filter(|(_, member)| member.group == group)
                .filter_map(|(entity, _)| {
                    globals?
                        .get(*entity)
                        .map(|global| global.matrix.to_scale_rotation_translation().2)
                })
                .collect()
        })
        .unwrap_or_default();

    // 🔴 With one member the centre IS that member, and two marks on one pixel read as a rendering
    // bug. With several it is a place nothing stands, which is exactly why it has to be drawn.
    if members.len() > 1 {
        for member in &members {
            if let Some(on) = screen.of(*member) {
                view.marks
                    .push(Mark::new(on, MarkKind::Member).from(centre));
            }
        }
        view.marks.push(Mark::new(centre, MarkKind::Centre));
        view.notes.push(format!(
            "group {group}: {} targets, weighted",
            members.len()
        ));
    } else {
        view.marks.push(Mark::new(centre, MarkKind::Target));
    }
}

/// Each composer's zones, and the lead that moved the point they are centred on.
fn composers(
    view: &mut RigView,
    registry: &ComponentRegistry,
    entity: Entity,
    screen: &Screen,
    target: Option<Vec3>,
    lead: Vec3,
) {
    let led = |point: Vec2| point - on_axes(screen, target, lead).unwrap_or(Vec2::ZERO);

    if let Some(framing) = registry
        .get_cpu::<kooch_camera::RotationComposer>()
        .and_then(|storage| storage.get(entity))
        .filter(|framing| framing.enabled)
    {
        let held = led(framing.screen);
        view.zones.push(Zones {
            centre: held,
            dead: framing.dead_zone,
            soft: framing.soft_zone.max(framing.dead_zone),
            kind: ZoneKind::Rotation,
        });
        view.marks.push(Mark::new(held, MarkKind::Held));
        // Where the lead has moved it, when it has: a line back to the authored `screen` says how
        // much of what you see is the lookahead rather than the setting.
        if (held - framing.screen).length() > 1e-3 {
            view.marks
                .push(Mark::new(framing.screen, MarkKind::Authored).from(held));
        }
    }

    if let Some(composer) = registry
        .get_cpu::<kooch_camera::PositionComposer>()
        .and_then(|storage| storage.get(entity))
    {
        view.zones.push(Zones {
            centre: composer.screen_position,
            dead: composer.dead_zone,
            soft: composer.soft_zone.max(composer.dead_zone),
            kind: ZoneKind::Position,
        });
        view.notes.push(format!(
            "position composer: {:.1} m, dead depth {:.1}",
            composer.camera_distance, composer.dead_zone_depth
        ));
    }

    if let Some(look) = registry
        .get_cpu::<kooch_camera::CameraLookahead>()
        .and_then(|storage| storage.get(entity))
        .filter(|look| look.enabled)
    {
        // The cap as a ring on the target, drawn whether or not the lead has reached it: a ceiling
        // you cannot see is a number you cannot tune.
        if let Some((centre, radius)) = target.and_then(|target| {
            Some((
                screen.of(target)?,
                screen.across(target, look.max_distance)?,
            ))
        }) {
            view.marks
                .push(Mark::new(centre, MarkKind::Target).ring(radius));
        }
        view.notes.push(format!(
            "lookahead: {:.2} s, capped at {:.1} m",
            look.time, look.max_distance
        ));
    }
}

/// A world offset in the camera's own axes, at the target's depth.
fn on_axes(screen: &Screen, target: Option<Vec3>, offset: Vec3) -> Option<Vec2> {
    let target = target?;
    let depth = (target - screen.at).dot(screen.forward);
    if depth <= 0.0 || offset.length_squared() < 1e-9 {
        return None;
    }
    let span = screen.lens.span(depth);
    Some(Vec2::new(
        offset.dot(screen.right) / span.x,
        offset.dot(screen.above) / span.y,
    ))
}

/// What the body is holding: the point it follows, and a shoulder where there is one.
fn bodies(
    view: &mut RigView,
    registry: &ComponentRegistry,
    entity: Entity,
    screen: &Screen,
    target: Option<Vec3>,
    resources: &kooch_core::resource::Resources,
    vcam: &kooch_camera::VirtualCamera,
) {
    let Some(target) = target else { return };
    let centre = screen.of(target);

    // 🔴 An offset body follows a point BESIDE the target, and that gap is a screen-space fact: it
    // is why the character is not where the zones say it should be.
    if let Some(body) = registry
        .get_cpu::<kooch_camera::Follow>()
        .and_then(|s| s.get(entity))
        && body.offset.length_squared() > 1e-9
        && let (Some(on), Some(centre)) = (screen.of(target + body.offset), centre)
    {
        view.marks
            .push(Mark::new(on, MarkKind::Followed).from(centre));
    }

    // 🔴 `HardLockToTarget` is a unit struct: the camera IS the target, with nothing beside it to
    // mark. Saying so is the useful thing, because a locked rig looks broken until you know.
    if registry
        .get_cpu::<kooch_camera::HardLockToTarget>()
        .is_some_and(|s| s.get(entity).is_some())
    {
        view.notes
            .push("hard lock: the camera sits on the target".to_owned());
    }

    if let Some(body) = registry
        .get_cpu::<kooch_camera::ThirdPersonFollow>()
        .and_then(|s| s.get(entity))
        .copied()
    {
        // The rig's own answer where it has run, and its own first-step answer where it has not —
        // not a second opinion. A stopped editor has no `RigMemory` at all (#1387).
        let (up, reference) = resources
            .get::<kooch_camera::RigMemory>()
            .and_then(|memory| memory.horizons.used(entity))
            .unwrap_or_else(|| {
                let up = kooch_camera::up_for(vcam, resources, target, glam::Quat::IDENTITY);
                (up, kooch_camera::seed_reference(up))
            });
        // 🔴 The direction the rig is actually looking FROM, levelled against `up` — not the
        // seeded reference, which is a fixed axis that knows nothing about where anything points.
        // Handed the seed, the shoulder sat in one place while the camera swung around it, which is
        // the opposite of what a shoulder is. `gizmos/virtual_camera.rs` works it out the same way.
        let eye = registry
            .get_cpu::<GlobalTransform>()
            .and_then(|globals| globals.get(entity))
            .map(|global| global.matrix.to_scale_rotation_translation().2);
        let back = eye
            .and_then(|eye| (eye - target).try_normalize())
            .map(|out| out - up * out.dot(up))
            .and_then(Vec3::try_normalize)
            .unwrap_or(reference);
        let (_, shoulder, _) = vcam.rig_positions(target, up, back, body);
        if let Some(on) = screen.of(shoulder) {
            view.marks
                .push(Mark::new(on, MarkKind::Shoulder).from(centre.unwrap_or(on)));
        }
        view.notes.push(format!(
            "third person: shoulder {:.2}, arm {:.2} m, side {:.2}",
            body.shoulder_offset.x, body.vertical_arm_length, body.camera_side
        ));
    }

    if let Some(body) = kooch_camera::orbital_follow::of(registry, entity) {
        // Depth, so it is a number rather than a mark: an orbital radius projects onto the eye.
        view.notes
            .push(match body.orbit_style == kooch_camera::ORBIT_THREE_RING {
                true => format!(
                    "orbital: three rings, {:.1}/{:.1}/{:.1} m",
                    body.top_radius, body.center_radius, body.bottom_radius
                ),
                false => format!("orbital: sphere, {:.1} m", body.radius),
            });
    }
}

/// Which aim is turning the camera. The centre of the picture IS where it points, so the target's
/// own mark already says how far off it is — what is missing is WHICH aim to blame.
fn aims(view: &mut RigView, registry: &ComponentRegistry, entity: Entity) {
    let named = [
        (
            "hard look at",
            registry
                .get_cpu::<kooch_camera::HardLookAt>()
                .is_some_and(|s| s.get(entity).is_some()),
        ),
        (
            "pan tilt",
            registry
                .get_cpu::<kooch_camera::PanTilt>()
                .is_some_and(|s| s.get(entity).is_some()),
        ),
        (
            "rotate with target",
            registry
                .get_cpu::<kooch_camera::RotateWithFollowTarget>()
                .is_some_and(|s| s.get(entity).is_some()),
        ),
    ];
    let on: Vec<&str> = named
        .iter()
        .filter(|(_, has)| *has)
        .map(|(name, _)| *name)
        .collect();
    match on.len() {
        0 => view
            .notes
            .push("no aim: nothing turns this camera".to_owned()),
        // The rig reports this too, but the overlay is where an author is looking (#1397).
        1 => {}
        _ => view.notes.push(format!("TWO aims: {}", on.join(" and "))),
    }
}

/// Whether a wall is holding the arm in, which is depth and therefore a number.
fn arm(view: &mut RigView, resources: &kooch_core::resource::Resources, entity: Entity) {
    let Some((_, length, returning)) = resources
        .get::<kooch_camera::RigMemory>()
        .and_then(|memory| memory.arms.held_of(entity))
    else {
        return;
    };
    view.notes.push(match returning {
        true => format!("deoccluder: returning, arm {length:.2} m"),
        false => format!("deoccluder: arm held at {length:.2} m"),
    });
}

#[cfg(test)]
mod tests;
