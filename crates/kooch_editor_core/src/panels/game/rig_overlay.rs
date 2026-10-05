//! The rig over the Game view: what each camera component does to the PICTURE (#1402).
//!
//! 🔴 The Edit view's gizmos cannot simply be drawn here. Seen *from* the camera they are
//! degenerate: the arm runs from the target toward the eye and collapses to a point, the
//! deoccluder's sweep is end-on, the lead is mostly depth. True and useless.
//!
//! What reads through the lens is where a point LANDS: how far the target is from the mark the rig
//! means to hold it at, and whether something is being clamped right now. So this is a flat list of
//! projected points with a role each, drawn in one pass — not a visualizer per component, which in
//! an immediate-mode panel is a callback per component per frame.
//!
//! Hues match `gizmos/camera_rig.rs` on purpose: the same concept is the same colour whether you
//! are looking at the rig from outside or through it.

use glam::Vec2;

/// Where the rig put a point, as a fraction of the vcam's screen: `0` centre, `±0.5` the edges.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mark {
    pub at: Vec2,
    pub kind: MarkKind,
    /// A line back to this point, when the PAIR is what means something — a lead's distance from
    /// the authored mark, a group member's pull towards the centre.
    pub from: Option<Vec2>,
    /// A ring of this radius, in screen fractions, when the mark has a limit worth seeing.
    pub ring: Option<f32>,
}

impl Mark {
    pub fn new(at: Vec2, kind: MarkKind) -> Self {
        Self {
            at,
            kind,
            from: None,
            ring: None,
        }
    }

    pub fn from(mut self, at: Vec2) -> Self {
        self.from = Some(at);
        self
    }

    pub fn ring(mut self, radius: f32) -> Self {
        self.ring = Some(radius);
        self
    }
}

/// What a mark is. The role decides the colour and the shape, so a reader learns eight symbols
/// rather than eight components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkKind {
    /// Where the target actually sits.
    Target,
    /// Where the rig is holding it.
    Held,
    /// The authored `screen`, when a lead has moved the held point off it.
    Authored,
    /// One member of a target group.
    Member,
    /// The point a group resolves to — a place nothing stands.
    Centre,
    /// The shoulder a `ThirdPersonFollow` is holding.
    Shoulder,
    /// The point a body is following: the target plus its offset.
    Followed,
}

impl MarkKind {
    fn colour(self) -> egui::Color32 {
        match self {
            // Green, as the Edit view's target mark.
            Self::Target => egui::Color32::from_rgb(120, 230, 140),
            Self::Held => egui::Color32::from_rgb(255, 210, 60),
            Self::Authored => egui::Color32::from_rgb(150, 125, 40),
            Self::Member => egui::Color32::from_rgb(140, 217, 242),
            Self::Centre => egui::Color32::from_rgb(242, 242, 153),
            Self::Shoulder => egui::Color32::from_rgb(255, 217, 115),
            Self::Followed => egui::Color32::from_rgb(245, 160, 90),
        }
    }

    /// Filled for a place the rig DECIDED, hollow for a place something merely is. A reader can
    /// tell an intention from an observation without the legend.
    fn filled(self) -> bool {
        matches!(self, Self::Held | Self::Centre | Self::Shoulder)
    }

    fn radius(self) -> f32 {
        match self {
            Self::Target | Self::Centre => 5.0,
            _ => 3.5,
        }
    }
}

/// A composer's dead and soft zones, around the point it holds the target at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Zones {
    pub centre: Vec2,
    pub dead: Vec2,
    pub soft: Vec2,
    pub kind: ZoneKind,
}

/// Which composer a pair of zones belongs to.
///
/// 🔴 Drawn in different hues because **a vcam can carry both**, and then two things are deciding
/// where the target sits on screen — one by turning, one by moving. Two sets of zones in one colour
/// would read as one set with a bug in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZoneKind {
    /// `RotationComposer` — re-aims without moving.
    Rotation,
    /// `PositionComposer` — re-frames by moving.
    Position,
}

impl ZoneKind {
    fn stroke(self) -> egui::Color32 {
        match self {
            Self::Rotation => egui::Color32::from_rgb(220, 60, 60),
            Self::Position => egui::Color32::from_rgb(190, 110, 230),
        }
    }

    fn dead_stroke(self) -> egui::Color32 {
        match self {
            Self::Rotation => egui::Color32::from_rgb(90, 180, 255),
            Self::Position => egui::Color32::from_rgb(140, 200, 230),
        }
    }

    /// 🔴 Faint, because a soft zone of `1.0` IS the whole screen — the default — and at the old
    /// alpha the overlay tinted the entire game red. The band is a hint about where correction
    /// ramps, not a filter over somebody's picture.
    fn tint(self) -> egui::Color32 {
        match self {
            Self::Rotation => egui::Color32::from_rgba_unmultiplied(220, 60, 60, 14),
            Self::Position => egui::Color32::from_rgba_unmultiplied(190, 110, 230, 13),
        }
    }
}

/// Everything the selected vcam's components put on the picture this frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct RigView {
    pub zones: Vec<Zones>,
    pub marks: Vec<Mark>,
    /// One short line per component that is doing something, so a half-built rig says what it has.
    pub notes: Vec<String>,
}

impl RigView {
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty() && self.marks.is_empty() && self.notes.is_empty()
    }
}

