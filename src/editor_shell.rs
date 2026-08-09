use std::hash::Hash;
use std::time::Duration;

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{param_field_sized, param_toggle_dot};
use crate::editor_envelope::envelope_view;
use crate::editor_history::EditorHistory;
use crate::editor_oscillator::{
    antialiasing_selector_compact, quality_selector_compact, waveform_preview, waveform_view,
};
use crate::editor_presets::{PresetEntry, PresetStore};
use crate::editor_unison::{UnisonUiParams, pan_shape_view, stereo_square_view, unison_view};
use crate::generators::{GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, OscillatorSlot};
use crate::{KurvParams, P, editor, editor_theme, performance};

const UI_BUILD_VERSION: &str = "v0.8.0 | group-output-runtime";
const OSCILLATOR_CARD_HEIGHT: f32 = 138.0;
const GENERATOR_GROUP_BAR_HEIGHT: f32 = 28.0;
const ADD_MODULE_HEIGHT: f32 = 28.0;

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

#[derive(Clone, Copy)]
struct OscillatorUi {
    enabled: P,
    shape: P,
    pulse_width: P,
    warp_mode: P,
    warp_amount: P,
    custom_shape: P,
    transpose: P,
    cents: P,
    level: P,
    pan: P,
    unison: UnisonUiParams,
}

const OSCILLATORS: [OscillatorUi; 3] = [
    OscillatorUi {
        enabled: P::Osc1Enabled,
        shape: P::Shape,
        pulse_width: P::PulseWidth,
        warp_mode: P::Osc1WarpMode,
        warp_amount: P::Osc1WarpAmount,
        custom_shape: P::Osc1CustomShape,
        transpose: P::Osc1Transpose,
        cents: P::Osc1Cents,
        level: P::Osc1Level,
        pan: P::Osc1Pan,
        unison: UnisonUiParams::OSC1,
    },
    OscillatorUi {
        enabled: P::Osc2Enabled,
        shape: P::Osc2Shape,
        pulse_width: P::Osc2PulseWidth,
        warp_mode: P::Osc2WarpMode,
        warp_amount: P::Osc2WarpAmount,
        custom_shape: P::Osc2CustomShape,
        transpose: P::Osc2Transpose,
        cents: P::Osc2Cents,
        level: P::Osc2Level,
        pan: P::Osc2Pan,
        unison: UnisonUiParams::OSC2,
    },
    OscillatorUi {
        enabled: P::Osc3Enabled,
        shape: P::Osc3Shape,
        pulse_width: P::Osc3PulseWidth,
        warp_mode: P::Osc3WarpMode,
        warp_amount: P::Osc3WarpAmount,
        custom_shape: P::Osc3CustomShape,
        transpose: P::Osc3Transpose,
        cents: P::Osc3Cents,
        level: P::Osc3Level,
        pan: P::Osc3Pan,
        unison: UnisonUiParams::OSC3,
    },
];

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
    synchronize_legacy_generator_slots(state);
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

fn synchronize_legacy_generator_slots(state: &PluginContext<KurvParams>) {
    if state.generator_stack.is_materialized() {
        return;
    }
    let required = [
        true,
        state.get_param(P::Osc2Enabled) >= 0.5,
        state.get_param(P::Osc3Enabled) >= 0.5,
    ];
    let snapshot = state.generator_stack.snapshot();
    let missing: Vec<_> = required
        .into_iter()
        .enumerate()
        .filter_map(|(index, required)| {
            let slot = OscillatorSlot::from_index(index)?;
            (required && !snapshot.contains_oscillator_slot(slot)).then_some(slot)
        })
        .collect();

    state.generator_stack.edit(|patch| {
        let Some(group_id) = patch.groups().first().map(|group| group.id()) else {
            return;
        };
        for slot in missing {
            let insert_at = patch
                .groups()
                .first()
                .map_or(0, |group| group.modules().len());
            let _ = patch.insert_oscillator_with_slot(group_id, insert_at, slot);
        }
    });
}

