use std::hash::Hash;
use std::time::Duration;

use truce_core::editor::PluginContext;

use crate::editor_controls::{
    fit_font_to_width, paint_metric_readout, paint_metric_readout_response,
};
use crate::editor_history::EditorHistory;
use crate::editor_oscillator::{
    antialiasing_selector_compact, oscillator_waveform_view, quality_selector_compact,
};
use crate::editor_presets::{PresetEntry, PresetStore};
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, normalized_unison_rate,
    paint_vertical_selector, vertical_selector_value,
};
use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, OscillatorSlot, Patch,
};
use crate::modulators::routing::{GroupControl, ModulationRouteTarget, OscillatorControl};
use crate::voices::SwarmMode;
use crate::{KurvParams, editor, editor_theme, performance};

const UI_BUILD_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Default)]
struct ThemeUi {
    library: Option<editor_theme::ThemeLibrary>,
    save_name: String,
    error: Option<String>,
    dirty_since: Option<f64>,
}

impl ThemeUi {
    fn ensure_library(&mut self, initial: editor_theme::ThemeSettings) {
        if self.library.is_none() && self.error.is_none() {
            match editor_theme::ThemeLibrary::load(initial) {
                Ok(library) => self.library = Some(library),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    fn active_settings(
        &mut self,
        fallback: editor_theme::ThemeSettings,
    ) -> editor_theme::ThemeSettings {
        self.ensure_library(fallback);
        self.library
            .as_ref()
            .and_then(editor_theme::ThemeLibrary::active_settings)
            .unwrap_or(fallback)
    }

    fn changed(&mut self, settings: editor_theme::ThemeSettings, now: f64, ctx: &egui::Context) {
        if let Some(library) = self.library.as_mut() {
            library.update_active(settings);
            self.dirty_since = Some(now);
            ctx.request_repaint_after(Duration::from_millis(320));
        }
    }

    fn flush(&mut self, now: f64, force: bool) {
        let due = self
            .dirty_since
            .is_some_and(|changed| force || now - changed >= 0.30);
        if !due {
            return;
        }
        if let Some(library) = self.library.as_ref() {
            match library.write() {
                Ok(()) => {
                    self.dirty_since = None;
                    self.error = None;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
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

    fn entries(&mut self) -> Vec<PresetEntry> {
        self.ensure_store();
        let Some(store) = self.store.as_mut() else {
            return Vec::new();
        };
        match store.entries() {
            Ok(entries) => entries.to_vec(),
            Err(error) => {
                self.error = Some(error.to_string());
                Vec::new()
            }
        }
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    ensure_icon_font(ui);
    let persisted_theme = state.params().editor_state.lock().map_or_else(
        |_| editor_theme::ThemeSettings::default(),
        |mut editor| {
            let theme = editor_theme::ThemeSettings::from_persisted(
                editor.theme_schema,
                editor.theme_preset,
                [
                    editor.background_red,
                    editor.background_green,
                    editor.background_blue,
                ],
                editor.theme_tint,
                editor.theme_contrast,
                [
                    editor.primary_red,
                    editor.primary_green,
                    editor.primary_blue,
                ],
                [
                    editor.secondary_red,
                    editor.secondary_green,
                    editor.secondary_blue,
                ],
                [
                    editor.tertiary_red,
                    editor.tertiary_green,
                    editor.tertiary_blue,
                ],
            );
            if editor.theme_schema < 2 {
                editor.theme_schema = 2;
                [
                    editor.background_red,
                    editor.background_green,
                    editor.background_blue,
                ] = theme.background_rgb;
                editor.theme_tint = theme.tint;
                editor.theme_contrast = theme.contrast;
                [
                    editor.secondary_red,
                    editor.secondary_green,
                    editor.secondary_blue,
                ] = theme.secondary_rgb;
                [
                    editor.tertiary_red,
                    editor.tertiary_green,
                    editor.tertiary_blue,
                ] = theme.tertiary_rgb;
            }
            theme
        },
    );
    let theme_id = egui::Id::new("kurv-theme-ui");
    let mut themes = ui
        .data_mut(|data| data.remove_temp::<ThemeUi>(theme_id))
        .unwrap_or_default();
    let theme = themes.active_settings(persisted_theme);
    if theme != persisted_theme {
        write_editor_theme(state, theme);
    }
    editor_theme::apply_with(ui, theme);
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
    let header_height = (content.height() * 44.0 / 704.0).clamp(
        editor_theme::control_height(ui),
        editor_theme::control_height(ui) + editor_theme::space::MD,
    );
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
        header_rect.shrink2(egui::vec2(editor_theme::space::SM, editor_theme::space::XS)),
        "header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| draw_header(ui, state, &mut history, &mut presets, &mut settings_open),
    );

    let usable_width = (workspace.width() - section_gap).max(1.0);
    let left_width = usable_width * 0.72;
    let left = egui::Rect::from_min_size(workspace.min, egui::vec2(left_width, workspace.height()));
    let right = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, workspace.top()),
        workspace.right_bottom(),
    );
    draw_generator_group(ui, state, left, gap, section_gap);

    let stacked_height = (right.height() - section_gap).max(1.0);
    let performance_height = (editor_theme::title_height(ui)
        + editor_theme::control_height(ui) * 3.0
        + editor_theme::space::SM * 4.0)
        .min(stacked_height * 0.42)
        .max(editor_theme::title_height(ui) * 4.5);
    let modulation_max = (stacked_height - performance_height - section_gap).max(1.0);
    let modulation_height =
        crate::editor_lfo::preferred_height(ui, state, modulation_max).min(modulation_max);
    let lfo_rect =
        egui::Rect::from_min_size(right.min, egui::vec2(right.width(), modulation_height));
    let performance_rect = egui::Rect::from_min_size(
        egui::pos2(right.left(), lfo_rect.bottom() + section_gap),
        egui::vec2(right.width(), performance_height),
    );
    draw_modulation(ui, state, lfo_rect);
    draw_performance(ui, state, performance_rect);
    if !settings_open && !presets.save_open {
        crate::editor_modulation::draw_overlay(ui, state);
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
        draw_settings_panel(
            ui,
            state,
            workspace,
            &mut settings_open,
            &mut themes,
            popup_was_open,
        );
    }
    if ui.input(|input| input.pointer.any_released()) {
        presets.dirty |= history.commit(state);
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
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ui.ctx().set_fonts(fonts);
    let frame = ui.ctx().cumulative_frame_nr();
    ui.data_mut(|data| data.insert_temp(id, frame));
    ui.ctx().request_repaint();
}

fn icon_font_ready(ui: &egui::Ui) -> bool {
    let id = egui::Id::new("kurv-phosphor-font-ready");
    ui.data(|data| data.get_temp::<u64>(id))
        .is_some_and(|registered| registered < ui.ctx().cumulative_frame_nr())
}

fn draw_header(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    history: &mut EditorHistory,
    presets: &mut PresetUi,
    settings_open: &mut bool,
) {
    let rect = ui.max_rect();
    let width = rect.width();
    let unit = editor_theme::title_height(ui);
    let section_gap = editor_theme::space::XS;
    let compact = width < unit * 38.0;
    let left_width = (width * 0.24)
        .clamp(unit * 7.0, unit * if compact { 9.0 } else { 12.0 })
        .min(width * 0.30);
    let right_width = (width * 0.18)
        .clamp(unit * 6.4, unit * if compact { 7.0 } else { 10.0 })
        .min(width * 0.24);
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
            if ui
                .add_sized(
                    [unit * 1.4, unit * 1.4],
                    egui::Button::new(
                        egui::RichText::new(if icon_font_ready(ui) {
                            egui_phosphor::regular::GEAR
                        } else {
                            "⚙"
                        })
                        .font(egui::FontId::proportional(
                            editor_theme::font::TITLE_SIZE * 1.35,
                        )),
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
                        egui::RichText::new("KURV")
                            .font(egui::FontId::proportional(if width < unit * 35.0 {
                                editor_theme::font::TITLE_SIZE * 1.6
                            } else {
                                editor_theme::font::TITLE_SIZE * 1.95
                            }))
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
            if left.width() >= unit * 8.5 {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        egui::RichText::new("PROCEDURAL VA")
                            .font(editor_theme::font::label())
                            .color(editor_theme::semantic().text),
                    );
                    ui.label(
                        egui::RichText::new(UI_BUILD_VERSION)
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                });
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
            editor::output_meter(ui, state, right.width(), right.height().max(unit * 1.2));
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
    let control_height = editor_theme::control_height(ui).min(ui.available_height());
    let nav_width = control_height * if cramped { 0.55 } else { 0.72 };
    let default_width = unit * if compact { 2.2 } else { 5.4 };
    let save_width = unit
        * if cramped {
            2.0
        } else if compact {
            2.5
        } else {
            3.8
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

    let entries = presets.entries();
    let mut chosen = None;
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
            for entry in entries {
                if ui
                    .selectable_label(presets.selected == entry.name(), entry.name())
                    .clicked()
                {
                    chosen = Some(entry);
                }
            }
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

fn read_editor_theme(state: &PluginContext<KurvParams>) -> editor_theme::ThemeSettings {
    state.params().editor_state.lock().map_or_else(
        |_| editor_theme::ThemeSettings::default(),
        |editor| {
            editor_theme::ThemeSettings::from_persisted(
                editor.theme_schema,
                editor.theme_preset,
                [
                    editor.background_red,
                    editor.background_green,
                    editor.background_blue,
                ],
                editor.theme_tint,
                editor.theme_contrast,
                [
                    editor.primary_red,
                    editor.primary_green,
                    editor.primary_blue,
                ],
                [
                    editor.secondary_red,
                    editor.secondary_green,
                    editor.secondary_blue,
                ],
                [
                    editor.tertiary_red,
                    editor.tertiary_green,
                    editor.tertiary_blue,
                ],
            )
        },
    )
}

fn write_editor_theme(state: &PluginContext<KurvParams>, settings: editor_theme::ThemeSettings) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.theme_schema = 2;
        [
            editor.background_red,
            editor.background_green,
            editor.background_blue,
        ] = settings.background_rgb;
        editor.theme_tint = settings.tint;
        editor.theme_contrast = settings.contrast;
        [
            editor.primary_red,
            editor.primary_green,
            editor.primary_blue,
        ] = settings.primary_rgb;
        [
            editor.secondary_red,
            editor.secondary_green,
            editor.secondary_blue,
        ] = settings.secondary_rgb;
        [
            editor.tertiary_red,
            editor.tertiary_green,
            editor.tertiary_blue,
        ] = settings.tertiary_rgb;
    }
}

fn draw_settings_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    workspace: egui::Rect,
    open: &mut bool,
    themes: &mut ThemeUi,
    popup_was_open: bool,
) {
    let available = workspace.shrink(editor_theme::space::SM);
    let title_height = editor_theme::title_height(ui);
    let size = egui::vec2(
        (available.width() * 0.42)
            .clamp(title_height * 12.0, title_height * 18.0)
            .min(available.width()),
        (available.height() * 0.72)
            .max(title_height * 10.0)
            .min(available.height()),
    );
    let rect = egui::Rect::from_center_size(available.center(), size);
    ui.painter().rect(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().surface,
        egui::Stroke::new(editor_theme::shape::STROKE, editor_theme::semantic().grid),
        egui::StrokeKind::Inside,
    );
    let panel = rect.shrink(editor_theme::space::SM);
    let header_height = title_height.min(panel.height());
    let header = egui::Rect::from_min_size(panel.min, egui::vec2(panel.width(), header_height));
    with_child(
        ui,
        header,
        "settings-header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new("SETTINGS")
                    .font(editor_theme::font::title())
                    .color(editor_theme::semantic().text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized(
                        [header_height, header_height],
                        egui::Button::new(
                            egui::RichText::new(if icon_font_ready(ui) {
                                egui_phosphor::regular::X
                            } else {
                                "×"
                            })
                            .font(editor_theme::font::title())
                            .color(editor_theme::semantic().text_muted),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Close settings")
                    .clicked()
                {
                    *open = false;
                }
            });
        },
    );
    ui.painter().line_segment(
        [
            egui::pos2(panel.left(), header.bottom()),
            egui::pos2(panel.right(), header.bottom()),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().grid.gamma_multiply(0.55),
        ),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(
            panel.left(),
            header.bottom() + editor_theme::compact_gap(ui),
        ),
        panel.right_bottom(),
    );
    with_child(
        ui,
        body,
        "settings-body",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width((body.width() - editor_theme::space::SM).max(1.0));
                    ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
                    let mut editor = state
                        .params()
                        .editor_state
                        .lock()
                        .map_or_else(|_| crate::KurvEditorState::default(), |value| value.clone());
                    let mut changed = false;
                    ui.label(
                        egui::RichText::new("APPEARANCE")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    if let Some(library) = themes.library.as_mut() {
                        let names = library.names();
                        let active = library.active_name().to_owned();
                        let mut selected = None;
                        ui.horizontal(|ui| {
                            ui.label("Theme");
                            egui::ComboBox::from_id_salt("kurv-theme-picker")
                                .selected_text(&active)
                                .show_ui(ui, |ui| {
                                    for name in names {
                                        if ui.selectable_label(name == active, &name).clicked() {
                                            selected = Some(name);
                                        }
                                    }
                                });
                        });
                        if let Some(name) = selected {
                            match library.select(&name) {
                                Ok(settings) => {
                                    write_editor_theme(state, settings);
                                    themes.error = None;
                                    editor_theme::request_display_repaint(ui);
                                }
                                Err(error) => themes.error = Some(error.to_string()),
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut themes.save_name)
                                    .hint_text("Theme name")
                                    .desired_width(body.width() * 0.46),
                            );
                            if ui.button("SAVE AS").clicked() {
                                let settings = read_editor_theme(state);
                                match library.save_as(&themes.save_name, settings) {
                                    Ok(()) => {
                                        themes.save_name.clear();
                                        themes.error = None;
                                    }
                                    Err(error) => themes.error = Some(error.to_string()),
                                }
                            }
                        });
                        ui.label(
                            egui::RichText::new("Theme edits are saved automatically")
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                        );
                    }
                    if let Some(error) = themes.error.as_deref() {
                        ui.label(
                            egui::RichText::new(error)
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().danger),
                        );
                    }
                    egui::Grid::new("theme-colors")
                        .num_columns(2)
                        .spacing(egui::vec2(editor_theme::space::MD, editor_theme::space::XS))
                        .show(ui, |ui| {
                            for (label, rgb) in [
                                (
                                    "Background",
                                    &mut [
                                        editor.background_red,
                                        editor.background_green,
                                        editor.background_blue,
                                    ],
                                ),
                                (
                                    "Primary",
                                    &mut [
                                        editor.primary_red,
                                        editor.primary_green,
                                        editor.primary_blue,
                                    ],
                                ),
                                (
                                    "Unison",
                                    &mut [
                                        editor.secondary_red,
                                        editor.secondary_green,
                                        editor.secondary_blue,
                                    ],
                                ),
                                (
                                    "Envelope / Pan Shape",
                                    &mut [
                                        editor.tertiary_red,
                                        editor.tertiary_green,
                                        editor.tertiary_blue,
                                    ],
                                ),
                            ] {
                                ui.label(label);
                                if ui.color_edit_button_srgb(rgb).changed() {
                                    changed = true;
                                }
                                match label {
                                    "Background" => {
                                        [
                                            editor.background_red,
                                            editor.background_green,
                                            editor.background_blue,
                                        ] = *rgb;
                                    }
                                    "Primary" => {
                                        [
                                            editor.primary_red,
                                            editor.primary_green,
                                            editor.primary_blue,
                                        ] = *rgb;
                                    }
                                    "Unison" => {
                                        [
                                            editor.secondary_red,
                                            editor.secondary_green,
                                            editor.secondary_blue,
                                        ] = *rgb;
                                    }
                                    _ => {
                                        [
                                            editor.tertiary_red,
                                            editor.tertiary_green,
                                            editor.tertiary_blue,
                                        ] = *rgb;
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    let mut tint = f32::from(editor.theme_tint);
                    let mut contrast = f32::from(editor.theme_contrast);
                    changed |= ui
                        .add(egui::Slider::new(&mut tint, 0.0..=100.0).text("Tint"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut contrast, 50.0..=175.0).text("Contrast"))
                        .changed();
                    editor.theme_tint = tint.round() as u8;
                    editor.theme_contrast = contrast.round() as u8;
                    if changed {
                        editor.theme_schema = 2;
                        let settings = editor_theme::ThemeSettings::from_persisted(
                            editor.theme_schema,
                            editor.theme_preset,
                            [
                                editor.background_red,
                                editor.background_green,
                                editor.background_blue,
                            ],
                            editor.theme_tint,
                            editor.theme_contrast,
                            [
                                editor.primary_red,
                                editor.primary_green,
                                editor.primary_blue,
                            ],
                            [
                                editor.secondary_red,
                                editor.secondary_green,
                                editor.secondary_blue,
                            ],
                            [
                                editor.tertiary_red,
                                editor.tertiary_green,
                                editor.tertiary_blue,
                            ],
                        );
                        if let Ok(mut stored) = state.params().editor_state.lock() {
                            *stored = editor;
                        }
                        themes.changed(settings, ui.input(|input| input.time), ui.ctx());
                        editor_theme::request_display_repaint(ui);
                    }

                    ui.add_space(editor_theme::space::SM);
                    ui.label(
                        egui::RichText::new("GENERATOR QUALITY")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    let selector_gap = editor_theme::space::XS;
                    let selector_width = ((body.width() - selector_gap) * 0.5)
                        .max(editor_theme::title_height(ui) * 4.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = selector_gap;
                        antialiasing_selector_compact(ui, state, selector_width);
                        quality_selector_compact(ui, state, selector_width);
                    });
                    ui.add_space(editor_theme::space::SM);
                    ui.label(
                        egui::RichText::new("CPU OPTIMIZATION")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !performance::calibration_running(),
                                egui::Button::new("CALIBRATE"),
                            )
                            .on_hover_text(
                                "Benchmarks equivalent spline kernels off the audio thread",
                            )
                            .clicked()
                        {
                            performance::start_calibration();
                            editor_theme::request_display_repaint(ui);
                        }
                        if ui
                            .add_enabled(
                                !performance::calibration_running(),
                                egui::Button::new("RESET AUTO"),
                            )
                            .clicked()
                        {
                            performance::reset_auto();
                        }
                    });
                    ui.label(
                        egui::RichText::new(performance::status_text())
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    if performance::calibration_running() {
                        editor_theme::request_display_repaint(ui);
                    }
                    ui.add_space(editor_theme::compact_gap(ui));
                    ui.label(
                        egui::RichText::new("Window resize and host DPI remain independent.")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
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
        *open = false;
    }
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

fn group_accent(group_id: GroupId) -> egui::Color32 {
    let palette = editor_theme::semantic();
    let accents = [
        palette.primary,
        palette.unison,
        palette.pan_shape,
        crate::editor_modulation::source_color(0),
        crate::editor_modulation::source_color(1),
        crate::editor_modulation::source_color(2),
        crate::editor_modulation::source_color(3),
        crate::editor_modulation::source_color(5),
    ];
    let index = group_id.get().wrapping_mul(0x9E37_79B9) as usize % accents.len();
    accents[index]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GeneratorInsertionTarget {
    Group(usize),
    Module(u64, usize),
}

#[derive(Clone, Copy)]
struct GeneratorInsertionCandidate {
    target: GeneratorInsertionTarget,
    left: f32,
    right: f32,
    edge: f32,
}

fn generator_insertion_candidates_id() -> egui::Id {
    egui::Id::new("generator-insertion-candidates")
}

fn generator_insertion_menu_id(target: GeneratorInsertionTarget) -> egui::Id {
    match target {
        GeneratorInsertionTarget::Group(insertion) => {
            egui::Id::new(("generator-stack-insert-menu", insertion))
        }
        GeneratorInsertionTarget::Module(group, insertion) => {
            egui::Id::new(("generator-module-insert-menu", group, insertion))
        }
    }
}

fn generator_root_menu_id() -> egui::Id {
    egui::Id::new("generator-add-menu-root")
}

fn register_generator_insertion(
    ui: &mut egui::Ui,
    target: GeneratorInsertionTarget,
    left: f32,
    right: f32,
    edge: f32,
) {
    if edge < ui.clip_rect().top() || edge > ui.clip_rect().bottom() {
        return;
    }
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<GeneratorInsertionCandidate>>(
            generator_insertion_candidates_id(),
        )
        .push(GeneratorInsertionCandidate {
            target,
            left,
            right,
            edge,
        });
    });
}

fn active_generator_insertion(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
) -> Option<GeneratorInsertionTarget> {
    let candidates = ui
        .data(|data| {
            data.get_temp::<Vec<GeneratorInsertionCandidate>>(generator_insertion_candidates_id())
        })
        .unwrap_or_default();
    ui.data_mut(|data| {
        data.insert_temp(
            generator_insertion_candidates_id(),
            Vec::<GeneratorInsertionCandidate>::new(),
        );
    });

    if let Some(open) = ui.data(|data| {
        candidates.iter().find_map(|candidate| {
            data.get_temp::<bool>(generator_insertion_menu_id(candidate.target))
                .unwrap_or(false)
                .then_some(candidate.target)
        })
    }) {
        return Some(open);
    }

    let pointer = ui.input(|input| {
        (input.modifiers.alt
            && !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx())
            && !egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx())
            && !crate::editor_modulation::source_drag_active(ui))
        .then(|| input.pointer.latest_pos())
        .flatten()
    })?;
    if !viewport.contains(pointer) {
        return None;
    }

    let activation_radius = editor_theme::title_height(ui) * 0.72;
    candidates
        .iter()
        .filter(|candidate| (candidate.left..=candidate.right).contains(&pointer.x))
        .filter(|candidate| (candidate.edge - pointer.y).abs() <= activation_radius)
        .min_by(|left, right| {
            (left.edge - pointer.y)
                .abs()
                .total_cmp(&(right.edge - pointer.y).abs())
        })
        .map(|candidate| candidate.target)
}

fn draw_generator_group(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    let patch = state.generator_stack.snapshot();
    let root_menu_id = generator_root_menu_id();
    let root_menu_open = ui
        .data(|data| data.get_temp::<bool>(root_menu_id))
        .unwrap_or(false);
    let active_insertion = active_generator_insertion(ui, rect).filter(|_| !root_menu_open);
    let compact_text_height = editor_theme::font::caption().size + editor_theme::font::value().size;
    let card_height = (rect.width() * 0.23)
        .min(rect.height() * 0.52)
        .max(compact_text_height * 5.4);
    let output_height = (card_height * 0.16).max(compact_text_height * 1.55);
    with_child(
        ui,
        rect,
        "generator-groups",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("generator-groups-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let keep_rack_interactions_alive = active_insertion.is_some()
                        || root_menu_open
                        || egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx())
                        || egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx())
                        || ui.ctx().dragged_id().is_some()
                        || ui.ctx().any_popup_open();
                    for (group_index, group) in patch.groups().iter().enumerate() {
                        draw_generator_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_index,
                            active_insertion,
                            card_height,
                        );
                        let group_id = group.id();
                        let group_accent = group_accent(group_id);
                        let modules: Vec<_> = group
                            .modules()
                            .iter()
                            .filter_map(|module| {
                                module.oscillator_slot().map(|slot| (module.id(), slot))
                            })
                            .collect();
                        let mut collapsed =
                            state.params().editor_state.lock().is_ok_and(|editor| {
                                editor.collapsed_group_ids.contains(&group_id.get())
                            });
                        let group_top = ui.cursor().top();
                        let module_insertions = if collapsed {
                            usize::from(generator_insertion_visible(
                                ui,
                                GeneratorInsertionTarget::Module(group_id.get(), modules.len()),
                                active_insertion,
                            ))
                        } else {
                            (0..=modules.len())
                                .filter(|insertion| {
                                    generator_insertion_visible(
                                        ui,
                                        GeneratorInsertionTarget::Module(
                                            group_id.get(),
                                            *insertion,
                                        ),
                                        active_insertion,
                                    )
                                })
                                .count()
                        };
                        let group_height = if collapsed {
                            0.0
                        } else {
                            card_height * modules.len() as f32
                        } + editor_theme::title_height(ui)
                            * module_insertions as f32
                            + section_gap * 0.35
                            + output_height;
                        let group_background = egui::Rect::from_min_size(
                            egui::pos2(ui.cursor().left(), group_top),
                            egui::vec2(ui.available_width(), group_height),
                        );
                        let group_visible = rack_item_visible(ui, group_background);
                        if group_visible {
                            ui.painter().rect_filled(
                                group_background,
                                editor_theme::shape::CONTROL_RADIUS,
                                translucent(editor_theme::semantic().surface, 112),
                            );
                        }

                        if !collapsed {
                            for (visible, (module_id, slot)) in modules.iter().enumerate() {
                                draw_group_module_insert_zone(
                                    ui,
                                    state,
                                    &patch,
                                    group_id,
                                    visible,
                                    active_insertion,
                                    card_height,
                                );
                                let (_, card) = ui
                                    .allocate_space(egui::vec2(ui.available_width(), card_height));
                                if rack_item_visible(ui, card) || keep_rack_interactions_alive {
                                    draw_compact_oscillator(
                                        ui, state, card, *slot, *module_id, gap,
                                    );
                                }
                            }
                        }
                        draw_group_module_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_id,
                            modules.len(),
                            active_insertion,
                            card_height,
                        );

                        ui.add_space(section_gap * 0.35);
                        let (_, footer) =
                            ui.allocate_space(egui::vec2(ui.available_width(), output_height));
                        let interaction =
                            if rack_item_visible(ui, footer) || keep_rack_interactions_alive {
                                draw_group_output(
                                    ui,
                                    state,
                                    footer,
                                    group_id,
                                    group_index,
                                    patch.groups().len() > 1,
                                    modules.len(),
                                    collapsed,
                                    group.output(),
                                    group_accent,
                                )
                            } else {
                                GroupOutputInteraction::default()
                            };
                        if interaction.toggle_collapse {
                            collapsed = !collapsed;
                            if let Ok(mut editor) = state.params().editor_state.lock() {
                                if collapsed {
                                    if !editor.collapsed_group_ids.contains(&group_id.get()) {
                                        editor.collapsed_group_ids.push(group_id.get());
                                    }
                                } else {
                                    editor
                                        .collapsed_group_ids
                                        .retain(|id| *id != group_id.get());
                                }
                            }
                        }

                        let group_rect = egui::Rect::from_min_max(
                            egui::pos2(footer.left(), group_top),
                            footer.right_bottom(),
                        );
                        if group_visible || keep_rack_interactions_alive {
                            if interaction.dragging {
                                ui.painter().rect_filled(
                                    group_rect.shrink(editor_theme::shape::STROKE),
                                    editor_theme::shape::CONTROL_RADIUS,
                                    translucent(editor_theme::semantic().chrome, 156),
                                );
                            }
                            ui.painter().rect_stroke(
                                group_rect,
                                2.0,
                                egui::Stroke::new(
                                    if interaction.dragging || interaction.pressed {
                                        editor_theme::shape::FOCUS_STROKE
                                    } else {
                                        editor_theme::shape::STROKE
                                    },
                                    group_accent.gamma_multiply(if interaction.dragging {
                                        0.88
                                    } else if interaction.pressed {
                                        0.76
                                    } else if interaction.hovered {
                                        0.62
                                    } else {
                                        0.48
                                    }),
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }

                        if interaction.remove {
                            remove_generator_group(state, group_id, &modules);
                        }
                    }
                    draw_generator_insert_zone(
                        ui,
                        state,
                        &patch,
                        patch.groups().len(),
                        active_insertion,
                        card_height,
                    );
                    let next_oscillator = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    if active_insertion.is_none() {
                        if let Some(action) = draw_generator_add_menu(
                            ui,
                            root_menu_id,
                            next_oscillator.is_some(),
                            patch.groups().len() < MAX_OUTPUT_PAIRS,
                        ) {
                            match action {
                                GeneratorAddAction::Oscillator => {
                                    if let Some(slot) = next_oscillator {
                                        add_oscillator_to_new_group(
                                            state,
                                            slot,
                                            patch.groups().len(),
                                        );
                                    }
                                }
                                GeneratorAddAction::Group => {
                                    add_generator_group(state, patch.groups().len());
                                }
                            }
                        }
                    }
                });
        },
    );
}

fn generator_insertion_visible(
    ui: &egui::Ui,
    target: GeneratorInsertionTarget,
    active: Option<GeneratorInsertionTarget>,
) -> bool {
    active == Some(target)
        || ui
            .data(|data| data.get_temp::<bool>(generator_insertion_menu_id(target)))
            .unwrap_or(false)
}

fn draw_generator_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    card_height: f32,
) {
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let group_drag = egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx());
    let target_id = GeneratorInsertionTarget::Group(insertion);
    let menu_id = generator_insertion_menu_id(target_id);
    let menu_open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    let lane_right = (ui.cursor().left() + outside_lane_width).min(ui.cursor().right());
    let has_trailing_add = insertion == patch.groups().len();
    if !has_trailing_add {
        register_generator_insertion(ui, target_id, ui.cursor().left(), lane_right, edge);
    }
    if !has_trailing_add && (active_insertion == Some(target_id) || menu_open) {
        let (button_rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_generator_add_button(ui, button_rect, &response, true, menu_open);
        if let Some(action) = generator_add_popup(
            ui,
            menu_id,
            button_rect,
            &response,
            patch.oscillator_count() < MAX_OSCILLATORS,
            patch.groups().len() < MAX_OUTPUT_PAIRS,
        ) {
            match action {
                GeneratorAddAction::Oscillator => {
                    let next = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    if let Some(slot) = next {
                        add_oscillator_to_new_group(state, slot, insertion);
                    }
                }
                GeneratorAddAction::Group => add_generator_group(state, insertion),
            }
        }
        return;
    }
    if !module_drag && !group_drag {
        return;
    }

    let target = egui::Rect::from_min_max(
        egui::pos2(ui.cursor().left(), edge - row_height * 0.50),
        egui::pos2(ui.cursor().right(), edge + row_height * 0.50),
    );
    let outside_target = egui::Rect::from_min_max(
        target.min,
        egui::pos2(
            (target.left() + outside_lane_width).min(target.right()),
            target.bottom(),
        ),
    );
    let group_response = ui
        .interact(
            target,
            egui::Id::new(("generator-group-stack-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_response = ui
        .interact(
            outside_target,
            egui::Id::new(("generator-module-new-group-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_hovered = module_response.dnd_hover_payload::<ModuleId>().is_some();
    let group_hovered = group_response.dnd_hover_payload::<GroupId>().is_some();
    let placeholder_id = egui::Id::new(("generator-new-group-placeholder", insertion));
    let placeholder_open = module_drag
        && ui
            .data(|data| data.get_temp::<bool>(placeholder_id))
            .unwrap_or(false);
    let module_at_capacity =
        (module_hovered || placeholder_open) && patch.groups().len() >= MAX_OUTPUT_PAIRS;
    let color = if module_at_capacity {
        editor_theme::semantic().text_muted
    } else {
        editor_theme::semantic().primary
    };
    let show_placeholder = !module_at_capacity && (module_hovered || placeholder_open);
    let mut placeholder_release = None;
    if show_placeholder {
        let (placeholder, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), card_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::Grabbing);
        placeholder_release = response.dnd_release_payload::<ModuleId>();
        paint_generator_drop_placeholder(
            ui,
            placeholder,
            color,
            "DROP OSCILLATOR · NEW GROUP",
            row_height,
        );
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || outside_target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
    let group_placeholder_id = egui::Id::new(("generator-group-placeholder", insertion));
    let group_placeholder_open = group_drag
        && ui
            .data(|data| data.get_temp::<bool>(group_placeholder_id))
            .unwrap_or(false);
    let mut group_placeholder_release = None;
    if group_hovered || group_placeholder_open {
        let (placeholder, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::Grabbing);
        group_placeholder_release = response.dnd_release_payload::<GroupId>();
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP GROUP", row_height);
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, false));
    }
    if module_at_capacity {
        let line_inset = target.width() * 0.012;
        ui.painter().text(
            target.right_center() - egui::vec2(line_inset, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("{MAX_OUTPUT_PAIRS} GROUP LIMIT"),
            editor_theme::font::caption(),
            color,
        );
    }
    if let Some(module_id) =
        placeholder_release.or_else(|| module_response.dnd_release_payload::<ModuleId>())
        && patch.groups().len() < MAX_OUTPUT_PAIRS
    {
        move_module_to_new_group(state, *module_id, insertion);
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    } else if let Some(group_id) =
        group_placeholder_release.or_else(|| group_response.dnd_release_payload::<GroupId>())
    {
        move_group_to_insertion(state, patch, *group_id, insertion);
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, false));
    }
}

fn move_module_to_new_group(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    insertion: usize,
) {
    state.generator_stack.edit(|patch| {
        let module_exists = patch.groups().iter().any(|group| {
            group
                .modules()
                .iter()
                .any(|module| module.id() == module_id)
        });
        if !module_exists {
            return;
        }
        if let Ok(group_id) = patch.insert_group(insertion) {
            let output = GroupOutput {
                pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                ..GroupOutput::default()
            };
            let _ = patch.set_group_output(group_id, output);
            if patch.move_module(module_id, group_id, 0).is_err() {
                let _ = patch.remove_group(group_id);
            }
        }
    });
}

fn draw_group_module_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    card_height: f32,
) {
    let alt_held = ui.input(|input| input.modifiers.alt);
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let target_id = GeneratorInsertionTarget::Module(group_id.get(), insertion);
    let menu_id = generator_insertion_menu_id(target_id);
    let menu_open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    let lane_left = (ui.cursor().left() + outside_lane_width).min(ui.cursor().right());
    register_generator_insertion(ui, target_id, lane_left, ui.cursor().right(), edge);
    if active_insertion == Some(target_id) || menu_open {
        let (button_rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_generator_add_button(ui, button_rect, &response, true, menu_open);
        let next_oscillator = (0..MAX_OSCILLATORS)
            .filter_map(OscillatorSlot::from_index)
            .find(|slot| !patch.contains_oscillator_slot(*slot));
        if let Some(action) = generator_add_popup(
            ui,
            menu_id,
            button_rect,
            &response,
            next_oscillator.is_some(),
            patch.groups().len() < MAX_OUTPUT_PAIRS,
        ) {
            match action {
                GeneratorAddAction::Oscillator => {
                    if let Some(slot) = next_oscillator {
                        add_oscillator_to_group(state, group_id, insertion, slot);
                    }
                }
                GeneratorAddAction::Group => {
                    state.generator_stack.edit(|patch| {
                        let _ = patch.split_group_at(group_id, insertion);
                    });
                }
            }
        }
        return;
    }
    if !module_drag {
        return;
    }

    let target = egui::Rect::from_min_max(
        egui::pos2(ui.cursor().left(), edge - row_height * 0.50),
        egui::pos2(ui.cursor().right(), edge + row_height * 0.50),
    );
    let inside_target = egui::Rect::from_min_max(
        egui::pos2(
            (target.left() + outside_lane_width).min(target.right()),
            target.top(),
        ),
        target.max,
    );
    let response = ui
        .interact(
            inside_target,
            egui::Id::new(("generator-module-insert", group_id.get(), insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let hovered_module = response.dnd_hover_payload::<ModuleId>();
    let dragged_module = egui::DragAndDrop::payload::<ModuleId>(ui.ctx());
    let source_group = dragged_module.as_deref().and_then(|module_id| {
        patch.groups().iter().find_map(|group| {
            group
                .modules()
                .iter()
                .any(|module| module.id() == *module_id)
                .then_some(group.id())
        })
    });
    let valid = source_group.is_some_and(|source| source == group_id || alt_held);
    let needs_alt = hovered_module.is_some()
        && source_group.is_some_and(|source| source != group_id)
        && !alt_held;
    let color = group_accent(group_id);
    let placeholder_id = egui::Id::new(("generator-module-placeholder", group_id.get(), insertion));
    let placeholder_open = module_drag
        && ui
            .data(|data| data.get_temp::<bool>(placeholder_id))
            .unwrap_or(false);
    let mut placeholder_release = None;
    if valid && (hovered_module.is_some() || placeholder_open) {
        let (placeholder, placeholder_response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), card_height),
            egui::Sense::click(),
        );
        let placeholder_response = placeholder_response.on_hover_cursor(egui::CursorIcon::Grabbing);
        placeholder_release = placeholder_response.dnd_release_payload::<ModuleId>();
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP OSCILLATOR", row_height);
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || inside_target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
    if needs_alt {
        let muted = editor_theme::semantic().text_muted;
        ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        ui.painter()
            .rect_filled(inside_target, 1.0, translucent(color, 10));
        ui.painter().text(
            inside_target.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
            egui::Align2::RIGHT_CENTER,
            "HOLD ALT",
            editor_theme::font::caption(),
            muted,
        );
    }

    if let Some(module_id) =
        placeholder_release.or_else(|| response.dnd_release_payload::<ModuleId>())
        && valid
    {
        move_module_to_group(state, patch, *module_id, group_id, insertion);
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
}

fn paint_generator_drop_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    color: egui::Color32,
    label: &str,
    dash_unit: f32,
) {
    ui.painter().rect_filled(rect, 1.0, translucent(color, 14));
    let outline = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
        dash_unit * 0.42,
        dash_unit * 0.30,
    ));
    ui.painter().text(
        rect.left_center() + egui::vec2(editor_theme::space::SM, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        color,
    );
}

fn translucent(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn move_group_to_insertion(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
) {
    let Some(source) = patch
        .groups()
        .iter()
        .position(|group| group.id() == group_id)
    else {
        return;
    };
    let target = if source < insertion {
        insertion.saturating_sub(1)
    } else {
        insertion
    }
    .min(patch.groups().len().saturating_sub(1));
    if target != source {
        state.generator_stack.edit(|patch| {
            let _ = patch.move_group(group_id, target);
        });
    }
}

fn move_module_to_group(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    module_id: ModuleId,
    destination: GroupId,
    insertion: usize,
) {
    let Some((source_group, source_index)) = patch.groups().iter().find_map(|group| {
        group
            .modules()
            .iter()
            .position(|module| module.id() == module_id)
            .map(|index| (group.id(), index))
    }) else {
        return;
    };
    let Some(destination_len) = patch
        .groups()
        .iter()
        .find(|group| group.id() == destination)
        .map(|group| group.modules().len())
    else {
        return;
    };
    let target = if source_group == destination && source_index < insertion {
        insertion.saturating_sub(1)
    } else {
        insertion
    };
    let target = if source_group == destination {
        target.min(destination_len.saturating_sub(1))
    } else {
        target.min(destination_len)
    };
    if source_group != destination || source_index != target {
        state.generator_stack.edit(|patch| {
            let _ = patch.move_module(module_id, destination, target);
        });
    }
}

#[derive(Clone, Copy)]
enum GeneratorAddAction {
    Oscillator,
    Group,
}

fn draw_generator_add_menu(
    ui: &mut egui::Ui,
    menu_id: egui::Id,
    can_add_oscillator: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let (id, button_rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        editor_theme::title_height(ui),
    ));
    if !rack_item_visible(ui, button_rect) && !open {
        return None;
    }
    let response = ui
        .interact(button_rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_generator_add_button(ui, button_rect, &response, false, open);
    generator_add_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        can_add_group,
    )
}

fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}

fn paint_generator_add_button(
    ui: &egui::Ui,
    button_rect: egui::Rect,
    response: &egui::Response,
    insertion: bool,
    open: bool,
) {
    let palette = editor_theme::semantic();
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    if insertion || open || hovered {
        ui.painter().rect_filled(
            button_rect,
            1.0,
            if insertion {
                translucent(palette.primary, if pressed { 34 } else { 22 })
            } else if open || pressed {
                palette.control
            } else {
                palette.surface
            },
        );
    }
    ui.painter().rect_stroke(
        button_rect,
        1.0,
        egui::Stroke::new(
            if pressed || open {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if insertion || hovered || open {
                palette.primary
            } else {
                palette.grid
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        button_rect.left_center() + egui::vec2(button_rect.height() * 0.5, 0.0),
        egui::Align2::LEFT_CENTER,
        "+ ADD MODULE",
        editor_theme::font::label(),
        if insertion {
            palette.primary
        } else if hovered || open || pressed {
            palette.text
        } else {
            palette.text_muted
        },
    );
}

fn generator_add_popup(
    ui: &mut egui::Ui,
    menu_id: egui::Id,
    button_rect: egui::Rect,
    response: &egui::Response,
    can_add_oscillator: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let mut action = None;
    let mut open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let clicked = response.clicked()
        || ui.input(|input| {
            input.pointer.primary_clicked()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|pointer| response.rect.contains(pointer))
        });
    if clicked {
        open = !open;
    }
    if open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }

    if open {
        let frame_margin = (ui.spacing().item_spacing.x * 0.5).round() as i8;
        let row_height = ui.spacing().interact_size.y * 0.9;
        let popup_width = (button_rect.width() * 0.24)
            .clamp(ui.spacing().interact_size.x * 5.0, button_rect.width());
        let popup_height = row_height * 2.0
            + editor_theme::font::caption().size
            + editor_theme::space::SM
            + f32::from(frame_margin) * 2.0;
        let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
        let popup_x = button_rect.left().clamp(
            screen.left(),
            (screen.right() - popup_width).max(screen.left()),
        );
        let popup_y = if button_rect.bottom() + popup_height <= screen.bottom() {
            button_rect.bottom()
        } else {
            (button_rect.top() - popup_height).max(screen.top())
        };
        let popup = egui::Area::new(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(popup_x, popup_y))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(editor_theme::semantic().surface)
                    .stroke(egui::Stroke::new(1.0_f32, editor_theme::semantic().grid))
                    .inner_margin(egui::Margin::same(frame_margin))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        let oscillator_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                        });
                        let group_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                        });
                        let oscillator = ui
                            .add_enabled(
                                can_add_oscillator,
                                egui::Button::new("1   OSCILLATOR")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_oscillator && oscillator_key);
                        let group = ui
                            .add_enabled(
                                can_add_group,
                                egui::Button::new("2   GROUP")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_group && group_key);
                        if oscillator {
                            action = Some(GeneratorAddAction::Oscillator);
                        } else if group {
                            action = Some(GeneratorAddAction::Group);
                        }
                        ui.label(
                            egui::RichText::new("KEYS 1 / 2")
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                        );
                    });
            });
        if ui.input(|input| {
            input.pointer.primary_clicked()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    !button_rect.contains(pointer) && !popup.response.rect.contains(pointer)
                })
        }) {
            open = false;
        }
    }
    if action.is_some() {
        open = false;
    }
    ui.data_mut(|data| data.insert_temp(menu_id, open));
    action
}

