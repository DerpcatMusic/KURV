use std::hash::Hash;
use std::time::Duration;

use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_envelope::envelope_view;
use crate::editor_history::EditorHistory;
use crate::editor_oscillator::{
    antialiasing_selector_compact, oscillator_waveform_view, quality_selector_compact,
};
use crate::editor_presets::{PresetEntry, PresetStore};
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, normalized_unison_rate,
};
use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, OscillatorSlot, Patch,
};
use crate::{KurvParams, editor, editor_theme, performance};

const UI_BUILD_VERSION: &str = "v0.8.0 | phaseplant-groups";

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
        self.store
            .as_mut()
            .and_then(|store| store.entries().ok())
            .map_or_else(Vec::new, <[PresetEntry]>::to_vec)
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
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
        .data_mut(|data| data.get_temp::<ThemeUi>(theme_id))
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
        .data_mut(|data| data.get_temp::<EditorHistory>(history_id))
        .unwrap_or_default();
    let mut presets = ui
        .data_mut(|data| data.get_temp::<PresetUi>(preset_id))
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

    let inset = (bounds.width().min(bounds.height()) / 90.0).clamp(4.0, 8.0);
    let gap = (bounds.width().min(bounds.height()) / 180.0).clamp(2.0, 4.0);
    let section_gap = (gap * 1.6).clamp(4.0, 7.0);
    let content = bounds.shrink(inset);
    let header_height = (content.height() * 44.0 / 704.0).clamp(32.0, 44.0);
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
        header_rect.shrink2(egui::vec2(8.0, 4.0)),
        "header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| draw_header(ui, state, &mut history, &mut presets, &mut settings_open),
    );

    let left_width = (workspace.width() * 700.0 / 1104.0).clamp(
        workspace.width() * 0.58,
        (workspace.width() - 160.0).max(1.0),
    );
    let left = egui::Rect::from_min_size(workspace.min, egui::vec2(left_width, workspace.height()));
    let right = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, workspace.top()),
        workspace.right_bottom(),
    );
    draw_generator_group(ui, state, left, gap, section_gap);

    let stacked_height = (right.height() - section_gap * 2.0).max(3.0);
    let envelope_height = stacked_height * 0.35;
    let lfo_height = stacked_height * 0.35;
    let envelope_rect =
        egui::Rect::from_min_size(right.min, egui::vec2(right.width(), envelope_height));
    let lfo_rect = egui::Rect::from_min_size(
        egui::pos2(right.left(), envelope_rect.bottom() + section_gap),
        egui::vec2(right.width(), lfo_height),
    );
    let performance_rect = egui::Rect::from_min_max(
        egui::pos2(right.left(), lfo_rect.bottom() + section_gap),
        right.right_bottom(),
    );
    draw_envelope(ui, state, envelope_rect);
    draw_modulation(ui, state, lfo_rect);
    draw_performance(ui, state, performance_rect);
    if !settings_open && !presets.save_open {
        crate::editor_modulation::draw_overlay(ui, state);
    }

    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        if presets.save_open {
            presets.save_open = false;
        } else {
            settings_open = false;
        }
    }
    if presets.save_open {
        settings_open = false;
        draw_overlay_scrim(ui, workspace, "save-preset-scrim");
        draw_save_preset_panel(ui, state, workspace, &mut presets);
    } else if settings_open {
        draw_overlay_scrim(ui, workspace, "settings-scrim");
        draw_settings_panel(ui, state, workspace, &mut settings_open, &mut themes);
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

fn draw_header(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    history: &mut EditorHistory,
    presets: &mut PresetUi,
    settings_open: &mut bool,
) {
    let rect = ui.max_rect();
    let width = rect.width();
    let center_width = (width * 0.46).clamp(300.0, 520.0);
    let center =
        egui::Rect::from_center_size(rect.center(), egui::vec2(center_width, rect.height()));
    let right_width = if width < 760.0 { 132.0 } else { 198.0 };
    let left = egui::Rect::from_min_max(rect.min, egui::pos2(center.left() - 6.0, rect.bottom()));
    let right = egui::Rect::from_min_max(
        egui::pos2(
            (rect.right() - right_width).max(center.right() + 6.0),
            rect.top(),
        ),
        rect.right_bottom(),
    );

    with_child(
        ui,
        left,
        "header-left",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if ui
                .add_sized(
                    [28.0, 28.0],
                    egui::Button::new("⚙").selected(*settings_open),
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
                            .font(egui::FontId::proportional(if width < 700.0 {
                                18.0
                            } else {
                                22.0
                            }))
                            .strong()
                            .color(editor_theme::palette().accent),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text("Double-click for factory default");
            if reset.double_clicked() {
                editor::reset_to_defaults(state);
                history.commit(state);
                presets.selected = "Init".to_owned();
                presets.dirty = false;
                presets.error = None;
            }
            if left.width() >= 190.0 {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("PROCEDURAL VA")
                            .font(egui::FontId::proportional(10.0))
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
            editor::output_meter(ui, state, right.width(), right.height().max(24.0));
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
    let compact = width < 390.0;
    if ui
        .add_enabled(history.can_undo(), egui::Button::new("<").frame(false))
        .on_hover_text("Undo — Ctrl/Cmd+Z")
        .clicked()
    {
        presets.dirty |= history.undo(state);
    }
    if ui
        .add_enabled(history.can_redo(), egui::Button::new(">").frame(false))
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
        .width(if compact { 92.0 } else { 160.0 })
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

    if ui
        .button(if compact { "DEF" } else { "SAVE DEFAULT" })
        .on_hover_text("Save the current complete state as Default.kurv")
        .clicked()
        && let Some(store) = presets.store.as_mut()
    {
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
        .button(if compact { "SAVE" } else { "SAVE AS" })
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
) {
    let available = workspace.shrink(8.0);
    let size = egui::vec2(
        360.0_f32.min(available.width()),
        332.0_f32.min(available.height()),
    );
    let rect = egui::Rect::from_min_size(available.min, size);
    ui.painter()
        .rect_filled(rect, 3.0, editor_theme::semantic().surface);
    let panel = rect.shrink(10.0);
    let header_height = 32.0_f32.min(panel.height());
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
                if ui.button("CLOSE").clicked() {
                    *open = false;
                }
            });
        },
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(panel.left(), header.bottom() + 4.0),
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
                    ui.set_width(body.width() - 8.0);
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
                                    .desired_width(150.0),
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
                        .spacing(egui::vec2(12.0, 6.0))
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

                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("GENERATOR QUALITY")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    let selector_width = ((body.width() - 28.0) * 0.5).max(92.0);
                    ui.horizontal(|ui| {
                        antialiasing_selector_compact(ui, state, selector_width);
                        quality_selector_compact(ui, state, selector_width);
                    });
                    ui.add_space(8.0);
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
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Window resize and host DPI remain independent.")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                });
        },
    );
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
) {
    let size = egui::vec2(
        330.0_f32.min(workspace.width()),
        104.0_f32.min(workspace.height()),
    );
    let rect = egui::Rect::from_center_size(
        egui::pos2(workspace.center().x, workspace.top() + size.y * 0.5),
        size,
    );
    ui.painter()
        .rect_filled(rect, 3.0, editor_theme::semantic().surface);
    with_child(
        ui,
        rect.shrink(10.0),
        "save-preset-panel",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label("SAVE PRESET");
            let response = ui.text_edit_singleline(&mut presets.save_name);
            if presets.save_focus {
                response.request_focus();
                presets.save_focus = false;
            }
            let submit =
                response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
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
                if ui.button("CANCEL").clicked() {
                    presets.save_open = false;
                }
                if let Some(error) = presets.error.as_deref() {
                    ui.label(
                        egui::RichText::new(error)
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().danger),
                    );
                }
            });
        },
    );
}

