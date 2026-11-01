//! Toggles and sliders (#280 phase 6): a value on the node, parts the sheet styles, and the
//! gestures that change it.

use crate::source::{Element, ElementKind, UiSource};
use crate::style::STATE_CHECKED;
use crate::{UiNode, UiTree};

/// The classes of the nodes a control is drawn with, styled like any other by the sheet.
pub const TOGGLE_CHECK: &str = "toggle-check";
pub const SLIDER_FILL: &str = "slider-fill";
pub const SLIDER_THUMB: &str = "slider-thumb";
pub const SLIDER_REST: &str = "slider-rest";

/// Steps a pad takes across a slider without a `step`.
const PAD_STEPS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Toggle,
    Slider,
}

/// `bind:value="Type.field"`: a field of a component on the document's entity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Binding {
    pub component: Box<str>,
    pub field: Box<str>,
}

#[derive(Debug, Clone)]
pub(crate) struct Control {
    pub node: UiNode,
    pub kind: ControlKind,
    /// A toggle's is 0 or 1.
    pub value: f32,
    pub min: f32,
    pub max: f32,
    /// `0` slides freely.
    pub step: f32,
    pub bind: Option<Binding>,
    /// A slider's fill, thumb and rest.
    pub parts: Option<[UiNode; 3]>,
    /// Changed by a gesture since the bindings last ran: the UI writes, the field does not.
    pub touched: bool,
}

/// `source` with each control's parts appended as its last children.
pub(crate) fn with_parts(source: &UiSource) -> UiSource {
    let mut out = source.clone();
    for (index, element) in source.elements.iter().enumerate() {
        let classes: &[&str] = match element.kind {
            ElementKind::Toggle => &[TOGGLE_CHECK],
            ElementKind::Slider => &[SLIDER_FILL, SLIDER_THUMB, SLIDER_REST],
            _ => continue,
        };
        for class in classes {
            out.elements.push(Element {
                kind: ElementKind::Panel,
                parent: index as u32,
                name: None,
                classes: vec![(*class).into()],
                text: None,
                attributes: Vec::new(),
                overrides: Vec::new(),
            });
        }
    }
    out
}

/// The control an element declares, from its attributes; parts are found once the tree is built.
pub(crate) fn declared(element: &Element, node: UiNode) -> Option<Control> {
    let kind = match element.kind {
        ElementKind::Toggle => ControlKind::Toggle,
        ElementKind::Slider => ControlKind::Slider,
        _ => return None,
    };
    let number = |key: &str| {
        element
            .attributes
            .iter()
            .find(|(own, _)| &**own == key)
            .and_then(|(_, value)| value.parse::<f32>().ok())
    };
    let value = match kind {
        ControlKind::Toggle => element
            .attributes
            .iter()
            .any(|(key, value)| &**key == "checked" && &**value != "false")
            as u8 as f32,
        ControlKind::Slider => number("value").unwrap_or(0.0),
    };
    let bind = element
        .attributes
        .iter()
        .find(|(key, _)| &**key == "bind:value")
        .and_then(|(_, path)| path.split_once('.'))
        .map(|(component, field)| Binding {
            component: component.into(),
            field: field.into(),
        });
    Some(Control {
        node,
        kind,
        value,
        min: number("min").unwrap_or(0.0),
        max: number("max").unwrap_or(1.0),
        step: number("step").unwrap_or(0.0).max(0.0),
        bind,
        parts: None,
        touched: false,
    })
}

impl UiTree {
    /// A node the pointer and focus act on: one with `on:click`, or a control.
    pub fn interactive(&self, node: UiNode) -> bool {
        self.click(node).is_some() || self.control(node).is_some()
    }

    /// A toggle's (`0` or `1`) or a slider's value.
    pub fn value(&self, node: UiNode) -> Option<f32> {
        self.control(node).map(|control| control.value)
    }

    /// Sets a control's value from code, without `on:change`: what a binding does too.
    pub fn set_value(&mut self, node: UiNode, value: f32) {
        self.change(node, value, false);
    }

    pub(crate) fn control(&self, node: UiNode) -> Option<&Control> {
        self.controls.iter().find(|control| control.node == node)
    }

    pub(crate) fn control_kind(&self, node: UiNode) -> Option<ControlKind> {
        self.control(node).map(|control| control.kind)
    }

    /// Flips a toggle; anything else, a slider included, is left as is. Whether it changed.
    pub(crate) fn flip(&mut self, node: UiNode) -> bool {
        match self.control(node) {
            Some(control) if control.kind == ControlKind::Toggle => {
                self.change(node, 1.0 - control.value, true)
            }
            _ => false,
        }
    }

    /// Moves a slider by one step, `sign` sets the way. Whether it changed.
    pub(crate) fn step(&mut self, node: UiNode, sign: f32) -> bool {
        let Some(control) = self.control(node) else {
            return false;
        };
        let step = match control.step > 0.0 {
            true => control.step,
            false => (control.max - control.min) / PAD_STEPS,
        };
        self.change(node, control.value + step * sign, true)
    }

    /// Puts a slider's thumb under `x`, in canvas units. Whether it changed.
    pub(crate) fn slide_to(&mut self, node: UiNode, x: f32) -> bool {
        let Some(control) = self.control(node) else {
            return false;
        };
        let (Some(rect), thumb) = (
            self.rect(node),
            control.parts.and_then(|[_, thumb, _]| self.rect(thumb)),
        ) else {
            return false;
        };
        let thumb = thumb.map_or(0.0, |thumb| thumb.size.x);
        let track = (rect.size.x - thumb).max(1.0);
        let t = ((x - rect.min.x - thumb * 0.5) / track).clamp(0.0, 1.0);
        let mut value = control.min + t * (control.max - control.min);
        if control.step > 0.0 {
            value = control.min + ((value - control.min) / control.step).round() * control.step;
        }
        self.change(node, value, true)
    }

    /// Clamps, stores and shows `value`. Whether it changed.
    fn change(&mut self, node: UiNode, value: f32, touched: bool) -> bool {
        // A gesture on a disabled control does nothing; code still sets it.
        if touched && self.disabled(node) {
            return false;
        }
        let Some(control) = self.controls.iter_mut().find(|c| c.node == node) else {
            return false;
        };
        let value = match control.kind {
            ControlKind::Toggle => (value >= 0.5) as u8 as f32,
            ControlKind::Slider => {
                value.clamp(control.min.min(control.max), control.max.max(control.min))
            }
        };
        if value == control.value {
            return false;
        }
        control.value = value;
        control.touched |= touched;
        let control = control.clone();
        self.show(&control);
        true
    }

    /// Every control's value on its parts and `:checked`; the restyle resets both.
    pub(crate) fn show_values(&mut self) {
        for control in self.controls.clone() {
            self.show(&control);
        }
    }

    fn show(&mut self, control: &Control) {
        match control.kind {
            ControlKind::Toggle => self.set_state(control.node, STATE_CHECKED, control.value > 0.5),
            ControlKind::Slider => {
                let Some([fill, _, rest]) = control.parts else {
                    return;
                };
                let span = control.max - control.min;
                let t = match span.abs() > f32::EPSILON {
                    true => ((control.value - control.min) / span).clamp(0.0, 1.0),
                    false => 0.0,
                };
                self.set_grow(fill, t);
                self.set_grow(rest, 1.0 - t);
            }
        }
    }
}