fn add_oscillator_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: OscillatorSlot,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insert_at = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| insertion.min(group.modules().len()));
        patch
            .insert_oscillator_with_slot(group_id, insert_at, slot)
            .is_ok()
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn add_generator_group(state: &PluginContext<KurvParams>, insertion: usize) {
    state.generator_stack.edit(|patch| {
        if let Ok(id) = patch.insert_group(insertion) {
            let output = GroupOutput {
                pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                ..GroupOutput::default()
            };
            let _ = patch.set_group_output(id, output);
        }
    });
}

fn add_oscillator_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insertion = insertion.min(patch.groups().len());
        let Ok(group_id) = patch.insert_group(insertion) else {
            return false;
        };
        let output = GroupOutput {
            pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
            ..GroupOutput::default()
        };
        let _ = patch.set_group_output(group_id, output);
        patch.insert_oscillator_with_slot(group_id, 0, slot).is_ok()
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn remove_generator_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    modules: &[(ModuleId, OscillatorSlot)],
) {
    if state
        .generator_stack
        .edit(|patch| patch.remove_group(group_id).is_ok())
    {
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor
                .collapsed_group_ids
                .retain(|id| *id != group_id.get());
        }
        clear_group_bindings(state, group_id);
        for (_, slot) in modules {
            let mut config = state.generator_stack.oscillator_config(*slot);
            config.enabled = false;
            state.generator_stack.set_oscillator_config(*slot, config);
        }
        for (module_id, _) in modules {
            clear_module_bindings(state, *module_id);
        }
    }
}