fn group_accent(group_id: GroupId) -> egui::Color32 {
    let palette = editor_theme::semantic();
    let accents = [
        palette.primary,
        palette.unison,
        palette.pan_shape,
        palette.envelope,
        palette.danger,
    ];
    let index = group_id.get().wrapping_mul(0x9E37_79B9) as usize % accents.len();
    accents[index]
}

fn draw_generator_group(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    let patch = state.generator_stack.snapshot();
    let compact_text_height = editor_theme::font::caption().size + editor_theme::font::value().size;
    let card_height = (rect.width() * 0.145)
        .min(rect.height() * 0.34)
        .max(compact_text_height * 4.2);
    let output_height = card_height * 0.40;
    let alt_held = ui.input(|input| input.modifiers.alt);
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
                    for (group_index, group) in patch.groups().iter().enumerate() {
                        draw_generator_insert_zone(ui, state, &patch, group_index, section_gap);
                        let group_id = group.id();
                        let group_accent = group_accent(group_id);
                        let modules: Vec<_> = group
                            .modules()
                            .iter()
                            .filter_map(|module| {
                                module.oscillator_slot().map(|slot| (module.id(), slot))
                            })
                            .collect();
                        let group_top = ui.cursor().top();

                        for (visible, (module_id, slot)) in modules.iter().enumerate() {
                            draw_group_module_insert_zone(
                                ui,
                                state,
                                &patch,
                                group_id,
                                visible,
                                section_gap,
                            );
                            let (card, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), card_height),
                                egui::Sense::hover(),
                            );
                            draw_compact_oscillator(ui, state, card, *slot, *module_id, gap);
                        }
                        draw_group_module_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_id,
                            modules.len(),
                            section_gap,
                        );

                        let next_oscillator = (0..MAX_OSCILLATORS)
                            .filter_map(OscillatorSlot::from_index)
                            .find(|slot| !patch.contains_oscillator_slot(*slot));
                        if alt_held
                            && let Some(action) = draw_generator_add_menu(
                                ui,
                                egui::Id::new(("generator-add-menu", group_id.get())),
                                next_oscillator.is_some(),
                                patch.groups().len() < MAX_OUTPUT_PAIRS,
                            )
                        {
                            match action {
                                GeneratorAddAction::Oscillator => {
                                    if let Some(slot) = next_oscillator {
                                        add_oscillator_to_group(state, group_id, slot);
                                    }
                                }
                                GeneratorAddAction::Group => {
                                    state.generator_stack.edit(|patch| {
                                        if let Ok(id) = patch.insert_group(group_index + 1) {
                                            let output = GroupOutput {
                                                pair: ((group_index + 1) % MAX_OUTPUT_PAIRS) as u8,
                                                ..GroupOutput::default()
                                            };
                                            let _ = patch.set_group_output(id, output);
                                        }
                                    });
                                }
                            }
                        }

                        ui.add_space(section_gap);
                        let (footer, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), output_height),
                            egui::Sense::hover(),
                        );
                        let remove_group = draw_group_output(
                            ui,
                            state,
                            footer,
                            group_id,
                            group_index,
                            patch.groups().len() > 1,
                            group.output(),
                        );

                        let group_rect = egui::Rect::from_min_max(
                            egui::pos2(footer.left(), group_top),
                            footer.right_bottom(),
                        );
                        ui.painter().rect_stroke(
                            group_rect,
                            2.0,
                            egui::Stroke::new(1.0_f32, group_accent.gamma_multiply(0.62)),
                            egui::StrokeKind::Inside,
                        );

                        if remove_group {
                            remove_generator_group(state, group_id, &modules);
                        }
                    }
                    if alt_held {
                        draw_generator_insert_zone(
                            ui,
                            state,
                            &patch,
                            patch.groups().len(),
                            section_gap,
                        );
                        let next_oscillator = (0..MAX_OSCILLATORS)
                            .filter_map(OscillatorSlot::from_index)
                            .find(|slot| !patch.contains_oscillator_slot(*slot));
                        if let Some(action) = draw_generator_add_menu(
                            ui,
                            egui::Id::new("generator-add-menu-root"),
                            next_oscillator.is_some(),
                            patch.groups().len() < MAX_OUTPUT_PAIRS,
                        ) {
                            match action {
                                GeneratorAddAction::Oscillator => {
                                    if let Some(slot) = next_oscillator {
                                        add_oscillator_to_new_group(state, slot);
                                    }
                                }
                                GeneratorAddAction::Group => {
                                    state.generator_stack.edit(|patch| {
                                        let insertion = patch.groups().len();
                                        if let Ok(id) = patch.insert_group(insertion) {
                                            let output = GroupOutput {
                                                pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                                                ..GroupOutput::default()
                                            };
                                            let _ = patch.set_group_output(id, output);
                                        }
                                    });
                                }
                            }
                        }
                    }
                });
        },
    );
}