/// Draws `view` over `image`.
pub(crate) fn draw(painter: &egui::Painter, image: egui::Rect, view: &RigView) {
    let at = |point: Vec2| {
        image.center() + egui::vec2(point.x * image.width(), -point.y * image.height())
    };
    for zones in &view.zones {
        draw_zones(painter, image, *zones, &at);
    }
    for mark in &view.marks {
        draw_mark(painter, mark, &at);
    }
    draw_notes(painter, image, &view.notes);
}

/// The soft zone tinted around a clear dead zone — Phantom Camera's viewfinder, since the numbers
/// are chosen by looking.
fn draw_zones(
    painter: &egui::Painter,
    image: egui::Rect,
    zones: Zones,
    at: &impl Fn(Vec2) -> egui::Pos2,
) {
    let centre = at(zones.centre);
    let rect = |size: Vec2| {
        egui::Rect::from_center_size(
            centre,
            egui::vec2(size.x * image.width(), size.y * image.height()),
        )
        .intersect(image)
    };
    let soft = rect(zones.soft.max(zones.dead));
    let dead = rect(zones.dead);
    // Four bands, so the dead zone itself stays clear of tint.
    for band in [
        egui::Rect::from_min_max(soft.min, egui::pos2(soft.max.x, dead.min.y)),
        egui::Rect::from_min_max(egui::pos2(soft.min.x, dead.max.y), soft.max),
        egui::Rect::from_min_max(
            egui::pos2(soft.min.x, dead.min.y),
            egui::pos2(dead.min.x, dead.max.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(dead.max.x, dead.min.y),
            egui::pos2(soft.max.x, dead.max.y),
        ),
    ] {
        painter.rect_filled(band, 0.0, zones.kind.tint());
    }
    painter.rect_stroke(
        soft,
        0.0,
        egui::Stroke::new(1.0, zones.kind.stroke()),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        dead,
        0.0,
        egui::Stroke::new(1.0, zones.kind.dead_stroke()),
        egui::StrokeKind::Inside,
    );
}

fn draw_mark(painter: &egui::Painter, mark: &Mark, at: &impl Fn(Vec2) -> egui::Pos2) {
    let point = at(mark.at);
    let colour = mark.kind.colour();
    if let Some(from) = mark.from {
        let from = at(from);
        // Only when the two are actually apart: a line a pixel long reads as a smudge on the mark.
        if (from - point).length() > 1.0 {
            painter.line_segment([from, point], egui::Stroke::new(1.0, colour));
            painter.circle_stroke(
                from,
                3.0,
                egui::Stroke::new(1.0, colour.gamma_multiply(0.6)),
            );
        }
    }
    // 🔴 Only while it fits. A 4 m cap on a target two metres away is wider than the screen, and
    // drawn it is an arc across the whole picture that communicates no limit at all.
    if let Some(radius) = mark.ring.filter(|radius| *radius < 0.5) {
        // 🔴 A round circle, and the radius is a fraction of the WIDTH on both axes. That looks
        // wrong and is right: the vertical fraction of a world distance is `aspect` times the
        // horizontal one, and the panel's height is its width over `aspect`, so the two cancel. A
        // circle in pixels is what a circle in the world projects to.
        painter.add(egui::Shape::Path(egui::epaint::PathShape::closed_line(
            ring(point, radius, painter),
            egui::Stroke::new(1.0, colour.gamma_multiply(0.7)),
        )));
    }
    if mark.kind.filled() {
        painter.circle_filled(point, mark.kind.radius(), colour);
    } else {
        painter.circle_stroke(point, mark.kind.radius(), egui::Stroke::new(1.5, colour));
    }
}

/// A ring whose radius arrives as a fraction of the panel's width — see the note above for why one
/// axis is enough.
fn ring(centre: egui::Pos2, radius: f32, painter: &egui::Painter) -> Vec<egui::Pos2> {
    const SEGMENTS: usize = 32;
    let pixels = radius * painter.clip_rect().width();
    (0..SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            centre + egui::vec2(t.cos() * pixels, t.sin() * pixels)
        })
        .collect()
}

/// The notes, bottom-left, where no card sits.
///
/// 🔴 On a plate, because pale text over a bright floor is not text. The cards on the right edge
/// have carried one since they existed; these shipped without and could not be read.
fn draw_notes(painter: &egui::Painter, image: egui::Rect, notes: &[String]) {
    const LINE: f32 = 15.0;
    const PAD: f32 = 6.0;
    if notes.is_empty() {
        return;
    }
    let font = egui::FontId::proportional(12.0);
    let widths: Vec<f32> = notes
        .iter()
        .map(|note| {
            painter
                .layout_no_wrap(note.clone(), font.clone(), egui::Color32::WHITE)
                .rect
                .width()
        })
        .collect();
    let widest = widths.iter().copied().fold(0.0_f32, f32::max);
    let height = LINE * notes.len() as f32;
    let top_left = image.left_bottom() + egui::vec2(10.0, -10.0 - height);
    painter.rect_filled(
        egui::Rect::from_min_size(
            top_left - egui::vec2(PAD, PAD),
            egui::vec2(widest + PAD * 2.0, height + PAD * 2.0),
        ),
        4.0,
        egui::Color32::from_rgba_unmultiplied(12, 12, 16, 190),
    );
    let mut cursor = top_left;
    for note in notes {
        painter.text(
            cursor,
            egui::Align2::LEFT_TOP,
            note,
            font.clone(),
            egui::Color32::from_rgb(220, 225, 235),
        );
        cursor.y += LINE;
    }
}