fn clear_module_bindings(state: &PluginContext<KurvParams>, module_id: ModuleId) {
    state
        .params()
        .modulation_route_targets
        .clear_module(module_id.get());
    state
        .params()
        .host_automation_targets
        .clear_module(module_id.get());
}

fn clear_group_bindings(state: &PluginContext<KurvParams>, group_id: GroupId) {
    state
        .params()
        .modulation_route_targets
        .clear_group(group_id.get());
    state
        .params()
        .host_automation_targets
        .clear_group(group_id.get());
}

#[derive(Default)]
struct GroupOutputInteraction {
    remove: bool,
    toggle_collapse: bool,
    dragging: bool,
    hovered: bool,
    pressed: bool,
}

fn paint_generator_drag_ghost(
    ui: &egui::Ui,
    id: impl Hash,
    pointer: egui::Pos2,
    size: egui::Vec2,
    accent: egui::Color32,
    title: &str,
    detail: &str,
    lanes: usize,
    group_preview: bool,
) {
    let palette = editor_theme::semantic();
    let offset = egui::vec2(size.y * 0.22, size.y * 0.18);
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let mut rect = egui::Rect::from_min_size(pointer + offset, size);
    if rect.right() > screen.right() {
        rect = egui::Rect::from_min_size(pointer - egui::vec2(size.x + offset.x, -offset.y), size);
    }
    rect = rect.translate(egui::vec2(
        (screen.left() - rect.left()).max(0.0) - (rect.right() - screen.right()).max(0.0),
        (screen.top() - rect.top()).max(0.0) - (rect.bottom() - screen.bottom()).max(0.0),
    ));
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("generator-drag-ghost", id)),
    ));
    painter.rect_filled(rect, size.y * 0.08, palette.surface);
    painter.rect_stroke(
        rect,
        size.y * 0.08,
        egui::Stroke::new((size.y * 0.035).max(1.0), accent),
        egui::StrokeKind::Inside,
    );
    let marker = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.left() + size.y * 0.16, rect.bottom()),
    );
    painter.rect_filled(marker, size.y * 0.08, accent.gamma_multiply(0.20));
    painter.line_segment(
        [marker.right_top(), marker.right_bottom()],
        egui::Stroke::new((size.y * 0.025).max(1.0), accent),
    );
    let preview = egui::Rect::from_min_max(
        egui::pos2(marker.right() + size.y * 0.10, rect.top() + size.y * 0.32),
        egui::pos2(rect.right() - size.y * 0.10, rect.bottom() - size.y * 0.30),
    );
    let lane_count = if group_preview { lanes.clamp(1, 3) } else { 1 };
    let lane_gap = size.y * 0.06;
    let lane_height = ((preview.height() - lane_gap * (lane_count.saturating_sub(1)) as f32)
        / lane_count as f32)
        .max(editor_theme::shape::FOCUS_STROKE);
    for lane in 0..lane_count {
        let top = preview.top() + lane as f32 * (lane_height + lane_gap);
        let lane_rect = egui::Rect::from_min_size(
            egui::pos2(preview.left(), top),
            egui::vec2(preview.width(), lane_height),
        );
        painter.rect_filled(lane_rect, size.y * 0.025, palette.well);
        let oscillator_right = lane_rect.left() + lane_rect.width() * 0.40;
        let unison_right = lane_rect.left() + lane_rect.width() * 0.80;
        for x in [oscillator_right, unison_right] {
            painter.line_segment(
                [
                    egui::pos2(x, lane_rect.top()),
                    egui::pos2(x, lane_rect.bottom()),
                ],
                egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
            );
        }
        painter.add(egui::Shape::line(
            vec![
                egui::pos2(
                    lane_rect.left() + lane_rect.width() * 0.03,
                    lane_rect.center().y,
                ),
                egui::pos2(
                    oscillator_right - lane_rect.width() * 0.10,
                    lane_rect.top() + lane_rect.height() * 0.24,
                ),
                egui::pos2(
                    oscillator_right - lane_rect.width() * 0.10,
                    lane_rect.bottom() - lane_rect.height() * 0.22,
                ),
                egui::pos2(
                    oscillator_right - lane_rect.width() * 0.03,
                    lane_rect.center().y,
                ),
            ],
            egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.78)),
        ));
        for bar in 0..5 {
            let phase = (bar + 1) as f32 / 6.0;
            let x = egui::lerp(oscillator_right..=unison_right, phase);
            let half = lane_rect.height() * (0.18 + (phase - 0.5).abs() * 0.38);
            painter.line_segment(
                [
                    egui::pos2(x, lane_rect.center().y - half),
                    egui::pos2(x, lane_rect.center().y + half),
                ],
                egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.72)),
            );
        }
        let pan_center = egui::pos2(
            (unison_right + lane_rect.right()) * 0.5,
            lane_rect.center().y,
        );
        painter.line_segment(
            [
                egui::pos2(unison_right, pan_center.y),
                egui::pos2(lane_rect.right(), pan_center.y),
            ],
            egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
        );
        painter.line_segment(
            [
                egui::pos2(pan_center.x, lane_rect.top()),
                egui::pos2(pan_center.x, lane_rect.bottom()),
            ],
            egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
        );
        painter.circle_filled(pan_center, (lane_rect.height() * 0.10).max(1.0), accent);
    }
    let text_left = marker.right() + size.y * 0.18;
    painter.text(
        egui::pos2(text_left, rect.top() + size.y * 0.18),
        egui::Align2::LEFT_CENTER,
        title,
        editor_theme::font::label(),
        palette.text,
    );
    painter.text(
        egui::pos2(text_left, rect.bottom() - size.y * 0.18),
        egui::Align2::LEFT_CENTER,
        detail,
        editor_theme::font::caption(),
        palette.text_muted,
    );
}