fn draw_generator_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    insertion: usize,
    height: f32,
) {
    let alt_held = ui.input(|input| input.modifiers.alt);
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let group_drag = egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx());
    if !module_drag && !group_drag {
        return;
    }
    let (zone, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(5.0)),
        egui::Sense::hover(),
    );
    let target = zone.expand2(egui::vec2(0.0, if alt_held { 12.0 } else { 5.0 }));
    let response = ui.interact(
        target,
        egui::Id::new(("generator-stack-insert", insertion)),
        egui::Sense::hover(),
    );
    let module_hovered = response.dnd_hover_payload::<ModuleId>().is_some();
    let group_hovered = response.dnd_hover_payload::<GroupId>().is_some();
    let module_at_capacity = module_hovered && patch.groups().len() >= MAX_OUTPUT_PAIRS;
    let hovered = group_hovered || (module_hovered && !module_at_capacity);
    let color = if module_at_capacity {
        editor_theme::semantic().text_muted
    } else {
        editor_theme::semantic().primary
    };
    let line = [
        egui::pos2(target.left() + 6.0, target.center().y),
        egui::pos2(target.right() - 6.0, target.center().y),
    ];
    if hovered {
        ui.painter()
            .rect_filled(target, 1.0, translucent(color, 20));
    }
    ui.painter().add(egui::Shape::dashed_line(
        &line,
        egui::Stroke::new(if hovered { 1.6_f32 } else { 1.0_f32 }, color),
        5.0,
        4.0,
    ));
    if module_at_capacity {
        ui.painter().text(
            target.right_center() - egui::vec2(8.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "8 GROUP LIMIT",
            editor_theme::font::caption(),
            color,
        );
    }
    if let Some(module_id) = response.dnd_release_payload::<ModuleId>()
        && patch.groups().len() < MAX_OUTPUT_PAIRS
    {
        state.generator_stack.edit(|patch| {
            if let Ok(group_id) = patch.insert_group(insertion) {
                let output = GroupOutput {
                    pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                    ..GroupOutput::default()
                };
                let _ = patch.set_group_output(group_id, output);
                let _ = patch.move_module(*module_id, group_id, 0);
            }
        });
    } else if let Some(group_id) = response.dnd_release_payload::<GroupId>() {
        move_group_to_insertion(state, patch, *group_id, insertion);
    }
}

