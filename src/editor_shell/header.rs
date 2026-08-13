use truce_core::editor::PluginContext;

use crate::editor_history::EditorHistory;
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::{KurvParams, editor, editor_theme};

use super::PresetUi;

pub(super) fn draw(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    history: &mut EditorHistory,
    presets: &mut PresetUi,
    settings_open: &mut bool,
) {
    let rect = ui.max_rect();
    let width = rect.width();
    let unit = editor_theme::title_height(ui);
    let section_gap = editor_theme::space::XXS;
    let compact = width < unit * 34.0;
    let left_width = (width * 0.17)
        .clamp(unit * 5.5, unit * if compact { 7.5 } else { 9.5 })
        .min(width * 0.25);
    let right_width = (width * 0.15)
        .clamp(unit * 4.8, unit * if compact { 6.2 } else { 8.0 })
        .min(width * 0.22);
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(left_width, rect.height()));
    let right =
        egui::Rect::from_min_max(egui::pos2(rect.right() - right_width, rect.top()), rect.max);
    let center = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, rect.top()),
        egui::pos2(right.left() - section_gap, rect.bottom()),
    );

    with_child(
        ui,
        left,
        "header-left",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = editor_theme::space::XS;
            let control_height = unit
                .min(ui.available_height())
                .max(editor_theme::shape::STROKE);
            if ui
                .add_sized(
                    [control_height, control_height],
                    egui::Button::new(
                        egui::RichText::new(if icon_font_ready(ui) {
                            egui_phosphor::regular::GEAR
                        } else {
                            "⚙"
                        })
                        .font(egui::FontId::proportional(editor_theme::font::TITLE_SIZE)),
                    )
                    .selected(*settings_open),
                )
                .on_hover_text("Settings, theme, and generator quality")
                .clicked()
            {
                *settings_open = !*settings_open;
            }
            let reset = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(concat!("KURV  v", env!("CARGO_PKG_VERSION")))
                            .font(egui::FontId::proportional(
                                editor_theme::font::TITLE_SIZE * 1.15,
                            ))
                            .strong()
                            .color(editor_theme::palette().accent),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Double-click for factory default");
            if reset.double_clicked() {
                editor::reset_to_defaults(state);
                history.commit(state);
                presets.selected = "Init".to_owned();
                presets.dirty = false;
                presets.error = None;
            }
            if left.width() >= unit * 7.5 {
                ui.label(
                    egui::RichText::new("PROCEDURAL VA")
                        .font(editor_theme::font::caption())
                        .color(editor_theme::semantic().text_muted),
                );
            }
        },
    );
    with_child(
        ui,
        center,
        "header-presets",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| draw_preset_toolbar(ui, state, history, presets, center.width()),
    );
    with_child(
        ui,
        right,
        "header-output",
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            editor::output_meter(ui, state, right.width(), right.height());
        },
    );
}