fn draw_group_output(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    group_index: usize,
    can_remove_group: bool,
    module_count: usize,
    collapsed: bool,
    mut output: GroupOutput,
    group_accent: egui::Color32,
) -> GroupOutputInteraction {
    let palette = editor_theme::semantic();
    let accent = palette.primary;
    let base_output = output;
    apply_host_automation_to_group(state, group_id, &mut output);
    ui.painter()
        .rect_filled(rect, editor_theme::shape::CONTROL_RADIUS, palette.chrome);
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            group_accent.gamma_multiply(0.34),
        ),
    );
    let before = output;
    let inset = rect.shrink2(egui::vec2(
        editor_theme::space::SM.min(rect.width() * 0.008),
        editor_theme::space::XXS,
    ));
    let identity_width = (inset.height() * 1.32).min(inset.width() * 0.07);
    let identity = egui::Rect::from_min_size(inset.min, egui::vec2(identity_width, inset.height()));
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::XS, inset.top()),
        inset.max,
    );
    let remove_width = if can_remove_group {
        identity.width() * 0.20
    } else {
        0.0
    };
    let collapse_width = identity.width() * 0.22;
    let grip_width = identity.width() * 0.22;
    let collapse_rect =
        egui::Rect::from_min_size(identity.min, egui::vec2(collapse_width, identity.height()));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), identity.top()),
        egui::pos2(collapse_rect.right() + grip_width, identity.bottom()),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(drag_rect.right(), identity.top()),
        egui::pos2(identity.right() - remove_width, identity.bottom()),
    );
    let remove_rect =
        egui::Rect::from_min_max(egui::pos2(label_rect.right(), identity.top()), identity.max);
    let collapse_response = ui
        .interact(
            collapse_rect,
            egui::Id::new(("generator-group-collapse", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand this group"
        } else {
            "Collapse this group"
        });
    let group_drag = ui
        .interact(
            drag_rect,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move this whole group");
    group_drag.dnd_set_drag_payload(group_id);
    let group_pressed = group_drag.is_pointer_button_down_on();
    if group_drag.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.painter().rect_filled(
            identity,
            1.0,
            translucent(
                group_accent,
                (identity.height() * 0.10).clamp(0.0, 255.0) as u8,
            ),
        );
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("group", group_id.get()),
                pointer,
                egui::vec2(
                    rect.width() * 0.42,
                    rect.height() * (1.8 + module_count.min(3) as f32),
                ),
                group_accent,
                &format!("GROUP {}", group_index + 1),
                &format!("{module_count} OSC  ·  {}", output_pair_label(output.pair)),
                module_count,
                true,
            );
        }
    } else if group_drag.hovered() || group_pressed {
        ui.painter().rect_filled(
            drag_rect,
            1.0,
            translucent(group_accent, if group_pressed { 30 } else { 18 }),
        );
    }
    let marker_side = identity.height() * 0.16;
    let marker_center = collapse_rect.center();
    let marker_points = if collapsed {
        vec![
            marker_center + egui::vec2(-marker_side * 0.30, -marker_side * 0.48),
            marker_center + egui::vec2(-marker_side * 0.30, marker_side * 0.48),
            marker_center + egui::vec2(marker_side * 0.48, 0.0),
        ]
    } else {
        vec![
            marker_center + egui::vec2(-marker_side * 0.48, -marker_side * 0.30),
            marker_center + egui::vec2(marker_side * 0.48, -marker_side * 0.30),
            marker_center + egui::vec2(0.0, marker_side * 0.48),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        marker_points,
        if collapse_response.hovered() || collapse_response.is_pointer_button_down_on() {
            palette.text
        } else {
            group_accent
        },
        egui::Stroke::NONE,
    ));
    let grip_dot = editor_theme::shape::STROKE;
    let grip_gap = editor_theme::space::XXS;
    let grip_origin = drag_rect.center() - egui::vec2(grip_gap * 0.5, grip_gap);
    let grip_color = if group_drag.dragged() {
        palette.text
    } else if group_drag.hovered() {
        group_accent
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                grip_origin + egui::vec2(column as f32 * grip_gap, row as f32 * grip_gap),
                grip_dot,
                grip_color,
            );
        }
    }
    let group_label = format!("G{}", group_index + 1);
    ui.painter().text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        &group_label,
        fit_font_to_width(
            ui.painter(),
            &group_label,
            editor_theme::font::label(),
            label_rect.width() * 0.92,
        ),
        palette.text,
    );
    let remove_response = can_remove_group.then(|| {
        ui.interact(
            remove_rect,
            egui::Id::new(("generator-group-remove", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove this group and its modules")
    });
    if let Some(response) = &remove_response {
        let pressed = response.is_pointer_button_down_on();
        if response.hovered() || pressed {
            ui.painter().rect_filled(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(palette.danger, if pressed { 48 } else { 28 }),
            );
        }
        ui.painter().text(
            remove_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            editor_theme::font::label(),
            if pressed || response.hovered() {
                palette.text
            } else {
                palette.text_muted
            },
        );
    }

    ui.painter().line_segment(
        [
            egui::pos2(identity.right(), identity.top() + editor_theme::space::XXS),
            egui::pos2(
                identity.right(),
                identity.bottom() - editor_theme::space::XXS,
            ),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.48),
        ),
    );

    let cells = weighted_cells(controls, [0.92, 1.12, 1.12, 0.82, 1.12, 0.78, 0.76, 1.16]);
    group_dropdown_readout(
        ui,
        cells[0],
        ("group-midi-channel", group_id.get()),
        "MIDI IN",
        if output.receive_midi_channel == 0 {
            "OMNI".to_owned()
        } else {
            format!("CH {}", output.receive_midi_channel)
        },
        group_accent,
        |ui| {
            ui.selectable_value(&mut output.receive_midi_channel, 0, "OMNI");
            for channel in 1..=16 {
                ui.selectable_value(
                    &mut output.receive_midi_channel,
                    channel,
                    format!("CH {channel}"),
                );
            }
        },
    );
    let (attack_response, attack_curve_response) = group_envelope_control(
        ui,
        cells[1],
        (group_id.get(), "attack"),
        &mut output.attack,
        &mut output.attack_curve,
        "ATTACK",
        GroupEnvelopeCurveDirection::Rise,
        GroupOutput::default().attack,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Attack,
        &attack_response,
        output,
        output.attack.to_bits() != before.attack.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::AttackCurve,
        &attack_curve_response,
        output,
        output.attack_curve.to_bits() != before.attack_curve.to_bits(),
    );
    let (decay_response, decay_curve_response) = group_envelope_control(
        ui,
        cells[2],
        (group_id.get(), "decay"),
        &mut output.decay,
        &mut output.decay_curve,
        "DECAY",
        GroupEnvelopeCurveDirection::Fall,
        GroupOutput::default().decay,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Decay,
        &decay_response,
        output,
        output.decay.to_bits() != before.decay.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::DecayCurve,
        &decay_curve_response,
        output,
        output.decay_curve.to_bits() != before.decay_curve.to_bits(),
    );
    with_child(
        ui,
        cells[3],
        ("group-output-sustain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (_, response) = group_scalar_readout(
                ui,
                &mut output.sustain,
                "SUSTAIN",
                0.0..=1.0,
                0.01,
                GroupOutput::default().sustain,
                cells[3].size(),
                format_percent,
                accent,
            );
            host_group_control(
                ui,
                state,
                group_id,
                GroupControl::Sustain,
                &response,
                output,
                output.sustain.to_bits() != before.sustain.to_bits(),
            );
        },
    );
    let (release_response, release_curve_response) = group_envelope_control(
        ui,
        cells[4],
        (group_id.get(), "release"),
        &mut output.release,
        &mut output.release_curve,
        "RELEASE",
        GroupEnvelopeCurveDirection::Fall,
        GroupOutput::default().release,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Release,
        &release_response,
        output,
        output.release.to_bits() != before.release.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::ReleaseCurve,
        &release_curve_response,
        output,
        output.release_curve.to_bits() != before.release_curve.to_bits(),
    );
    with_child(
        ui,
        cells[5],
        ("group-output-gain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (track, response) = group_scalar_readout(
                ui,
                &mut output.gain,
                "GAIN",
                0.0..=2.0,
                0.01,
                GroupOutput::default().gain,
                cells[5].size(),
                format_gain,
                accent,
            );
            let target = ModulationRouteTarget::group(group_id, GroupControl::Gain);
            let host_binding = crate::editor_modulation::host_automation_binding(state, target);
            if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
                output.gain = before.gain;
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                output.gain * 0.5,
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                1.0,
            );
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &response,
                    output.gain * 0.5,
                    output.gain.to_bits() != before.gain.to_bits(),
                );
            }
        },
    );
    with_child(
        ui,
        cells[6],
        ("group-output-pan", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (track, response) = group_scalar_readout(
                ui,
                &mut output.pan,
                "PAN",
                -1.0..=1.0,
                0.01,
                GroupOutput::default().pan,
                cells[6].size(),
                format_pan_value,
                accent,
            );
            let target = ModulationRouteTarget::group(group_id, GroupControl::Pan);
            let host_binding = crate::editor_modulation::host_automation_binding(state, target);
            if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
                output.pan = before.pan;
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                output.pan.mul_add(0.5, 0.5),
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                0.5,
            );
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &response,
                    output.pan.mul_add(0.5, 0.5),
                    output.pan.to_bits() != before.pan.to_bits(),
                );
            }
        },
    );
    let send_response = group_dropdown_readout(
        ui,
        cells[7],
        ("group-output-pair", group_id.get()),
        "SEND TO",
        output_pair_label(output.pair),
        accent,
        |ui| {
            for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                ui.selectable_value(&mut output.pair, pair, output_pair_label(pair));
            }
        },
    );
    if send_response.double_clicked() {
        output.pair = GroupOutput::default().pair;
    }
    restore_host_automated_group_controls(state, group_id, base_output, &mut output);
    if output != base_output {
        state.generator_stack.set_group_output(group_id, output);
    }
    GroupOutputInteraction {
        remove: remove_response.is_some_and(|response| response.clicked()),
        toggle_collapse: collapse_response.clicked(),
        dragging: group_drag.dragged(),
        hovered: group_drag.hovered(),
        pressed: group_pressed,
    }
}