fn draw_group_module_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
    height: f32,
) {
    if !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx()) {
        return;
    }
    let (zone, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height.max(4.0)),
        egui::Sense::hover(),
    );

    let alt_held = ui.input(|input| input.modifiers.alt);
    let target = zone.expand2(egui::vec2(0.0, if alt_held { 13.0 } else { 5.0 }));
    let response = ui.interact(
        target,
        egui::Id::new(("generator-module-insert", group_id.get(), insertion)),
        egui::Sense::hover(),
    );
    let hovered_module = response.dnd_hover_payload::<ModuleId>();
    let source_group = hovered_module.and_then(|module_id| {
        patch.groups().iter().find_map(|group| {
            group
                .modules()
                .iter()
                .any(|module| module.id() == *module_id)
                .then_some(group.id())
        })
    });
    let hovered = source_group.is_some_and(|source| source == group_id || alt_held);
    let color = editor_theme::semantic().primary;
    if hovered {
        ui.painter()
            .rect_filled(target, 1.0, translucent(color, 22));
    }
    let line = [
        egui::pos2(target.left() + 8.0, zone.center().y),
        egui::pos2(target.right() - 8.0, zone.center().y),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &line,
        egui::Stroke::new(if hovered { 1.6_f32 } else { 1.0_f32 }, color),
        5.0,
        4.0,
    ));

    if let Some(module_id) = response.dnd_release_payload::<ModuleId>()
        && (source_group == Some(group_id) || alt_held)
    {
        move_module_to_group(state, patch, *module_id, group_id, insertion);
    }
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
    let mut action = None;
    let mut open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let (button_rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), editor_theme::title_height(ui)),
        egui::Sense::click(),
    );
    let palette = editor_theme::semantic();
    if open || response.hovered() {
        ui.painter().rect_filled(
            button_rect,
            1.0,
            if open {
                palette.control
            } else {
                palette.surface
            },
        );
    }
    let outline = [
        button_rect.left_top(),
        button_rect.right_top(),
        button_rect.right_bottom(),
        button_rect.left_bottom(),
        button_rect.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        egui::Stroke::new(
            1.0_f32,
            if response.hovered() {
                palette.primary
            } else {
                palette.grid
            },
        ),
        button_rect.height() * 0.20,
        button_rect.height() * 0.16,
    ));
    ui.painter().text(
        button_rect.left_center() + egui::vec2(button_rect.width() * 0.012, 0.0),
        egui::Align2::LEFT_CENTER,
        "+ ADD MODULE",
        editor_theme::font::label(),
        if response.hovered() || open {
            palette.text
        } else {
            palette.text_muted
        },
    );
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
        let popup = egui::Area::new(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(response.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(editor_theme::semantic().surface)
                    .stroke(egui::Stroke::new(1.0_f32, editor_theme::semantic().grid))
                    .inner_margin(egui::Margin::same(frame_margin))
                    .show(ui, |ui| {
                        let row_height = ui.spacing().interact_size.y * 0.9;
                        let popup_width = (button_rect.width() * 0.24)
                            .clamp(ui.spacing().interact_size.x * 5.0, button_rect.width());
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
                    !response.rect.contains(pointer) && !popup.response.rect.contains(pointer)
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
    slot: OscillatorSlot,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insert_at = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| group.modules().len());
        patch
            .insert_oscillator_with_slot(group_id, insert_at, slot)
            .is_ok()
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn add_oscillator_to_new_group(state: &PluginContext<KurvParams>, slot: OscillatorSlot) {
    let inserted = state.generator_stack.edit(|patch| {
        let insertion = patch.groups().len();
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
        for (_, slot) in modules {
            let mut config = state.generator_stack.oscillator_config(*slot);
            config.enabled = false;
            state.generator_stack.set_oscillator_config(*slot, config);
        }
    }
}

fn draw_group_output(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    group_index: usize,
    can_remove_group: bool,
    mut output: GroupOutput,
) -> bool {
    let palette = editor_theme::semantic();
    ui.painter().rect_filled(rect, 2.0, palette.chrome);
    let before = output;
    let inset = rect.shrink2(egui::vec2(rect.width() * 0.012, rect.height() * 0.10));
    let graph_width = inset.width() * 0.24;
    let graph = egui::Rect::from_min_size(inset.min, egui::vec2(graph_width, inset.height()));
    let controls = egui::Rect::from_min_max(
        egui::pos2(graph.right() + inset.width() * 0.012, inset.top()),
        inset.max,
    );
    let group_drag = ui
        .interact(
            graph,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move this whole generator group");
    group_drag.dnd_set_drag_payload(group_id);
    if group_drag.dragged() {
        ui.painter().rect_filled(
            graph,
            1.0,
            translucent(
                palette.primary,
                (graph.height() * 0.10).clamp(0.0, 255.0) as u8,
            ),
        );
    }
    paint_group_envelope(ui, graph, output, palette.envelope);
    ui.painter().text(
        graph.left_top() + egui::vec2(graph.width() * 0.06, graph.height() * 0.05),
        egui::Align2::LEFT_TOP,
        format!("GROUP {}", group_index + 1),
        editor_theme::font::caption(),
        palette.text_muted,
    );
    let remove_size = graph.height() * 0.20;
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            graph.right() - graph.width() * 0.08,
            graph.top() + remove_size * 0.60,
        ),
        egui::vec2(graph.width() * 0.16, remove_size),
    );
    let remove_response = ui
        .interact(
            remove_rect,
            egui::Id::new(("generator-group-remove", group_id.get())),
            if can_remove_group {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_text("Remove this group and its modules");
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::caption(),
        if remove_response.hovered() {
            palette.text
        } else {
            palette.text_muted
        },
    );

    let row_gap = controls.height() * 0.08;
    let top_height = (controls.height() - row_gap) * 0.48;
    let top = egui::Rect::from_min_max(
        controls.min,
        egui::pos2(controls.right(), controls.top() + top_height),
    );
    let envelope_row = egui::Rect::from_min_max(
        egui::pos2(controls.left(), top.bottom() + row_gap),
        controls.max,
    );
    let top_cells = even_cells(top, 3);
    let envelope_cells = even_cells(envelope_row, 4);
    with_child(
        ui,
        top_cells[0],
        ("group-output-gain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            group_scalar_readout(
                ui,
                &mut output.gain,
                "GAIN",
                0.0..=2.0,
                0.01,
                GroupOutput::default().gain,
                top_cells[0].size(),
                format_gain,
                palette.primary,
            );
        },
    );
    with_child(
        ui,
        top_cells[1],
        ("group-output-pan", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            group_scalar_readout(
                ui,
                &mut output.pan,
                "PAN",
                -1.0..=1.0,
                0.01,
                GroupOutput::default().pan,
                top_cells[1].size(),
                format_pan_value,
                palette.primary,
            );
        },
    );
    with_child(
        ui,
        top_cells[2],
        ("group-output-send", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.label(
                egui::RichText::new("SEND TO DAW")
                    .font(editor_theme::font::caption())
                    .color(palette.text_muted),
            );
            let pair = egui::ComboBox::from_id_salt(("group-output-pair", group_id.get()))
                .selected_text(output_pair_label(output.pair))
                .width(top_cells[2].width())
                .show_ui(ui, |ui| {
                    for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                        ui.selectable_value(&mut output.pair, pair, output_pair_label(pair));
                    }
                });
            if pair.response.double_clicked() {
                output.pair = GroupOutput::default().pair;
            }
        },
    );
    for (cell, (label, value, range, speed, default, format)) in envelope_cells.into_iter().zip([
        (
            "ATTACK",
            &mut output.attack,
            0.0..=20.0,
            0.01,
            GroupOutput::default().attack,
            format_seconds as fn(f32) -> String,
        ),
        (
            "DECAY",
            &mut output.decay,
            0.0..=20.0,
            0.01,
            GroupOutput::default().decay,
            format_seconds as fn(f32) -> String,
        ),
        (
            "SUSTAIN",
            &mut output.sustain,
            0.0..=1.0,
            0.01,
            GroupOutput::default().sustain,
            format_percent as fn(f32) -> String,
        ),
        (
            "RELEASE",
            &mut output.release,
            0.0..=20.0,
            0.01,
            GroupOutput::default().release,
            format_seconds as fn(f32) -> String,
        ),
    ]) {
        with_child(
            ui,
            cell,
            ("group-output-envelope", group_id.get(), label),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                group_scalar_readout(
                    ui,
                    value,
                    label,
                    range,
                    speed,
                    default,
                    cell.size(),
                    format,
                    palette.primary,
                );
            },
        );
    }
    if output != before {
        state.generator_stack.set_group_output(group_id, output);
    }
    remove_response.clicked()
}

fn even_cells(rect: egui::Rect, count: usize) -> Vec<egui::Rect> {
    (0..count)
        .map(|index| {
            let start = rect.left() + rect.width() * index as f32 / count as f32;
            let end = rect.left() + rect.width() * (index + 1) as f32 / count as f32;
            egui::Rect::from_min_max(
                egui::pos2(start, rect.top()),
                egui::pos2(end, rect.bottom()),
            )
        })
        .collect()
}

fn paint_group_envelope(
    ui: &egui::Ui,
    rect: egui::Rect,
    output: GroupOutput,
    accent: egui::Color32,
) {
    let painter = ui.painter_at(rect);
    let plot = rect.shrink2(egui::vec2(rect.width() * 0.06, rect.height() * 0.12));
    let total = output.attack + output.decay + output.release + 1.0;
    let attack_end = egui::lerp(plot.left()..=plot.right(), output.attack / total);
    let decay_end = egui::lerp(
        plot.left()..=plot.right(),
        (output.attack + output.decay) / total,
    );
    let release_end = egui::lerp(
        plot.left()..=plot.right(),
        (output.attack + output.decay + output.release) / total,
    );
    let points = vec![
        plot.left_bottom(),
        egui::pos2(attack_end, plot.top()),
        egui::pos2(
            decay_end,
            egui::lerp(plot.bottom()..=plot.top(), output.sustain),
        ),
        egui::pos2(
            release_end,
            egui::lerp(plot.bottom()..=plot.top(), output.sustain),
        ),
        plot.right_bottom(),
    ];
    painter.rect_filled(rect, 1.0, editor_theme::semantic().well);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(rect.height() * 0.025, accent),
    ));
    painter.text(
        rect.left_top() + egui::vec2(rect.width() * 0.06, rect.height() * 0.05),
        egui::Align2::LEFT_TOP,
        "OUTPUT ENV",
        editor_theme::font::caption(),
        editor_theme::semantic().text_muted,
    );
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
) -> bool {
    let (rect, response, changed) = config_scalar_drag(ui, value, range, speed, default, size);
    let painter = ui.painter_at(rect);
    let value_text = format_value(*value);
    let active = response.is_pointer_button_down_on() || response.dragged();
    painter.text(
        egui::pos2(rect.center().x, rect.top() + rect.height() * 0.14),
        egui::Align2::CENTER_TOP,
        label,
        fit_font_to_width(
            &painter,
            label,
            editor_theme::font::caption(),
            rect.width() * 0.86,
        ),
        accent.gamma_multiply(if active { 1.0 } else { 0.72 }),
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + rect.height() * 0.56),
        egui::Align2::CENTER_TOP,
        &value_text,
        fit_font_to_width(
            &painter,
            &value_text,
            editor_theme::font::value(),
            rect.width() * 0.86,
        ),
        if active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
    changed
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
    std::array::from_fn(|index| {
        let left = rect.left() + rect.width() * index as f32 * 0.20;
        let right = rect.left() + rect.width() * (index + 1) as f32 * 0.20;
        egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        )
    })
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
    let painter = ui.painter_at(rect);
    let accent = editor_theme::semantic().primary;
    let label_font = fit_font_to_width(
        &painter,
        label,
        editor_theme::font::caption(),
        rect.width() * 0.86,
    );
    let value_font = fit_font_to_width(
        &painter,
        value,
        editor_theme::font::value(),
        rect.width() * 0.86,
    );
    let text_x = rect.center().x;
    let label_y = rect.top() + rect.height() * 0.14;
    let value_y = rect.top() + rect.height() * 0.56;
    painter.text(
        egui::pos2(text_x, label_y),
        egui::Align2::CENTER_TOP,
        label,
        label_font,
        accent.gamma_multiply(if active { 1.0 } else { 0.64 }),
    );
    painter.text(
        egui::pos2(text_x, value_y),
        egui::Align2::CENTER_TOP,
        value,
        value_font,
        if active {
            ui.visuals().text_color()
        } else {
            accent
        },
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

fn paint_phaseplant_wave_selector(ui: &egui::Ui, rect: egui::Rect, normalized: f32) {
    let palette = editor_theme::semantic();
    let painter = ui.painter_at(rect);
    let track_x = rect.center().x;
    let track = egui::Rect::from_min_max(
        egui::pos2(
            track_x - rect.width() * 0.06,
            rect.top() + rect.height() * 0.18,
        ),
        egui::pos2(
            track_x + rect.width() * 0.06,
            rect.bottom() - rect.height() * 0.18,
        ),
    );
    painter.rect_filled(track, 0.0, palette.primary.gamma_multiply(0.25));
    let y = egui::lerp(track.bottom()..=track.top(), normalized.clamp(0.0, 1.0));
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(track.left(), y), track.right_bottom()),
        0.0,
        palette.primary,
    );
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(track.center().x, y),
            egui::vec2(rect.width() * 0.24, rect.width() * 0.24),
        ),
        1.0,
        palette.primary,
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
    let mut config = state.generator_stack.oscillator_config(slot);
    let enabled = config.enabled;
    let mut config_changed = false;
    let mut reset_requested = false;
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let panel_gap = (gap * 0.45).max(rect.width() * 0.0015);
    let inner = rect.shrink(panel_gap * 0.45);
    let identity_width = inner.width() * 0.055;
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let drag_handle = ui
        .interact(
            identity,
            egui::Id::new(("oscillator-group-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag this oscillator to another group");
    drag_handle.dnd_set_drag_payload(module_id);
    drag_handle.context_menu(|ui| {
        if ui.button("RESET OSCILLATOR").clicked() {
            reset_requested = true;
            ui.close();
        }
    });
    if drag_handle.dragged() {
        ui.painter().rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.5_f32, editor_theme::semantic().primary),
            egui::StrokeKind::Inside,
        );
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
    let waveform_rail_width = oscillator_panel.width() * 0.055;
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
    for panel in [oscillator_panel, unison_panel, pan_panel] {
        ui.painter()
            .rect_filled(panel, 1.0, editor_theme::semantic().well);
    }

    ui.painter()
        .rect_filled(identity, 0.0, editor_theme::semantic().chrome);
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
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - identity.width() * 0.20,
            identity.top() + identity.height() * 0.10,
        ),
        egui::vec2(identity.width() * 0.36, identity.height() * 0.16),
    );
    let remove_response = ui
        .interact(
            remove_rect,
            egui::Id::new(("oscillator-remove", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_text(format!("Remove Oscillator {} from this group", index + 1));
    let remove_requested = remove_response.clicked();
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::label(),
        if remove_response.hovered() {
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
                        let (changed, active) =
                            config_field_drag(ui, &mut config, field, cell.size());
                        config_changed |= changed;
                        readout_active[readout_index] |= active;
                    },
                );
            }
            let (level, semi, cents, pan, phase_position, phase_random) = (
                format!("{:.0} %", config.level * 100.0),
                format!("{:+.0} st", config.transpose),
                format!("{:+.1} ct", config.cents),
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
            config_changed |= config_wave_field(ui, &mut config.shape, waveform_rail.size());
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
                        config_changed |=
                            unison_config_readout(ui, &mut config, field, cell.size());
                    },
                );
            }
        },
    );
    for panel in [oscillator_panel, unison_panel, pan_panel] {
        ui.painter().rect_stroke(
            panel,
            1.0,
            egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
            egui::StrokeKind::Inside,
        );
    }
    if reset_requested {
        state.generator_stack.reset_oscillator(slot);
        return;
    }
    if config_changed {
        state.generator_stack.set_oscillator_config(slot, config);
    }
    if remove_requested
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
    {
        config.enabled = false;
        state.generator_stack.set_oscillator_config(slot, config);
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
    config: &mut crate::generators::OscillatorConfig,
    field: ConfigField,
    size: egui::Vec2,
) -> (bool, bool) {
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
    (
        changed,
        response.is_pointer_button_down_on() || response.dragged(),
    )
}