fn draw_generator_group(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    synchronize_legacy_generator_slots(state);
    let patch = state.generator_stack.snapshot();
    let Some(group) = patch.groups().first() else {
        return;
    };
    let group_id = group.id();
    let oscillator_modules: Vec<_> = group
        .modules()
        .iter()
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .collect();
    let next_oscillator = (0..MAX_OSCILLATORS)
        .filter_map(OscillatorSlot::from_index)
        .find(|slot| !patch.contains_oscillator_slot(*slot));
    let bar_height = GENERATOR_GROUP_BAR_HEIGHT.min(rect.height() * 0.2);
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), bar_height));
    let stack = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom() + gap),
        rect.right_bottom(),
    );
    let oscillator_count = oscillator_modules.len();
    ui.painter()
        .rect_filled(header, 2.0, editor_theme::semantic().chrome);
    with_child(
        ui,
        header.shrink2(egui::vec2(8.0, 2.0)),
        "generator-group-header",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new("GROUP 1")
                    .font(editor_theme::font::label())
                    .color(editor_theme::semantic().text),
            );
            ui.label(
                egui::RichText::new("SHARED GENERATOR BUS")
                    .font(editor_theme::font::caption())
                    .color(editor_theme::semantic().text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{oscillator_count} OSCILLATOR{}",
                        if oscillator_count == 1 { "" } else { "S" }
                    ))
                    .font(editor_theme::font::caption())
                    .color(editor_theme::semantic().text_muted),
                );
            });
        },
    );
    with_child(
        ui,
        stack,
        "generator-module-stack",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("generator-module-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    for (visible, (module_id, slot)) in oscillator_modules.iter().enumerate() {
                        let index = slot.index();
                        if visible != 0 {
                            ui.add_space(section_gap);
                        }
                        let (card, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), OSCILLATOR_CARD_HEIGHT),
                            egui::Sense::hover(),
                        );
                        draw_compact_oscillator(ui, state, card, *slot, *module_id, index, gap);
                    }
                    ui.add_space(section_gap);
                    let add = ui
                        .add_enabled(
                            next_oscillator.is_some(),
                            egui::Button::new("+ ADD OSCILLATOR")
                                .min_size(egui::vec2(ui.available_width(), ADD_MODULE_HEIGHT)),
                        )
                        .on_hover_text(if next_oscillator.is_some() {
                            "Add the next available oscillator module"
                        } else {
                            "All 32 oscillator slots are in use"
                        });
                    if add.clicked()
                        && let Some(slot) = next_oscillator
                    {
                        let inserted = state.generator_stack.edit(|patch| {
                            let insert_at = patch
                                .groups()
                                .first()
                                .map_or(0, |group| group.modules().len());
                            patch
                                .insert_oscillator_with_slot(group_id, insert_at, slot)
                                .is_ok()
                        });
                        if inserted {
                            if let Some(oscillator) = OSCILLATORS.get(slot.index()) {
                                state.automate(oscillator.enabled, 1.0);
                            } else {
                                let mut config = state.generator_stack.oscillator_config(slot);
                                config.enabled = true;
                                state.generator_stack.set_oscillator_config(slot, config);
                            }
                        }
                    }
                    ui.add_space(section_gap);
                    let (footer, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), bar_height),
                        egui::Sense::hover(),
                    );
                    draw_group_output(ui, state, footer, group_id, group.output());
                });
        },
    );
}

fn draw_group_output(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    mut output: GroupOutput,
) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().chrome);
    let before = output;
    with_child(
        ui,
        rect.shrink2(egui::vec2(8.0, 2.0)),
        "group-output",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new("GROUP OUTPUT")
                    .font(editor_theme::font::label())
                    .color(editor_theme::semantic().text),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("GAIN")
                    .font(editor_theme::font::caption())
                    .color(editor_theme::semantic().text_muted),
            );
            ui.add(
                egui::DragValue::new(&mut output.gain)
                    .range(0.0..=2.0)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            ui.label(
                egui::RichText::new("PAN")
                    .font(editor_theme::font::caption())
                    .color(editor_theme::semantic().text_muted),
            );
            ui.add(
                egui::DragValue::new(&mut output.pan)
                    .range(-1.0..=1.0)
                    .speed(0.01)
                    .fixed_decimals(2),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt(("group-output-pair", group_id.get()))
                    .selected_text(output_pair_label(output.pair))
                    .width(104.0)
                    .show_ui(ui, |ui| {
                        for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                            ui.selectable_value(&mut output.pair, pair, output_pair_label(pair));
                        }
                    });
                ui.label(
                    egui::RichText::new("SEND TO DAW")
                        .font(editor_theme::font::caption())
                        .color(editor_theme::semantic().text_muted),
                );
            });
        },
    );
    if output != before {
        state.generator_stack.edit(|patch| {
            let _ = patch.set_group_output(group_id, output);
        });
    }
}

fn output_pair_label(pair: u8) -> String {
    let left = usize::from(pair) * 2 + 1;
    format!("OUT {left}/{}", left + 1)
}