const GROUP_HOST_CONTROLS: [GroupControl; 9] = [
    GroupControl::Gain,
    GroupControl::Pan,
    GroupControl::Attack,
    GroupControl::AttackCurve,
    GroupControl::Decay,
    GroupControl::DecayCurve,
    GroupControl::Sustain,
    GroupControl::Release,
    GroupControl::ReleaseCurve,
];

fn apply_host_automation_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    output: &mut GroupOutput,
) {
    for control in GROUP_HOST_CONTROLS {
        let target = ModulationRouteTarget::group(group_id, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(state, target)
        {
            control.apply_normalized(output, normalized);
        }
    }
}

fn restore_host_automated_group_controls(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    base: GroupOutput,
    output: &mut GroupOutput,
) {
    for control in GROUP_HOST_CONTROLS {
        let target = ModulationRouteTarget::group(group_id, control);
        if crate::editor_modulation::host_automation_binding(state, target).is_some() {
            control.apply_normalized(output, control.normalized_value(base));
        }
    }
}

fn host_group_control(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    control: GroupControl,
    response: &egui::Response,
    output: GroupOutput,
    changed: bool,
) {
    let target = ModulationRouteTarget::group(group_id, control);
    let normalized = control.normalized_value(output);
    let host_binding = crate::editor_modulation::host_automation_binding(state, target);
    crate::editor_modulation::host_automation_destination(ui, state, target, response, normalized);
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, response, normalized, changed,
        );
    }
}

