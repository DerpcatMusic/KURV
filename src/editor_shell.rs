use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_history::EditorHistory;
use crate::editor_presets::PresetStore;
use crate::editor_widgets::with_child;
use crate::{KurvParams, editor_theme};

mod header;
mod settings;

const HISTORY_COMMIT_REQUEST_ID: &str = "kurv-history-explicit-commit";

/// Schedule host dirty notification and a whole-editor history snapshot after
/// a structural action that is not guaranteed to end in an egui pointer-release
/// event (for example, an OS file drop). The shell handles the request after all
/// callers have published their state and flushes history on the next frame.
pub(crate) fn request_structural_commit(ui: &egui::Ui) {
    ui.data_mut(|data| {
        data.insert_temp(egui::Id::new(HISTORY_COMMIT_REQUEST_ID), true);
    });
    ui.ctx().request_repaint();
}

#[derive(Clone, Default)]
struct PresetUi {
    store: Option<PresetStore>,
    selected: String,
    dirty: bool,
    save_name: String,
    save_open: bool,
    save_focus: bool,
    error: Option<String>,
}

impl PresetUi {
    fn ensure_store(&mut self) {
        if self.store.is_none() && self.error.is_none() {
            match PresetStore::new() {
                Ok(mut store) => {
                    if let Err(error) = store.refresh() {
                        self.error = Some(error.to_string());
                    }
                    self.store = Some(store);
                    if self.selected.is_empty() {
                        self.selected = "Init".to_owned();
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let persisted_theme = settings::read_persisted_theme(state);
    let theme_id = egui::Id::new("kurv-theme-ui");
    let mut themes = ui
        .data_mut(|data| data.remove_temp::<settings::ThemeUi>(theme_id))
        .unwrap_or_default();
    settings::apply_active_theme(ui, state, &mut themes, persisted_theme);
    let history_id = egui::Id::new("kurv-editor-history");
    let preset_id = egui::Id::new("kurv-preset-ui");
    let settings_id = egui::Id::new("kurv-settings-open");
    let mut history = ui
        .data_mut(|data| data.remove_temp::<EditorHistory>(history_id))
        .unwrap_or_default();
    let mut presets = ui
        .data_mut(|data| data.remove_temp::<PresetUi>(preset_id))
        .unwrap_or_default();
    let mut settings_open = ui
        .data(|data| data.get_temp::<bool>(settings_id))
        .unwrap_or(false);
    history.capture_initial(state);
    presets.dirty |= history.flush_deferred(state);
    if history.handle_shortcuts(ui, state) {
        presets.dirty = true;
        crate::editor::notify_persisted_state_changed(state);
    }
    let bounds = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(bounds, 0.0, editor_theme::semantic().background);
    ui.allocate_rect(bounds, egui::Sense::hover());

    let shortest_side = bounds.width().min(bounds.height());
    let gap = (shortest_side / 180.0).clamp(editor_theme::space::XXS, editor_theme::space::XS);
    let section_gap = (gap * 1.6).clamp(editor_theme::space::XS, editor_theme::space::SM);
    let content = bounds;
    let title_height = editor_theme::title_height(ui);
    let header_height = (content.height() * 0.13).clamp(title_height * 4.2, title_height * 5.8);
    let header_rect =
        egui::Rect::from_min_size(content.min, egui::vec2(content.width(), header_height));
    let workspace = egui::Rect::from_min_max(
        egui::pos2(content.left(), header_rect.bottom()),
        content.right_bottom(),
    );

    ui.painter()
        .rect_filled(header_rect, 0.0, editor_theme::semantic().primary);
    with_child(
        ui,
        header_rect,
        "header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| header::draw(ui, state, &mut history, &mut presets, &mut settings_open),
    );

    let usable_width = (workspace.width() - section_gap).max(1.0);
    let split_id = egui::Id::new("generator-modulator-split");
    let mut split = ui
        .data(|data| data.get_temp::<f32>(split_id))
        .unwrap_or(0.72)
        .clamp(0.42, 0.84);
    let mut divider_x = workspace.left() + usable_width * split + section_gap * 0.5;
    let divider_response = ui
        .interact(
            egui::Rect::from_center_size(
                egui::pos2(divider_x, workspace.center().y),
                egui::vec2(section_gap.max(title_height * 0.7), workspace.height()),
            ),
            split_id,
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag to resize generators and modulators; double-click to reset");
    if divider_response.double_clicked() {
        split = 0.72;
    } else if divider_response.dragged()
        && let Some(pointer) = ui.input(|input| input.pointer.latest_pos())
    {
        split =
            ((pointer.x - workspace.left() - section_gap * 0.5) / usable_width).clamp(0.42, 0.84);
    } else if divider_response.has_focus() {
        split = (split
            + ui.input(|input| {
                0.01 * (f32::from(input.key_pressed(egui::Key::ArrowRight))
                    - f32::from(input.key_pressed(egui::Key::ArrowLeft)))
            }))
        .clamp(0.42, 0.84);
    }
    ui.data_mut(|data| data.insert_temp(split_id, split));
    let left_width = usable_width * split;
    let left = egui::Rect::from_min_size(workspace.min, egui::vec2(left_width, workspace.height()));
    let right = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, workspace.top()),
        workspace.right_bottom(),
    );
    let generator_body = section_body(ui, left, "GENERATORS");
    crate::editor_generator::show(ui, state, generator_body, gap, section_gap);
    draw_modulation(ui, state, right);
    divider_x = left.right() + section_gap * 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(divider_x, workspace.top()),
            egui::pos2(divider_x, workspace.bottom()),
        ],
        egui::Stroke::new(
            if divider_response.hovered() || divider_response.dragged() {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if divider_response.hovered() || divider_response.dragged() {
                editor_theme::semantic().primary
            } else {
                editor_theme::semantic().grid
            },
        ),
    );
    if !settings_open && !presets.save_open {
        crate::editor_modulation::draw_overlay(ui, state);
    } else {
        crate::editor_modulation::cancel_interaction(ui, state);
    }

    let popup_was_open = ui.ctx().any_popup_open();
    if !popup_was_open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        if presets.save_open {
            presets.save_open = false;
        } else {
            settings_open = false;
        }
    }
    if presets.save_open {
        settings_open = false;
        draw_overlay_scrim(ui, workspace, "save-preset-scrim");
        draw_save_preset_panel(ui, state, workspace, &mut presets, popup_was_open);
    } else if settings_open {
        draw_overlay_scrim(ui, workspace, "settings-scrim");
        settings::draw(
            ui,
            state,
            workspace,
            &mut settings_open,
            &mut themes,
            popup_was_open,
        );
    }
    let explicit_commit = ui.data_mut(|data| {
        let id = egui::Id::new(HISTORY_COMMIT_REQUEST_ID);
        let requested = data.get_temp::<bool>(id).unwrap_or(false);
        data.remove::<bool>(id);
        requested
    });
    if explicit_commit {
        crate::editor::notify_persisted_state_changed(state);
    }
    if explicit_commit || ui.input(|input| input.pointer.any_released()) {
        history.defer_commit();
        ui.ctx().request_repaint();
    }
    let now = ui.input(|input| input.time);
    themes.flush(now, !settings_open);
    ui.data_mut(|data| {
        data.insert_temp(history_id, history);
        data.insert_temp(preset_id, presets);
        data.insert_temp(settings_id, settings_open);
        data.insert_temp(theme_id, themes);
    });
}

fn draw_overlay_scrim(ui: &mut egui::Ui, workspace: egui::Rect, id: &'static str) {
    ui.painter()
        .rect_filled(workspace, 0.0, egui::Color32::from_black_alpha(96));
    ui.interact(workspace, egui::Id::new(id), egui::Sense::click());
}

fn draw_save_preset_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    workspace: egui::Rect,
    presets: &mut PresetUi,
    popup_was_open: bool,
) {
    let title_height = editor_theme::title_height(ui);
    let available = workspace.shrink(editor_theme::space::SM);
    let size = egui::vec2(
        (available.width() * 0.36)
            .clamp(title_height * 10.0, title_height * 15.0)
            .min(available.width()),
        (title_height * 5.0).min(available.height()),
    );
    let rect = egui::Rect::from_center_size(available.center(), size);
    ui.painter().rect(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().surface,
        egui::Stroke::new(editor_theme::shape::STROKE, editor_theme::semantic().grid),
        egui::StrokeKind::Inside,
    );
    with_child(
        ui,
        rect.shrink(editor_theme::space::SM),
        "save-preset-panel",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(
                egui::RichText::new("SAVE PRESET")
                    .font(editor_theme::font::title())
                    .color(editor_theme::semantic().text),
            );
            ui.add_space(editor_theme::compact_gap(ui));
            let response = ui.text_edit_singleline(&mut presets.save_name);
            if response.changed() {
                presets.error = None;
            }
            if presets.save_focus {
                response.request_focus();
                presets.save_focus = false;
            }
            let submit =
                response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if let Some(error) = presets.error.as_deref() {
                ui.label(
                    egui::RichText::new(error)
                        .font(editor_theme::font::caption())
                        .color(editor_theme::semantic().danger),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("CANCEL").clicked() {
                    presets.save_open = false;
                }
                if (ui.button("SAVE").clicked() || submit)
                    && let Some(store) = presets.store.as_mut()
                {
                    match store.save_as(&presets.save_name, state) {
                        Ok(entry) => {
                            presets.selected = entry.name().to_owned();
                            presets.dirty = false;
                            presets.error = None;
                            presets.save_open = false;
                        }
                        Err(error) => presets.error = Some(error.to_string()),
                    }
                }
                if ui
                    .button("SET DEFAULT")
                    .on_hover_text("Replace the patch loaded when KURV starts")
                    .clicked()
                    && let Some(store) = presets.store.as_mut()
                {
                    match store.save_default(state) {
                        Ok(entry) => {
                            presets.selected = entry.name().to_owned();
                            presets.dirty = false;
                            presets.error = None;
                            presets.save_open = false;
                        }
                        Err(error) => presets.error = Some(error.to_string()),
                    }
                }
            });
        },
    );
    if !popup_was_open
        && ui.input(|input| {
            input.pointer.primary_clicked()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|pointer| workspace.contains(pointer) && !rect.contains(pointer))
        })
    {
        presets.save_open = false;
    }
}

fn draw_modulation(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    let rect = section_body(ui, rect, "MODULATORS");
    with_child(
        ui,
        rect,
        "modulation",
        egui::Layout::top_down(egui::Align::Min),
        |ui| crate::editor_lfo::modulation_view(ui, state, rect.width(), rect.height()),
    );
}

fn section_body(ui: &egui::Ui, rect: egui::Rect, label: &str) -> egui::Rect {
    let palette = editor_theme::semantic();
    ui.painter()
        .rect_filled(rect, 0.0, palette.well.gamma_multiply(0.72));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        fit_font_to_width(
            ui.painter(),
            label,
            egui::FontId::proportional(editor_theme::title_height(ui) * 6.4),
            rect.width() * 0.78,
        ),
        egui::Color32::from_rgba_unmultiplied(
            palette.text.r(),
            palette.text.g(),
            palette.text.b(),
            10,
        ),
    );
    rect
}