fn draw_preset_toolbar(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    history: &mut EditorHistory,
    presets: &mut PresetUi,
    width: f32,
) {
    let unit = editor_theme::title_height(ui);
    let compact = width < unit * 19.5;
    let cramped = width < unit * 12.0;
    let control_height = unit
        .min(ui.available_height())
        .max(editor_theme::shape::STROKE);
    ui.spacing_mut().interact_size.y = control_height;
    let nav_width = control_height * if cramped { 0.7 } else { 0.85 };
    let default_width = unit * if compact { 2.0 } else { 4.8 };
    let save_width = unit
        * if cramped {
            1.8
        } else if compact {
            2.3
        } else {
            3.4
        };
    let gap = if cramped {
        editor_theme::space::XXS
    } else {
        editor_theme::space::XS
    };
    ui.spacing_mut().item_spacing.x = gap;
    let reserved = nav_width * 2.0
        + save_width
        + if cramped { 0.0 } else { default_width }
        + gap * if cramped { 3.0 } else { 4.0 };
    let picker_width = (width - reserved)
        .max(if cramped { 1.0 } else { unit * 3.8 })
        .min(unit * 8.0);
    let previous = if icon_font_ready(ui) {
        egui_phosphor::regular::CARET_LEFT
    } else {
        "<"
    };
    let next = if icon_font_ready(ui) {
        egui_phosphor::regular::CARET_RIGHT
    } else {
        ">"
    };
    if ui
        .add_enabled(
            history.can_undo(),
            egui::Button::new(previous)
                .frame(false)
                .min_size(egui::vec2(nav_width, control_height)),
        )
        .on_hover_text("Undo — Ctrl/Cmd+Z")
        .clicked()
    {
        presets.dirty |= history.undo(state);
    }
    if ui
        .add_enabled(
            history.can_redo(),
            egui::Button::new(next)
                .frame(false)
                .min_size(egui::vec2(nav_width, control_height)),
        )
        .on_hover_text("Redo — Ctrl/Cmd+Shift+Z")
        .clicked()
    {
        presets.dirty |= history.redo(state);
    }

    presets.ensure_store();
    let selected_name = presets.selected.clone();
    let mut chosen_index = None;
    let mut entries_error = None;
    let selected = if presets.selected.is_empty() {
        "Init".to_owned()
    } else if presets.dirty {
        format!("{} *", presets.selected)
    } else {
        presets.selected.clone()
    };
    egui::ComboBox::from_id_salt("kurv-preset-picker")
        .selected_text(selected)
        .width(picker_width)
        .show_ui(ui, |ui| {
            if let Some(store) = presets.store.as_mut() {
                match store.entries() {
                    Ok(entries) => {
                        for (index, entry) in entries.iter().enumerate() {
                            if ui
                                .selectable_label(selected_name == entry.name(), entry.name())
                                .clicked()
                            {
                                chosen_index = Some(index);
                            }
                        }
                    }
                    Err(error) => entries_error = Some(error.to_string()),
                }
            }
        });
    if let Some(error) = entries_error {
        presets.error = Some(error);
    }
    let chosen = chosen_index.and_then(|index| {
        presets
            .store
            .as_mut()
            .and_then(|store| store.entries().ok())
            .and_then(|entries| entries.get(index).cloned())
    });
    if let Some(entry) = chosen
        && let Some(store) = presets.store.as_ref()
    {
        if entry.is_init() {
            editor::reset_to_defaults(state);
            history.commit(state);
            presets.selected = entry.name().to_owned();
            presets.dirty = false;
            presets.error = None;
        } else {
            match store.load(&entry, state) {
                Ok(()) => {
                    history.commit(state);
                    presets.selected = entry.name().to_owned();
                    presets.dirty = false;
                    presets.error = None;
                }
                Err(error) => presets.error = Some(error.to_string()),
            }
        }
    }

    let save_default = !cramped
        && ui
            .add_sized(
                [default_width, control_height],
                egui::Button::new(if compact { "DEF" } else { "SAVE DEFAULT" }),
            )
            .on_hover_text("Save the current complete state as Default.kurv")
            .clicked();
    if save_default && let Some(store) = presets.store.as_mut() {
        match store.save_default(state) {
            Ok(entry) => {
                presets.selected = entry.name().to_owned();
                presets.dirty = false;
                presets.error = None;
            }
            Err(error) => presets.error = Some(error.to_string()),
        }
    }
    if ui
        .add_sized(
            [save_width, control_height],
            egui::Button::new(if compact { "SAVE" } else { "SAVE AS" }),
        )
        .on_hover_text("Save the current state as a named preset")
        .clicked()
    {
        presets.save_open = true;
        presets.save_focus = true;
        if presets.save_name.is_empty() {
            presets.save_name = if presets.selected.is_empty() || presets.selected == "Init" {
                "New Preset".to_owned()
            } else {
                presets.selected.clone()
            };
        }
    }
    if let Some(error) = presets.error.as_deref() {
        ui.label(
            egui::RichText::new("!")
                .strong()
                .color(editor_theme::semantic().danger),
        )
        .on_hover_text(error);
    }
}