fn draw_compact_oscillator(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    slot: OscillatorSlot,
    module_id: ModuleId,
    index: usize,
    gap: f32,
) {
    let mut config = state.generator_stack.oscillator_config(slot);
    let legacy = OSCILLATORS.get(index).copied();
    let enabled = legacy.map_or(config.enabled, |oscillator| {
        state.get_param(oscillator.enabled) >= 0.5
    });
    let mut config_changed = false;
    let mut remove_requested = false;
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let identity_width = (rect.width() * 42.0 / 700.0).clamp(34.0, 46.0);
    let inner = rect.shrink2(egui::vec2(gap.max(3.0), gap.max(3.0)));
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + gap, inner.top()),
        inner.right_bottom(),
    );
    let controls_height = 38.0_f32.min(body.height() * 0.34);
    let waveform = egui::Rect::from_min_max(
        body.min,
        egui::pos2(body.right(), body.bottom() - controls_height - gap),
    );
    let controls = egui::Rect::from_min_max(
        egui::pos2(body.left(), waveform.bottom() + gap),
        body.right_bottom(),
    );
    let shape_width = 44.0_f32.min(waveform.width() * 0.13);
    let pulse_width = 40.0_f32.min(waveform.width() * 0.12);
    let shape = egui::Rect::from_min_size(waveform.min, egui::vec2(shape_width, waveform.height()));
    let pulse_area = egui::Rect::from_min_size(
        egui::pos2(waveform.right() - pulse_width, waveform.top()),
        egui::vec2(pulse_width, waveform.height()),
    );
    let pulse_height = (pulse_area.height() * 0.61).max(36.0);
    let pulse =
        egui::Rect::from_min_size(pulse_area.min, egui::vec2(pulse_area.width(), pulse_height));
    let unison = egui::Rect::from_min_max(
        egui::pos2(pulse_area.left(), pulse.bottom() + gap),
        pulse_area.right_bottom(),
    );
    let graph = egui::Rect::from_min_max(
        egui::pos2(shape.right() + gap, waveform.top()),
        egui::pos2(pulse_area.left() - gap, waveform.bottom()),
    );

    ui.painter()
        .rect_filled(identity, 0.0, editor_theme::semantic().chrome);
    with_child(
        ui,
        identity,
        ("extended-osc-identity", index),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(4.0);
            if let Some(oscillator) = legacy {
                param_toggle_dot(ui, state, oscillator.enabled, identity.width().min(20.0));
            } else {
                config_changed |= compact_toggle(ui, &mut config.enabled);
            }
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
            remove_requested = ui
                .small_button("×")
                .on_hover_text(format!("Remove Oscillator {} from this group", index + 1))
                .clicked();
        },
    );
    with_child(
        ui,
        controls,
        ("extended-osc-controls", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            let cell_width = controls.width() * 0.25;
            for (cell_index, field) in [
                ConfigField::Level,
                ConfigField::Pan,
                ConfigField::Fine,
                ConfigField::Semi,
            ]
            .into_iter()
            .enumerate()
            {
                let cell = egui::Rect::from_min_size(
                    egui::pos2(
                        controls.left() + cell_index as f32 * cell_width,
                        controls.top(),
                    ),
                    egui::vec2(cell_width, controls.height()),
                );
                with_child(
                    ui,
                    cell,
                    ("compact-config", index, cell_index),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if let Some(oscillator) = legacy {
                            let (param, label) = match field {
                                ConfigField::Level => (oscillator.level, "LEVEL"),
                                ConfigField::Pan => (oscillator.pan, "PAN"),
                                ConfigField::Fine => (oscillator.cents, "FINE"),
                                ConfigField::Semi => (oscillator.transpose, "SEMI"),
                            };
                            param_field_sized(ui, state, param, label, cell.width(), cell.height());
                        } else {
                            config_changed |= config_field(ui, &mut config, field, cell.size());
                        }
                    },
                );
            }
        },
    );

    with_child(
        ui,
        shape,
        ("compact-wave-shape", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.35 });
            if let Some(oscillator) = legacy {
                param_field_sized(
                    ui,
                    state,
                    oscillator.shape,
                    "WAVE",
                    shape.width(),
                    shape.height(),
                );
            } else {
                config_changed |= config_wave_field(ui, &mut config.shape, shape.size());
            }
        },
    );
    with_child(
        ui,
        graph,
        ("compact-wave-cycle", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            if let Some(oscillator) = legacy {
                waveform_view(
                    ui,
                    state,
                    graph.width(),
                    graph.height(),
                    oscillator.shape,
                    oscillator.pulse_width,
                    oscillator.warp_mode,
                    oscillator.warp_amount,
                    oscillator.custom_shape,
                    index,
                );
            } else {
                waveform_preview(
                    ui,
                    graph.width(),
                    graph.height(),
                    config.shape,
                    config.pulse_width,
                );
            }
        },
    );
    with_child(
        ui,
        pulse,
        ("compact-pulse-width", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.35 });
            if let Some(oscillator) = legacy {
                param_field_sized(
                    ui,
                    state,
                    oscillator.pulse_width,
                    "PW",
                    pulse.width(),
                    pulse.height(),
                );
            } else {
                config_changed |= config_pulse_field(ui, &mut config.pulse_width, pulse.size());
            }
        },
    );
    draw_unison_access(ui, state, unison, legacy, index, enabled);

    if config_changed {
        state.generator_stack.set_oscillator_config(slot, config);
    }
    if remove_requested
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
        && let Some(oscillator) = legacy
    {
        state.automate(oscillator.enabled, 0.0);
    }
}

