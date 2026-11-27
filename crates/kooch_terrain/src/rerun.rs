//! When the levels have to be chosen again (#2013): only once something the choice reads has
//! changed. A viewer counts as moved only when it crosses into another finest block. The split set
//! is not compared: only a choice changes it, as the pool applies `wanted` only after one.

use glam::DVec3;

use crate::lod::{Limits, Planets, Shape, View, snapped_viewers};

/// What the last choice read.
#[derive(Default)]
pub(crate) struct Rerun {
    viewers: Vec<DVec3>,
    shapes: Vec<Shape>,
    limits: Vec<Limits>,
    view: Option<(f64, f64)>,
    budget: u32,
    revisions: Vec<u32>,
    /// Set when the last selection left a block unbounded, or new answers about surface arrived.
    forced: bool,
    /// The last selection's worst error, reported again while it is kept.
    pub worst: f64,
}

/// One frame's inputs to the selection.
pub(crate) struct Inputs<'a> {
    pub planets: &'a Planets,
    pub view: &'a View,
    pub budget: u32,
    pub revisions: &'a [u32],
}

impl Rerun {
    /// Whether `inputs` differ enough from the last selection's to select again.
    pub fn due(&self, inputs: &Inputs) -> bool {
        let planets = inputs.planets;
        self.forced
            || self.budget != inputs.budget
            || self.view != Some(view_key(inputs.view))
            || self.revisions != inputs.revisions
            || self.shapes != planets.shape
            || self.limits != planets.limits
            || self.viewers != snapped_viewers(planets)
    }

    /// Records the inputs a selection just ran on; `deferred` when it ran out of time to bound
    /// every block.
    pub fn ran(&mut self, inputs: &Inputs, deferred: bool, worst: f64) {
        let planets = inputs.planets;
        self.viewers = snapped_viewers(planets);
        self.shapes.clone_from(&planets.shape);
        self.limits.clone_from(&planets.limits);
        self.view = Some(view_key(inputs.view));
        self.budget = inputs.budget;
        self.revisions.clear();
        self.revisions.extend_from_slice(inputs.revisions);
        self.forced = deferred;
        self.worst = worst;
    }

    /// The GPU proved blocks empty, which changes what the selection keeps.
    pub fn force(&mut self) {
        self.forced = true;
    }
}

fn view_key(view: &View) -> (f64, f64) {
    (view.proj_scale, view.impostor_px)
}
