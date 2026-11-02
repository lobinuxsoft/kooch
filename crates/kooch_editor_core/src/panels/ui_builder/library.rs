//! The elements a document is built from, and the project's `.kxml` templates, dragged into the
//! hierarchy or the canvas, as Unity's Builder Library.

use kooch_ui::markup::Place;

use super::drop::{self, Dragged};
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::{SOURCE_TYPE, UiBuilder};

pub(super) struct Item {
    pub tag: &'static str,
    /// What a drop writes: enough to be seen on the canvas.
    pub snippet: &'static str,
    pub hint: &'static str,
}

const STANDARD: [Item; 5] = [
    Item {
        tag: "panel",
        snippet: "<panel/>",
        hint: "A box that lays out its children",
    },
    Item {
        tag: "label",
        snippet: "<label>Label</label>",
        hint: "Text",
    },
    Item {
        tag: "button",
        snippet: "<button><label>Button</label></button>",
        hint: "Sends on:click; holds a label",
    },
    Item {
        tag: "toggle",
        snippet: "<toggle/>",
        hint: "On or off; sends on:change",
    },
    Item {
        tag: "slider",
        snippet: "<slider/>",
        hint: "A value between min and max; sends on:change",
    },
];

const HOW: &str = "Drag into the hierarchy or the canvas; double-click adds it to the selection";

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, catalog: &[AssetCatalogEntry]) {
    ui.strong("Library");
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("ui_builder_library")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new("Standard")
                .default_open(true)
                .show(ui, |ui| {
                    for item in &STANDARD {
                        let text = egui::RichText::new(format!("<{}>", item.tag)).monospace();
                        let response = ui
                            .selectable_label(false, text)
                            .interact(egui::Sense::drag())
                            .on_hover_text(format!("{}. {HOW}", item.hint));
                        pick(builder, &response, Dragged::Element(item), catalog);
                    }
                });
            egui::CollapsingHeader::new("Project")
                .default_open(true)
                .show(ui, |ui| {
                    let mut shown = 0;
                    let open = builder.source;
                    for entry in catalog
                        .iter()
                        .filter(|e| e.type_name == SOURCE_TYPE && e.guid != open)
                    {
                        let response = ui
                            .selectable_label(false, &entry.display_name)
                            .interact(egui::Sense::drag())
                            .on_hover_text(format!("{}\n{HOW}", entry.path.display()));
                        pick(builder, &response, Dragged::Template(entry.guid), catalog);
                        shown += 1;
                    }
                    if shown == 0 {
                        ui.weak("(no other .kxml in the project)");
                    }
                });
        });
}

fn pick(
    builder: &mut UiBuilder,
    response: &egui::Response,
    dragged: Dragged,
    catalog: &[AssetCatalogEntry],
) {
    if response.drag_started() {
        egui::DragAndDrop::set_payload(&response.ctx, dragged);
    } else if response.double_clicked() {
        let at = match builder.selected {
            Some(origin) if origin != 0 && !builder.holds_children(origin) => Place::After(origin),
            Some(origin) => Place::Inside(origin),
            None => Place::Inside(0),
        };
        drop::apply(builder, dragged, at, catalog);
    }
}