fn draw_unison_access(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    oscillator: Option<OscillatorUi>,
    index: usize,
    enabled: bool,
) {
    let Some(oscillator) = oscillator else {
        ui.painter()
            .rect_filled(rect, 1.0, editor_theme::semantic().chrome);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "1 VOICE",
            editor_theme::font::caption(),
            editor_theme::semantic().text_muted,
        );
        return;
    };
    let open_id = egui::Id::new(("oscillator-advanced-open", index));
    let mut open = ui
        .data(|data| data.get_temp::<bool>(open_id))
        .unwrap_or(false);
    let response = ui.interact(rect, open_id.with("button"), egui::Sense::click());
    if response.clicked() {
        open = !open;
    }
    ui.painter().rect_filled(
        rect,
        1.0,
        if open || response.hovered() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().chrome
        },
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("UNI  {}", state.format_param(oscillator.unison.voices)),
        editor_theme::font::caption(),
        if enabled {
            editor_theme::semantic().text
        } else {
            editor_theme::semantic().text_muted
        },
    );
    response.on_hover_text("Open this oscillator's unison and stereo controls");

    if open {
        egui::Window::new(format!("OSC {} · UNISON / STEREO", index + 1))
            .id(open_id.with("window"))
            .open(&mut open)
            .default_size(egui::vec2(620.0, 460.0))
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.set_min_width(620.0);
                    ui.horizontal(|ui| {
                        for (param, label) in [
                            (oscillator.unison.voices, "VOICES"),
                            (oscillator.unison.detune, "RANGE"),
                            (oscillator.unison.detune_amount, "AMOUNT"),
                            (oscillator.unison.phase, "PHASE"),
                            (oscillator.unison.jitter, "JITTER"),
                            (oscillator.unison.jitter_rate, "RATE"),
                            (oscillator.unison.stereo, "WIDTH"),
                        ] {
                            param_field_sized(ui, state, param, label, 82.0, 42.0);
                        }
                    });
                    ui.add_space(6.0);
                    ui.columns(2, |columns| {
                        let unison_width = columns[0].available_width();
                        let pan_shape_width = columns[1].available_width();
                        unison_view(
                            &mut columns[0],
                            state,
                            unison_width,
                            250.0,
                            oscillator.unison,
                            true,
                            true,
                        );
                        pan_shape_view(
                            &mut columns[1],
                            state,
                            pan_shape_width,
                            250.0,
                            oscillator.unison,
                            true,
                        );
                    });
                    ui.add_space(6.0);
                    let width = ui.available_width().min(300.0);
                    let _ = stereo_square_view(ui, state, width, 150.0, oscillator.unison);
                });
            });
    }
    ui.data_mut(|data| data.insert_temp(open_id, open));
}

#[derive(Clone, Copy)]
enum ConfigField {
    Level,
    Pan,
    Fine,
    Semi,
}

fn compact_toggle(ui: &mut egui::Ui, enabled: &mut bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let clicked = response.clicked();
    if clicked {
        *enabled = !*enabled;
    }
    let color = if *enabled {
        editor_theme::palette().accent
    } else {
        editor_theme::semantic().grid
    };
    ui.painter().circle_filled(rect.center(), 5.0, color);
    response.on_hover_text(if *enabled {
        "Disable oscillator"
    } else {
        "Enable oscillator"
    });
    clicked
}