fn unison_config_readout(
    ui: &mut egui::Ui,
    config: &mut crate::generators::OscillatorConfig,
    field: UnisonConfigField,
    size: egui::Vec2,
) -> bool {
    let defaults = crate::generators::OscillatorConfig::default();
    match field {
        UnisonConfigField::Voices => {
            let before = config.unison_voices;
            let mut voices = f32::from(before);
            let changed = config_scalar_readout(
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
            changed || config.unison_voices != before
        }
        UnisonConfigField::Range => config_scalar_readout(
            ui,
            &mut config.unison_range,
            "RANGE",
            0.0..=48.0,
            0.1,
            defaults.unison_range,
            size,
            |value| format!("{value:.2} st"),
        ),
        UnisonConfigField::Jitter => config_scalar_readout(
            ui,
            &mut config.unison_jitter,
            "JITTER",
            0.0..=1.0,
            0.005,
            defaults.unison_jitter,
            size,
            |value| format!("{:.0}%", value * 100.0),
        ),
        UnisonConfigField::Rate => config_scalar_readout(
            ui,
            &mut config.unison_rate,
            "RATE",
            0.0..=1.0,
            0.005,
            defaults.unison_rate,
            size,
            |value| format!("{:.2} Hz", normalized_unison_rate(value)),
        ),
        UnisonConfigField::Width => config_scalar_readout(
            ui,
            &mut config.unison_width,
            "WIDTH",
            0.0..=1.0,
            0.005,
            defaults.unison_width,
            size,
            |value| format!("{:.0}%", value * 100.0),
        ),
    }
}

fn config_wave_field(ui: &mut egui::Ui, value: &mut f32, size: egui::Vec2) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(18.0), size.y.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let before = *value;
    if response.dragged() {
        if ui.input(|input| input.modifiers.shift) {
            *value = (*value - response.drag_motion().y * 0.001).clamp(0.0, 3.0);
        } else if let Some(pointer) = response.interact_pointer_pos() {
            *value = (((rect.bottom() - pointer.y) / rect.height()).clamp(0.0, 1.0) * 3.0)
                .clamp(0.0, 3.0);
        }
    } else if response.double_clicked() || response.secondary_clicked() {
        *value = crate::generators::OscillatorConfig::default().shape;
    }
    paint_phaseplant_wave_selector(ui, rect, *value / 3.0);
    response.on_hover_text("Drag vertically to select the VA waveform; double-click for saw.");
    value.to_bits() != before.to_bits()
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
) -> bool {
    let (rect, response, changed) = config_scalar_drag(ui, value, range, speed, default, size);
    let painter = ui.painter_at(rect);
    let value_text = format_value(*value);
    let label_font = fit_font_to_width(
        &painter,
        label,
        egui::FontId::new(6.75, egui::FontFamily::Proportional),
        rect.width() - 7.0,
    );
    let value_font = fit_font_to_width(
        &painter,
        &value_text,
        egui::FontId::new(8.75, egui::FontFamily::Proportional),
        rect.width() - 7.0,
    );
    let text_x = rect.left() + 3.5;
    let accent = editor_theme::semantic().unison;
    let active = response.is_pointer_button_down_on() || response.dragged();
    painter.text(
        egui::pos2(text_x, rect.top() + 1.5),
        egui::Align2::LEFT_TOP,
        label,
        label_font,
        accent,
    );
    painter.text(
        egui::pos2(text_x, rect.top() + 8.5),
        egui::Align2::LEFT_TOP,
        value_text,
        value_font,
        if active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
    changed
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

fn draw_envelope(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let inset = rect.shrink(8.0);
    let title_height = inset.height().min(20.0);
    ui.painter().text(
        inset.left_top(),
        egui::Align2::LEFT_TOP,
        "AMP ENVELOPE",
        editor_theme::font::title(),
        editor_theme::semantic().text_muted,
    );
    let graph = egui::Rect::from_min_max(
        egui::pos2(inset.left(), inset.top() + title_height),
        inset.right_bottom(),
    );
    with_child(
        ui,
        graph,
        "amp-envelope",
        egui::Layout::top_down(egui::Align::Min),
        |ui| envelope_view(ui, state, graph.height()),
    );
}

fn draw_modulation(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let inner = rect.shrink(8.0);
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
    let inner = rect.shrink(8.0);
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