fn group_dropdown_readout(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash + Copy,
    label: &str,
    selected: String,
    accent: egui::Color32,
    add_options: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let palette = editor_theme::semantic();
    let gap = editor_theme::space::XXS;
    let label_width = (rect.width() * 0.39 - gap * 0.5).max(0.0);
    let label_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.left() + label_width, rect.bottom()),
    );
    let field_rect = egui::Rect::from_min_max(
        egui::pos2(
            (label_rect.right() + gap).min(rect.right()),
            rect.top() + editor_theme::space::XXS,
        ),
        egui::pos2(rect.right(), rect.bottom() - editor_theme::space::XXS),
    );
    ui.painter().text(
        label_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        fit_font_to_width(
            ui.painter(),
            label,
            editor_theme::font::caption(),
            label_rect.width() * 0.92,
        ),
        palette.text_muted,
    );
    ui.painter().rect_filled(field_rect, 1.0, palette.control);
    ui.painter().rect_stroke(
        field_rect,
        1.0,
        egui::Stroke::new(1.0_f32, palette.grid.gamma_multiply(0.72)),
        egui::StrokeKind::Inside,
    );
    let mut response = None;
    with_child(
        ui,
        field_rect,
        ("group-dropdown", id_salt),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().button_padding = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size.y = field_rect.height();
            ui.visuals_mut().override_text_color = Some(accent);
            ui.visuals_mut().widgets.inactive.bg_fill = palette.control;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = palette.control;
            ui.visuals_mut().widgets.hovered.bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.active.bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.hovered.fg_stroke.color = accent;
            ui.visuals_mut().widgets.active.fg_stroke.color = accent;
            response = Some(
                egui::ComboBox::from_id_salt(("group-dropdown-combo", id_salt))
                    .selected_text(selected)
                    .width(field_rect.width())
                    .show_ui(ui, add_options)
                    .response,
            );
        },
    );
    response
        .unwrap_or_else(|| ui.interact(field_rect, ui.id().with(id_salt), egui::Sense::hover()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn weighted_cells<const N: usize>(rect: egui::Rect, weights: [f32; N]) -> [egui::Rect; N] {
    let total: f32 = weights.iter().sum();
    let gap = editor_theme::space::XS.min(rect.height() * 0.08);
    let usable_width = (rect.width() - gap * N.saturating_sub(1) as f32).max(0.0);
    let mut cursor = rect.left();
    std::array::from_fn(|index| {
        let right = if index + 1 == N {
            rect.right()
        } else {
            cursor + usable_width * weights[index] / total
        };
        let cell = egui::Rect::from_min_max(
            egui::pos2(cursor, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        cursor = right + gap;
        cell
    })
}

#[derive(Clone, Copy)]
enum GroupEnvelopeCurveDirection {
    Rise,
    Fall,
}

#[allow(clippy::too_many_arguments)]
fn group_envelope_control(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash + Copy,
    value: &mut f32,
    curve: &mut f32,
    label: &str,
    direction: GroupEnvelopeCurveDirection,
    default: f32,
    format_value: fn(f32) -> String,
    accent: egui::Color32,
) -> (egui::Response, egui::Response) {
    let gap = editor_theme::space::XXS;
    let curve_width = (rect.height() * 0.58).min(rect.width() * 0.27);
    let readout_width = (rect.height() * 1.08).min(rect.width() - curve_width - gap);
    let cluster_width = (readout_width + gap + curve_width).min(rect.width());
    let cluster =
        egui::Rect::from_center_size(rect.center(), egui::vec2(cluster_width, rect.height()));
    let readout = egui::Rect::from_min_max(
        cluster.min,
        egui::pos2(
            (cluster.right() - curve_width - gap).max(cluster.left()),
            cluster.bottom(),
        ),
    );
    let curve_rect = egui::Rect::from_min_max(
        egui::pos2(readout.right() + gap, cluster.top()),
        cluster.max,
    );
    let mut value_response = None;
    with_child(
        ui,
        readout,
        ("group-envelope-value", id_salt),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (_, response) = group_scalar_readout(
                ui,
                value,
                label,
                0.0..=20.0,
                0.01,
                default,
                readout.size(),
                format_value,
                accent,
            );
            value_response = Some(response);
        },
    );
    let curve_response = group_envelope_curve(ui, curve_rect, id_salt, curve, direction, accent);
    (
        value_response
            .unwrap_or_else(|| ui.interact(readout, ui.id().with(id_salt), egui::Sense::hover())),
        curve_response,
    )
}

fn group_envelope_curve(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    curve: &mut f32,
    direction: GroupEnvelopeCurveDirection,
    accent: egui::Color32,
) -> egui::Response {
    let interaction = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(rect.width(), rect.height() * 0.88),
    );
    let response = ui
        .interact(
            interaction,
            egui::Id::new(("group-envelope-curve", id_salt)),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Drag to bend the envelope stage; double-click to reset.");
    if response.dragged() {
        let delta = ui.input(|input| input.pointer.delta());
        let precision = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        *curve = (*curve + (delta.x - delta.y) * precision / interaction.height().max(1.0))
            .clamp(-1.0, 1.0);
    } else if response.double_clicked() {
        *curve = 0.0;
    }

    let show_value = response.hovered() || response.dragged();
    let glyph_side = rect.width().min(rect.height() * 0.58);
    let glyph_center = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.43);
    let glyph = egui::Rect::from_center_size(glyph_center, egui::vec2(glyph_side, glyph_side));
    let plot = egui::Rect::from_min_max(
        glyph.left_top() + egui::vec2(glyph.width() * 0.08, glyph.height() * 0.08),
        glyph.right_bottom() - egui::vec2(glyph.width() * 0.08, glyph.height() * 0.08),
    );
    let points = (0..=12)
        .map(|index| {
            let progress = index as f32 / 12.0;
            let shaped = progress + curve.clamp(-1.0, 1.0) * progress * (1.0 - progress);
            let y = match direction {
                GroupEnvelopeCurveDirection::Rise => 1.0 - shaped,
                GroupEnvelopeCurveDirection::Fall => shaped,
            };
            egui::pos2(
                egui::lerp(plot.left()..=plot.right(), progress),
                egui::lerp(plot.top()..=plot.bottom(), y),
            )
        })
        .collect();
    let color = if response.is_pointer_button_down_on() {
        ui.visuals().text_color()
    } else {
        accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.78 })
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new((rect.height() * 0.034).max(1.0), color),
    ));
    if show_value {
        let text = format!("{:+.0}%", *curve * 100.0);
        ui.painter().text(
            egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.04),
            egui::Align2::CENTER_BOTTOM,
            &text,
            fit_font_to_width(
                ui.painter(),
                &text,
                editor_theme::font::caption(),
                rect.width() * 0.95,
            ),
            color,
        );
    }
    response
}

#[allow(clippy::too_many_arguments)]
fn group_scalar_readout(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
    format_value: fn(f32) -> String,
    accent: egui::Color32,
) -> (egui::Rect, egui::Response) {
    let (rect, response, _) = config_scalar_drag(ui, value, range, speed, default, size);
    let value_text = format_value(*value);
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_metric_readout_response(ui, rect, label, &value_text, accent, &response);
    let track = egui::Rect::from_min_max(
        egui::pos2(
            rect.left(),
            rect.bottom() - editor_theme::shape::FOCUS_STROKE,
        ),
        rect.right_bottom(),
    );
    if response.hovered() || active {
        ui.painter().rect_filled(
            track,
            0.0,
            accent.gamma_multiply(if active { 0.92 } else { 0.48 }),
        );
    }
    (track, response)
}

fn format_gain(value: f32) -> String {
    format!("{value:.2}")
}

fn format_pan_value(value: f32) -> String {
    format_pan(value)
}

fn format_seconds(value: f32) -> String {
    format!("{:.0} ms", value * 1_000.0)
}

fn format_percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn output_pair_label(pair: u8) -> String {
    let left = usize::from(pair) * 2 + 1;
    format!("OUT {left}/{}", left + 1)
}

fn phaseplant_readout_rects(rect: egui::Rect) -> [egui::Rect; 5] {
    let groups = weighted_cells(rect, [1.0; 4]);
    [
        groups[0],
        left_half(groups[1]),
        right_half(groups[1]),
        groups[2],
        groups[3],
    ]
}

fn left_half(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(rect.min, egui::pos2(rect.center().x, rect.bottom()))
}

fn right_half(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(rect.center().x, rect.top()), rect.max)
}

fn paint_phaseplant_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    active: bool,
) {
    paint_metric_readout(
        ui,
        rect,
        label,
        value,
        editor_theme::semantic().primary,
        active,
    );
}