fn config_field(
    ui: &mut egui::Ui,
    config: &mut crate::generators::OscillatorConfig,
    field: ConfigField,
    size: egui::Vec2,
) -> bool {
    match field {
        ConfigField::Level => config_scalar_field(
            ui,
            &mut config.level,
            "LEVEL",
            0.0..=1.0,
            0.005,
            0.5,
            size,
            false,
            |value| format!("{:.0}%", value * 100.0),
        ),
        ConfigField::Pan => config_scalar_field(
            ui,
            &mut config.pan,
            "PAN",
            -1.0..=1.0,
            0.01,
            0.0,
            size,
            false,
            format_pan,
        ),
        ConfigField::Fine => config_scalar_field(
            ui,
            &mut config.cents,
            "FINE",
            -100.0..=100.0,
            0.25,
            0.0,
            size,
            false,
            |value| format!("{value:+.1} ct"),
        ),
        ConfigField::Semi => {
            let before = config.transpose;
            let changed = config_scalar_field(
                ui,
                &mut config.transpose,
                "SEMI",
                -48.0..=48.0,
                1.0,
                0.0,
                size,
                false,
                |value| format!("{value:+.0} st"),
            );
            config.transpose = config.transpose.round();
            changed || config.transpose.to_bits() != before.to_bits()
        }
    }
}

fn config_wave_field(ui: &mut egui::Ui, value: &mut f32, size: egui::Vec2) -> bool {
    config_scalar_field(
        ui,
        value,
        "WAVE",
        0.0..=3.0,
        0.01,
        2.0,
        size,
        true,
        format_wave_shape,
    )
}

fn config_pulse_field(ui: &mut egui::Ui, value: &mut f32, size: egui::Vec2) -> bool {
    config_scalar_field(
        ui,
        value,
        "PW",
        0.03..=0.97,
        0.005,
        0.5,
        size,
        true,
        |value| format!("{:.0}%", value * 100.0),
    )
}

#[allow(clippy::too_many_arguments)]
fn config_scalar_field(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
    vertical: bool,
    format_value: fn(f32) -> String,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(18.0), size.y.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(if vertical {
        egui::CursorIcon::ResizeVertical
    } else {
        egui::CursorIcon::ResizeHorizontal
    });
    let before = *value;
    let start = *range.start();
    let end = *range.end();
    if response.dragged() {
        if vertical {
            if ui.input(|input| input.modifiers.shift) {
                let delta = ui.input(|input| input.pointer.delta());
                *value = (*value - delta.y * speed * 0.1).clamp(start, end);
            } else if let Some(pointer) = response.interact_pointer_pos() {
                let normalized = ((rect.bottom() - pointer.y) / rect.height()).clamp(0.0, 1.0);
                *value = normalized.mul_add(end - start, start);
            }
        } else {
            let delta = ui.input(|input| input.pointer.delta());
            let precision = if ui.input(|input| input.modifiers.shift) {
                0.1
            } else {
                1.0
            };
            *value = (*value + (delta.x - delta.y) * speed * precision).clamp(start, end);
        }
    } else if response.double_clicked() {
        *value = default;
    }
    let normalized = ((*value - start) / (end - start)).clamp(0.0, 1.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        1.0,
        if response.hovered() || response.dragged() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().control
        },
    );
    let mut vertical_fill = None;
    if vertical {
        let fill_top = egui::lerp(rect.bottom()..=rect.top(), normalized);
        let fill = egui::Rect::from_min_max(egui::pos2(rect.left(), fill_top), rect.right_bottom());
        painter.rect_filled(fill, 0.0, editor_theme::semantic().primary);
        vertical_fill = Some(fill);
    } else {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 2.0),
                egui::pos2(
                    egui::lerp(rect.left()..=rect.right(), normalized),
                    rect.bottom(),
                ),
            ),
            0.0,
            editor_theme::palette().accent,
        );
    }
    let label_position = rect.center_top() + egui::vec2(0.0, 3.0);
    let value_position = rect.center_bottom() - egui::vec2(0.0, 3.0);
    let text_color = |position, fallback| {
        vertical_fill
            .filter(|fill| fill.contains(position))
            .map_or(fallback, |_| {
                editor_theme::readable_text(editor_theme::semantic().primary)
            })
    };
    painter.text(
        label_position,
        egui::Align2::CENTER_TOP,
        label,
        editor_theme::font::caption(),
        text_color(label_position, editor_theme::semantic().text_muted),
    );
    painter.text(
        value_position,
        egui::Align2::CENTER_BOTTOM,
        format_value(*value),
        editor_theme::font::value(),
        text_color(value_position, editor_theme::semantic().text),
    );
    response.on_hover_text("Drag to change. Hold Shift for fine control; double-click to reset.");
    value.to_bits() != before.to_bits()
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

fn format_wave_shape(value: f32) -> String {
    match value.round() as u8 {
        0 => "SIN".to_owned(),
        1 => "TRI".to_owned(),
        2 => "SAW".to_owned(),
        _ => "PULSE".to_owned(),
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
