use truce_core::editor::PluginContext;

use crate::editor_history::EditorHistory;
use crate::editor_presets::PresetStore;
use crate::editor_widgets::with_child;
use crate::{KurvParams, editor_theme};

mod header;
mod settings;

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
    ensure_icon_font(ui);
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
    }
    let bounds = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(bounds, 0.0, editor_theme::semantic().background);
    ui.allocate_rect(bounds, egui::Sense::hover());

    let shortest_side = bounds.width().min(bounds.height());
    let inset = (shortest_side / 90.0).clamp(editor_theme::space::XS, editor_theme::space::SM);
    let gap = (shortest_side / 180.0).clamp(editor_theme::space::XXS, editor_theme::space::XS);
    let section_gap = (gap * 1.6).clamp(editor_theme::space::XS, editor_theme::space::SM);
    let content = bounds.shrink(inset);
    let title_height = editor_theme::title_height(ui);
    let header_height =
        (content.height() * 28.0 / 704.0).clamp(title_height * 1.20, title_height * 1.52);
    let header_rect =
        egui::Rect::from_min_size(content.min, egui::vec2(content.width(), header_height));
    let workspace = egui::Rect::from_min_max(
        egui::pos2(content.left(), header_rect.bottom() + gap),
        content.right_bottom(),
    );

    ui.painter()
        .rect_filled(header_rect, 2.0, editor_theme::semantic().chrome);
    with_child(
        ui,
        header_rect.shrink2(egui::vec2(
            editor_theme::space::XS,
            editor_theme::space::XXS,
        )),
        "header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| header::draw(ui, state, &mut history, &mut presets, &mut settings_open),
    );

    let usable_width = (workspace.width() - section_gap).max(1.0);
    let left_width = usable_width * 0.72;
    let left = egui::Rect::from_min_size(workspace.min, egui::vec2(left_width, workspace.height()));
    let right = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, workspace.top()),
        workspace.right_bottom(),
    );
    crate::editor_generator::show(ui, state, left, gap, section_gap);

    let performance_height = crate::editor_performance::preferred_height(ui)
        .min((right.height() - section_gap).max(editor_theme::shape::STROKE));
    let performance_rect = egui::Rect::from_min_size(
        egui::pos2(right.left(), right.bottom() - performance_height),
        egui::vec2(right.width(), performance_height),
    );
    let lfo_rect = egui::Rect::from_min_max(
        right.min,
        egui::pos2(
            right.right(),
            (performance_rect.top() - section_gap).max(right.top()),
        ),
    );
    draw_modulation(ui, state, lfo_rect);
    draw_performance(ui, state, performance_rect);
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
    if ui.input(|input| input.pointer.any_released()) {
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

fn ensure_icon_font(ui: &egui::Ui) {
    let id = egui::Id::new("kurv-phosphor-font-ready");
    if ui.data(|data| data.get_temp::<u64>(id)).is_some() {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "kurv-inter".to_owned(),
        egui::FontData::from_static(ttf_inter::REGULAR).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "kurv-inter".to_owned());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ui.ctx().set_fonts(fonts);
    let frame = ui.ctx().cumulative_frame_nr();
    ui.data_mut(|data| data.insert_temp(id, frame));
    ui.ctx().request_repaint();
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
    with_child(
        ui,
        rect,
        "modulation",
        egui::Layout::top_down(egui::Align::Min),
        |ui| crate::editor_lfo::modulation_view(ui, state, rect.width(), rect.height()),
    );
}

fn draw_performance(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    let palette = editor_theme::semantic();
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        palette.surface.gamma_multiply(0.72),
    );
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.56),
        ),
    );
    let inner = rect.shrink(editor_theme::space::XS);
    with_child(
        ui,
        inner,
        "performance",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing =
                egui::vec2(editor_theme::compact_gap(ui), editor_theme::compact_gap(ui));
            ui.spacing_mut().button_padding =
                egui::vec2(editor_theme::space::XS, editor_theme::space::XXS);
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.active.weak_bg_fill = palette.control;
            crate::editor_performance::performance_view(ui, state, inner.width(), inner.height());
        },
    );
}