fn paint_phaseplant_phase_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    position: f32,
    random: f32,
    position_active: bool,
    random_active: bool,
) {
    let painter = ui.painter_at(rect);
    let accent = editor_theme::semantic().primary;
    let label_y = rect.top() + rect.height() * 0.14;
    let text_y = rect.top() + rect.height() * 0.56;
    let position_text = format!("{position:.0}°");
    let random_text = format!("±{random:.0}°");
    let value_text = format!("{position_text} {random_text}");
    let value_font = fit_font_to_width(
        &painter,
        &value_text,
        editor_theme::font::value(),
        rect.width() * 0.88,
    );
    let position_width = painter
        .layout_no_wrap(
            position_text.clone(),
            value_font.clone(),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    let random_width = painter
        .layout_no_wrap(
            random_text.clone(),
            value_font.clone(),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    let separator = rect.width() * 0.02;
    let value_left = rect.center().x - (position_width + separator + random_width) * 0.5;
    painter.text(
        egui::pos2(rect.center().x, label_y),
        egui::Align2::CENTER_TOP,
        "PHASE",
        editor_theme::font::caption(),
        accent.gamma_multiply(0.64),
    );
    painter.text(
        egui::pos2(value_left, text_y),
        egui::Align2::LEFT_TOP,
        position_text,
        value_font.clone(),
        if position_active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
    painter.text(
        egui::pos2(value_left + position_width + separator, text_y),
        egui::Align2::LEFT_TOP,
        random_text,
        value_font,
        if random_active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
}

fn draw_compact_oscillator(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    slot: OscillatorSlot,
    module_id: ModuleId,
    gap: f32,
) {
    let index = slot.index();
    let base_config = state.generator_stack.oscillator_config(slot);
    let mut config = base_config;
    apply_host_automation_to_oscillator(state, module_id, slot, &mut config);
    let enabled = config.enabled;
    let mut config_changed = false;
    let mut reset_requested = false;
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let panel_gap = (gap * 0.18).max(rect.height() * 0.006);
    let inner = rect.shrink(panel_gap * 0.45);
    let identity_width = inner.width() * 0.055;
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), identity.top() + identity.height() * 0.55),
        identity.max,
    );
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - identity.width() * 0.20,
            identity.top() + identity.height() * 0.10,
        ),
        egui::vec2(identity.width() * 0.36, identity.height() * 0.16),
    );
    let drag_handle = ui
        .interact(
            drag_rect,
            egui::Id::new(("oscillator-group-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(
            "Drag to reorder. Hold Alt to drop inside another group; use the left lane to create a group.",
        );
    drag_handle.dnd_set_drag_payload(module_id);
    drag_handle.context_menu(|ui| {
        if ui.button("RESET OSCILLATOR").clicked() {
            reset_requested = true;
            ui.close();
        }
    });
    if drag_handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("oscillator", module_id.get()),
                pointer,
                egui::vec2(rect.width() * 0.46, rect.height() * 0.68),
                editor_theme::semantic().primary,
                &format!("OSC {}", index + 1),
                "MOVE TO GROUP",
                1,
                false,
            );
        }
    }
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + panel_gap, inner.top()),
        inner.right_bottom(),
    );
    let panels_width = (body.width() - panel_gap * 2.0).max(1.0);
    let oscillator_width = panels_width * 0.40;
    let oscillator_panel =
        egui::Rect::from_min_size(body.min, egui::vec2(oscillator_width, body.height()));
    let unison_width = panels_width * 0.40;
    let unison_panel = egui::Rect::from_min_size(
        egui::pos2(oscillator_panel.right() + panel_gap, body.top()),
        egui::vec2(unison_width, body.height()),
    );
    let pan_panel = egui::Rect::from_min_max(
        egui::pos2(unison_panel.right() + panel_gap, body.top()),
        body.right_bottom(),
    );
    let readout_height = body.height() * 0.22;
    let wave_label_width = ui
        .painter()
        .layout_no_wrap(
            "WAVE".to_owned(),
            editor_theme::font::caption(),
            egui::Color32::WHITE,
        )
        .size()
        .x
        + editor_theme::space::XS * 2.0;
    let waveform_rail_width = wave_label_width
        .min(oscillator_panel.width() * 0.10)
        .max(oscillator_panel.width() * 0.055);
    let waveform_rail = egui::Rect::from_min_size(
        oscillator_panel.min,
        egui::vec2(waveform_rail_width, oscillator_panel.height()),
    );
    let oscillator_content = egui::Rect::from_min_max(
        egui::pos2(waveform_rail.right(), oscillator_panel.top()),
        oscillator_panel.max,
    );
    let oscillator_plot = egui::Rect::from_min_max(
        oscillator_content.min,
        egui::pos2(
            oscillator_content.right(),
            oscillator_content.bottom() - readout_height,
        ),
    );
    let oscillator_readouts = egui::Rect::from_min_max(
        egui::pos2(oscillator_content.left(), oscillator_plot.bottom()),
        oscillator_panel.right_bottom(),
    );
    let unison_plot = egui::Rect::from_min_max(
        unison_panel.min,
        egui::pos2(unison_panel.right(), unison_panel.bottom() - readout_height),
    );
    let unison_readouts = egui::Rect::from_min_max(
        egui::pos2(unison_panel.left(), unison_plot.bottom()),
        unison_panel.right_bottom(),
    );
    ui.painter()
        .rect_filled(body, 1.0, editor_theme::semantic().well);

    ui.painter().rect_filled(
        identity,
        0.0,
        if drag_handle.dragged() || drag_handle.is_pointer_button_down_on() {
            editor_theme::semantic().control_hover
        } else if drag_handle.hovered() {
            editor_theme::semantic().control
        } else {
            editor_theme::semantic().chrome
        },
    );
    let identity_content = identity.shrink2(egui::vec2(
        identity.width() * 0.08,
        identity.height() * 0.04,
    ));
    with_child(
        ui,
        identity_content,
        ("oscillator-identity", index),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            config_changed |= compact_toggle(ui, &mut config.enabled);
            ui.label(
                egui::RichText::new("OSC")
                    .font(editor_theme::font::caption())
                    .color(editor_theme::semantic().text_muted),
            );
            ui.label(
                egui::RichText::new((index + 1).to_string())
                    .font(editor_theme::font::title())
                    .color(editor_theme::semantic().text),
            );
        },
    );
    let remove_response = ui
        .interact(
            remove_rect,
            egui::Id::new(("oscillator-remove", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Remove Oscillator {} from this group", index + 1));
    let remove_requested = remove_response.clicked();
    let remove_pressed = remove_response.is_pointer_button_down_on();
    if remove_response.hovered() || remove_pressed {
        ui.painter().rect_filled(
            remove_rect,
            editor_theme::shape::CONTROL_RADIUS,
            translucent(
                editor_theme::semantic().danger,
                if remove_pressed { 48 } else { 28 },
            ),
        );
    }
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::label(),
        if remove_pressed || remove_response.hovered() {
            editor_theme::semantic().text
        } else {
            editor_theme::semantic().text_muted
        },
    );
    with_child(
        ui,
        oscillator_readouts,
        ("oscillator-controls", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            let readouts = phaseplant_readout_rects(oscillator_readouts);
            let mut readout_active = [false; 6];
            let hits = [
                (readouts[0], ConfigField::Level, 0),
                (readouts[1], ConfigField::Semi, 1),
                (readouts[2], ConfigField::Fine, 2),
                (readouts[3], ConfigField::Pan, 3),
                (left_half(readouts[4]), ConfigField::PhasePosition, 4),
                (right_half(readouts[4]), ConfigField::PhaseRandom, 5),
            ];
            for (cell_index, (cell, field, readout_index)) in hits.into_iter().enumerate() {
                with_child(
                    ui,
                    cell,
                    ("compact-config", index, cell_index),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let (changed, active) = config_field_drag(
                            ui,
                            state,
                            module_id,
                            slot,
                            &mut config,
                            field,
                            cell.size(),
                        );
                        config_changed |= changed;
                        readout_active[readout_index] |= active;
                    },
                );
            }
            let (level, semi, cents, pan, phase_position, phase_random) = (
                format!("{:.0} %", config.level * 100.0),
                format!("{:+.0}", config.transpose),
                format!("{:+.1}", config.cents),
                format_pan(config.pan),
                config.phase_position * 360.0,
                config.phase_random * 360.0,
            );
            for (rect, label, value, active) in [
                (readouts[0], "LEVEL", level, readout_active[0]),
                (readouts[1], "SEMI", semi, readout_active[1]),
                (readouts[2], "CENT", cents, readout_active[2]),
                (readouts[3], "PAN", pan, readout_active[3]),
            ] {
                paint_phaseplant_readout(ui, rect, label, &value, active);
            }
            ui.painter().circle_filled(
                egui::pos2(
                    (readouts[1].right() + readouts[2].left()) * 0.5,
                    readouts[1].top() + readouts[1].height() * 0.66,
                ),
                editor_theme::shape::FOCUS_STROKE,
                editor_theme::semantic().primary.gamma_multiply(0.64),
            );
            paint_phaseplant_phase_readout(
                ui,
                readouts[4],
                phase_position,
                phase_random,
                readout_active[4],
                readout_active[5],
            );
        },
    );

    with_child(
        ui,
        waveform_rail,
        ("compact-wave-shape", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.35 });
            config_changed |= config_wave_field(
                ui,
                state,
                module_id,
                slot,
                &mut config.shape,
                waveform_rail.size(),
            );
        },
    );
    with_child(
        ui,
        oscillator_plot,
        ("compact-wave-cycle", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= oscillator_waveform_view(
                ui,
                state,
                oscillator_plot.width(),
                oscillator_plot.height(),
                module_id,
                slot,
                &mut config,
            );
        },
    );
    with_child(
        ui,
        unison_plot,
        ("compact-unison-distribution", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= custom_unison_distribution_view(
                ui,
                state,
                module_id,
                slot,
                unison_plot.width(),
                unison_plot.height(),
                &mut config,
                state.generator_stack.pan_shape_curve(slot),
            );
        },
    );
    with_child(
        ui,
        pan_panel,
        ("compact-pan-panel", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= custom_pan_panel_view(
                ui,
                state,
                module_id,
                slot,
                pan_panel.width(),
                pan_panel.height(),
                &mut config,
                state.generator_stack.pan_shape_curve(slot),
            );
        },
    );
    with_child(
        ui,
        unison_readouts,
        ("compact-unison-controls", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            let cell_width = unison_readouts.width() * 0.2;
            for (cell_index, field) in [
                UnisonConfigField::Voices,
                UnisonConfigField::Range,
                UnisonConfigField::Jitter,
                UnisonConfigField::Rate,
                UnisonConfigField::Width,
            ]
            .into_iter()
            .enumerate()
            {
                let cell = egui::Rect::from_min_size(
                    egui::pos2(
                        unison_readouts.left() + cell_index as f32 * cell_width,
                        unison_readouts.top(),
                    ),
                    egui::vec2(cell_width, unison_readouts.height()),
                );
                with_child(
                    ui,
                    cell,
                    ("compact-unison-config", index, cell_index),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        config_changed |= unison_config_readout(
                            ui,
                            state,
                            module_id,
                            slot,
                            &mut config,
                            field,
                            cell.size(),
                        );
                    },
                );
            }
        },
    );
    let divider = egui::Stroke::new(1.0_f32, editor_theme::semantic().grid.gamma_multiply(0.52));
    for x in [oscillator_panel.right(), unison_panel.right()] {
        ui.painter().line_segment(
            [
                egui::pos2(x + panel_gap * 0.5, body.top()),
                egui::pos2(x + panel_gap * 0.5, body.bottom()),
            ],
            divider,
        );
    }
    if drag_handle.dragged() {
        ui.painter().rect_filled(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            translucent(editor_theme::semantic().chrome, 148),
        );
        ui.painter().rect_stroke(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE,
                editor_theme::semantic().primary,
            ),
            egui::StrokeKind::Inside,
        );
    }
    if reset_requested {
        state.generator_stack.reset_oscillator(slot);
        return;
    }
    if config_changed {
        restore_host_automated_oscillator_controls(
            state,
            module_id,
            slot,
            base_config,
            &mut config,
        );
        state.generator_stack.set_oscillator_config(slot, config);
    }
    if remove_requested
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
    {
        clear_module_bindings(state, module_id);
        let mut removed = base_config;
        removed.enabled = false;
        state.generator_stack.set_oscillator_config(slot, removed);
    }
}

#[derive(Clone, Copy)]
enum ConfigField {
    Level,
    Pan,
    Fine,
    Semi,
    PhasePosition,
    PhaseRandom,
}

#[derive(Clone, Copy)]
enum UnisonConfigField {
    Voices,
    Range,
    Jitter,
    Rate,
    Width,
}

const OSCILLATOR_HOST_CONTROLS: [OscillatorControl; 23] = [
    OscillatorControl::Shape,
    OscillatorControl::TablePosition,
    OscillatorControl::PulseWidth,
    OscillatorControl::Transpose,
    OscillatorControl::Cents,
    OscillatorControl::Level,
    OscillatorControl::Pan,
    OscillatorControl::PhasePosition,
    OscillatorControl::PhaseRandom,
    OscillatorControl::PhaseWarpAmount,
    OscillatorControl::UnisonVoices,
    OscillatorControl::UnisonRange,
    OscillatorControl::UnisonAmount,
    OscillatorControl::UnisonCurve,
    OscillatorControl::UnisonJitter,
    OscillatorControl::UnisonRate,
    OscillatorControl::UnisonWidth,
    OscillatorControl::UnisonWeight,
    OscillatorControl::UnisonAlignment,
    OscillatorControl::UnisonPanCurve,
    OscillatorControl::UnisonPanCenter,
    OscillatorControl::UnisonStereoPosition,
    OscillatorControl::UnisonStereoAlternate,
];

fn apply_host_automation_to_oscillator(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(state, target)
        {
            control.apply_normalized(config, normalized);
        }
    }
}

fn restore_host_automated_oscillator_controls(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    base: crate::generators::OscillatorConfig,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if crate::editor_modulation::host_automation_binding(state, target).is_some() {
            control.apply_normalized(config, control.normalized_value(base));
        }
    }
}

fn compact_toggle(ui: &mut egui::Ui, enabled: &mut bool) -> bool {
    let extent = ui
        .available_width()
        .min(ui.available_height())
        .min(ui.spacing().interact_size.y)
        .max(ui.spacing().interact_size.y * 0.42)
        * 0.72;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(extent, extent), egui::Sense::click());
    let clicked = response.clicked();
    if clicked {
        *enabled = !*enabled;
    }
    let color = if *enabled {
        editor_theme::palette().accent
    } else {
        editor_theme::semantic().grid
    };
    ui.painter()
        .circle_filled(rect.center(), extent * 0.28, color);
    response.on_hover_text(if *enabled {
        "Disable oscillator"
    } else {
        "Enable oscillator"
    });
    clicked
}

fn config_field_drag(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    field: ConfigField,
    size: egui::Vec2,
) -> (bool, bool) {
    let before = *config;
    let defaults = crate::generators::OscillatorConfig::default();
    let (changed, response) = match field {
        ConfigField::Level => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.level,
                0.0..=1.0,
                0.005,
                defaults.level,
                size,
            );
            (changed, response)
        }
        ConfigField::Pan => {
            let (_, response, changed) =
                config_scalar_drag(ui, &mut config.pan, -1.0..=1.0, 0.01, defaults.pan, size);
            (changed, response)
        }
        ConfigField::Fine => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.cents,
                -100.0..=100.0,
                0.25,
                defaults.cents,
                size,
            );
            (changed, response)
        }
        ConfigField::Semi => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.transpose,
                -48.0..=48.0,
                0.01,
                defaults.transpose,
                size,
            );
            (changed, response)
        }
        ConfigField::PhasePosition => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.phase_position,
                0.0..=1.0,
                0.005,
                defaults.phase_position,
                size,
            );
            (changed, response)
        }
        ConfigField::PhaseRandom => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.phase_random,
                0.0..=1.0,
                0.005,
                defaults.phase_random,
                size,
            );
            (changed, response)
        }
    };
    let (control, span, internal_modulation) = match field {
        ConfigField::Level => (OscillatorControl::Level, 1.0, true),
        ConfigField::Pan => (OscillatorControl::Pan, 0.5, true),
        ConfigField::Fine => (OscillatorControl::Cents, 0.5, true),
        ConfigField::Semi => (OscillatorControl::Transpose, 0.5, true),
        ConfigField::PhasePosition => (OscillatorControl::PhasePosition, 1.0, true),
        ConfigField::PhaseRandom => (OscillatorControl::PhaseRandom, 1.0, false),
    };
    let mut changed = changed;
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    if internal_modulation
        && crate::editor_modulation::modular_owns_gesture(ui, state, target, &response)
    {
        *config = before;
        changed = false;
    }
    let base = control.normalized_value(*config);
    if let Some((_, param, _)) = crate::editor_modulation::host_automation_binding(state, target) {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, &response, base, changed,
        );
        changed = false;
    }
    if internal_modulation {
        let track = egui::Rect::from_min_max(
            egui::pos2(
                response.rect.left(),
                response.rect.bottom() - response.rect.height() * 0.08,
            ),
            response.rect.right_bottom(),
        );
        crate::editor_modulation::modular_destination(
            ui,
            state,
            target,
            &response,
            base,
            track,
            crate::editor_modulation::TrackAxis::Horizontal,
            span,
        );
    } else {
        crate::editor_modulation::host_automation_destination(ui, state, target, &response, base);
    }
    (
        changed,
        response.is_pointer_button_down_on() || response.dragged(),
    )
}

fn unison_config_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    field: UnisonConfigField,
    size: egui::Vec2,
) -> bool {
    let defaults = crate::generators::OscillatorConfig::default();
    let (control, changed, host_value_changed, base_only_changed, response) = match field {
        UnisonConfigField::Voices => {
            let before = config.unison_voices;
            let mut voices = f32::from(before);
            let (changed, response) = config_scalar_readout(
                ui,
                &mut voices,
                "VOICES",
                1.0..=64.0,
                f32::from(defaults.unison_voices),
                1.0,
                size,
                |value| format!("{value:.0}"),
            );
            config.unison_voices = voices.round().clamp(1.0, 64.0) as u8;
            let changed = changed || config.unison_voices != before;
            (
                OscillatorControl::UnisonVoices,
                changed,
                changed,
                false,
                response,
            )
        }
        UnisonConfigField::Range => {
            let (changed, response) = config_scalar_readout(
                ui,
                &mut config.unison_range,
                "RANGE",
                0.0..=48.0,
                0.1,
                defaults.unison_range,
                size,
                |value| format!("{value:.2} st"),
            );
            (
                OscillatorControl::UnisonRange,
                changed,
                changed,
                false,
                response,
            )
        }
        UnisonConfigField::Jitter => {
            let (amount_changed, toggled, response) = jitter_config_readout(ui, config, size);
            (
                OscillatorControl::UnisonJitter,
                amount_changed || toggled,
                amount_changed,
                toggled,
                response,
            )
        }
        UnisonConfigField::Rate => {
            let (changed, response) = config_scalar_readout(
                ui,
                &mut config.unison_rate,
                "RATE",
                0.0..=1.0,
                0.005,
                defaults.unison_rate,
                size,
                |value| format!("{:.2} Hz", normalized_unison_rate(value)),
            );
            (
                OscillatorControl::UnisonRate,
                changed,
                changed,
                false,
                response,
            )
        }
        UnisonConfigField::Width => {
            let (changed, response) = config_scalar_readout(
                ui,
                &mut config.unison_width,
                "WIDTH",
                0.0..=1.0,
                0.005,
                defaults.unison_width,
                size,
                |value| format!("{:.0}%", value * 100.0),
            );
            (
                OscillatorControl::UnisonWidth,
                changed,
                changed,
                false,
                response,
            )
        }
    };
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    let normalized = control.normalized_value(*config);
    let host_binding = crate::editor_modulation::host_automation_binding(state, target);
    if matches!(field, UnisonConfigField::Jitter | UnisonConfigField::Rate) {
        let track = egui::Rect::from_min_max(
            egui::pos2(
                response.rect.left(),
                response.rect.bottom() - response.rect.height() * 0.08,
            ),
            response.rect.right_bottom(),
        );
        crate::editor_modulation::modular_destination(
            ui,
            state,
            target,
            &response,
            normalized,
            track,
            crate::editor_modulation::TrackAxis::Horizontal,
            1.0,
        );
    } else {
        crate::editor_modulation::host_automation_destination(
            ui, state, target, &response, normalized,
        );
    }
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state,
            param,
            &response,
            normalized,
            host_value_changed,
        );
        base_only_changed
    } else {
        changed
    }
}

fn config_wave_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    value: &mut f32,
    size: egui::Vec2,
) -> bool {
    let minimum = editor_theme::title_height(ui) * 0.8;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(minimum), size.y.max(minimum)),
        egui::Sense::click_and_drag(),
    );
    let label_inset =
        (editor_theme::font::caption().size + editor_theme::space::XXS).min(rect.height() * 0.24);
    let selector_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), (rect.top() + label_inset).min(rect.bottom())),
        egui::pos2(rect.right(), (rect.bottom() - label_inset).max(rect.top())),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let before = *value;
    let target = ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::Shape);
    let host_binding = crate::editor_modulation::host_automation_binding(state, target);
    let owns_modulation =
        crate::editor_modulation::modular_owns_gesture(ui, state, target, &response);
    if response.dragged() && !owns_modulation {
        if ui.input(|input| input.modifiers.shift) {
            *value = (*value - response.drag_motion().y * 0.001).clamp(0.0, 3.0);
        } else if let Some(pointer) = response.interact_pointer_pos() {
            *value = vertical_selector_value(selector_rect, pointer) * 3.0;
        }
    } else if !owns_modulation && (response.double_clicked() || response.secondary_clicked()) {
        *value = crate::generators::OscillatorConfig::default().shape;
    }
    paint_vertical_selector(
        &ui.painter_at(rect),
        selector_rect,
        *value / 3.0,
        editor_theme::semantic().primary,
    );
    let painter = ui.painter_at(rect);
    let shape_name = ["SIN", "TRI", "SAW", "PLS"][value.round().clamp(0.0, 3.0) as usize];
    painter.text(
        rect.center_top() + egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_TOP,
        "WAVE",
        fit_font_to_width(
            &painter,
            "WAVE",
            editor_theme::font::caption(),
            rect.width() * 0.88,
        ),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        rect.center_bottom() - egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_BOTTOM,
        shape_name,
        fit_font_to_width(
            &painter,
            shape_name,
            editor_theme::font::caption(),
            rect.width() * 0.88,
        ),
        editor_theme::semantic().primary,
    );
    let track = egui::Rect::from_min_max(
        egui::pos2(
            selector_rect.center().x - selector_rect.width() * 0.06,
            selector_rect.top(),
        ),
        egui::pos2(
            selector_rect.center().x + selector_rect.width() * 0.06,
            selector_rect.bottom(),
        ),
    );
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        *value / 3.0,
        track,
        crate::editor_modulation::TrackAxis::Vertical,
        1.0,
    );
    let changed = value.to_bits() != before.to_bits();
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state,
            param,
            &response,
            *value / 3.0,
            changed,
        );
    }
    response.on_hover_text("Drag vertically to select the VA waveform; double-click for saw.");
    changed && host_binding.is_none()
}

fn jitter_config_readout(
    ui: &mut egui::Ui,
    config: &mut crate::generators::OscillatorConfig,
    size: egui::Vec2,
) -> (bool, bool, egui::Response) {
    let defaults = crate::generators::OscillatorConfig::default();
    let mode = SwarmMode::from_index(config.unison_jitter_mode);
    let (rect, response, amount_changed) = config_scalar_drag(
        ui,
        &mut config.unison_jitter,
        0.0..=1.0,
        0.005,
        defaults.unison_jitter,
        size,
    );
    let toggled = response.clicked() && !response.double_clicked();
    if toggled {
        config.unison_jitter_mode = u8::from(mode == SwarmMode::Noise);
    }
    let mode = SwarmMode::from_index(config.unison_jitter_mode);
    paint_jitter_readout(
        ui,
        rect,
        config.unison_jitter,
        mode,
        response.is_pointer_button_down_on() || response.dragged(),
    );
    let response = response.on_hover_text(format!(
        "Jitter mode: {}. Click to switch; drag to change amount.",
        jitter_mode_name(mode)
    ));
    (amount_changed, toggled, response)
}

fn jitter_mode_name(mode: SwarmMode) -> &'static str {
    if mode == SwarmMode::Sine {
        "Sine"
    } else {
        "Noise"
    }
}

fn paint_jitter_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    amount: f32,
    mode: SwarmMode,
    active: bool,
) {
    let painter = ui.painter_at(rect);
    let accent = editor_theme::semantic().unison;
    let icon_side = rect.height().min(rect.width()) * 0.22;
    let label_font = fit_font_to_width(
        &painter,
        "JITTR",
        editor_theme::font::caption(),
        rect.width() - icon_side * 1.5,
    );
    let label_color = accent.gamma_multiply(if active { 1.0 } else { 0.68 });
    let label = painter.layout_no_wrap("JITTR".into(), label_font, label_color);
    let gap = icon_side * 0.22;
    let group_width = label.size().x + gap + icon_side;
    let label_pos = egui::pos2(
        rect.center().x - group_width * 0.5,
        rect.top() + rect.height() * 0.14,
    );
    painter.galley(label_pos, label, label_color);
    let icon = egui::Rect::from_min_size(
        egui::pos2(label_pos.x + group_width - icon_side, label_pos.y),
        egui::vec2(icon_side, icon_side),
    );
    if icon_font_ready(ui) {
        painter.text(
            icon.center(),
            egui::Align2::CENTER_CENTER,
            if mode == SwarmMode::Sine {
                egui_phosphor::regular::WAVE_SINE
            } else {
                egui_phosphor::regular::WAVEFORM
            },
            egui::FontId::proportional(icon_side),
            label_color,
        );
    } else {
        paint_jitter_icon_fallback(&painter, icon, mode, label_color);
    }

    let value = format!("{:.0}%", amount * 100.0);
    painter.text(
        egui::pos2(rect.center().x, rect.top() + rect.height() * 0.56),
        egui::Align2::CENTER_TOP,
        &value,
        fit_font_to_width(
            &painter,
            &value,
            editor_theme::font::value(),
            rect.width() * 0.88,
        ),
        if active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
}

fn paint_jitter_icon_fallback(
    painter: &egui::Painter,
    rect: egui::Rect,
    mode: SwarmMode,
    color: egui::Color32,
) {
    let points: Vec<_> = (0..=12)
        .map(|index| {
            let phase = index as f32 / 12.0;
            let wave = if mode == SwarmMode::Sine {
                (phase * std::f32::consts::TAU).sin()
            } else {
                ((index * 7 % 11) as f32 / 5.0) - 1.0
            };
            egui::pos2(
                egui::lerp(rect.left()..=rect.right(), phase),
                rect.center().y - wave * rect.height() * 0.30,
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new((rect.height() * 0.08).max(1.0), color),
    ));
}

#[allow(clippy::too_many_arguments)]
fn config_scalar_readout(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
    format_value: fn(f32) -> String,
) -> (bool, egui::Response) {
    let (rect, response, changed) = config_scalar_drag(ui, value, range, speed, default, size);
    let value_text = format_value(*value);
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_metric_readout(
        ui,
        rect,
        label,
        &value_text,
        editor_theme::semantic().unison,
        active,
    );
    (changed, response)
}

fn config_scalar_drag(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
) -> (egui::Rect, egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(12.0), size.y.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let before = *value;
    if response.dragged() {
        let delta = ui.input(|input| input.pointer.delta());
        let precision = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        *value =
            (*value + (delta.x - delta.y) * speed * precision).clamp(*range.start(), *range.end());
    } else if response.double_clicked() {
        *value = default;
    }
    let response = response
        .on_hover_text("Drag to change. Hold Shift for fine control; double-click to reset.");
    (rect, response, value.to_bits() != before.to_bits())
}

fn format_pan(value: f32) -> String {
    if value.abs() < 0.005 {
        "C".to_owned()
    } else if value < 0.0 {
        format!("L {:.0}", value.abs() * 100.0)
    } else {
        format!("R {:.0}", value * 100.0)
    }
}

fn draw_modulation(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().well);
    let inner = rect.shrink(rect.width().min(rect.height()) * 0.012);
    with_child(
        ui,
        inner,
        "modulation",
        egui::Layout::top_down(egui::Align::Min),
        |ui| crate::editor_lfo::modulation_view(ui, state, inner.width(), inner.height()),
    );
}

fn draw_performance(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let inner = rect.shrink(editor_theme::space::SM);
    with_child(
        ui,
        inner,
        "performance",
        egui::Layout::top_down(egui::Align::Min),
        |ui| crate::editor::performance_view(ui, state, inner.width(), inner.height()),
    );
}

fn with_child(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    layout: egui::Layout,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(layout),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add_contents(&mut child);
}
