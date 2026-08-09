use std::hash::Hash;
use std::time::Duration;

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{
    enum_cycle_field, param_field_sized, param_toggle_dot, shape_morph_strip,
};
use crate::editor_envelope::envelope_view;
use crate::editor_history::EditorHistory;
use crate::editor_oscillator::{
    antialiasing_selector_compact, quality_selector_compact, waveform_view,
};
use crate::editor_presets::{PresetEntry, PresetStore};
use crate::editor_unison::{UnisonUiParams, pan_shape_view, stereo_square_view, unison_view};
use crate::generators::{ModuleId, OscillatorSlot};
use crate::{KurvParams, P, editor, editor_theme, performance};

const UI_BUILD_VERSION: &str = "v0.8.0 | lfo-spline-runtime-simd";
const OSCILLATOR_CARD_HEIGHT: f32 = 210.0;
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
    let next_oscillator = (0..OSCILLATORS.len())
        .filter_map(OscillatorSlot::from_index)
        .find(|slot| !patch.contains_oscillator_slot(*slot));
    let bar_height = GENERATOR_GROUP_BAR_HEIGHT.min(rect.height() * 0.2);
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), bar_height));
    let footer = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - bar_height),
        egui::vec2(rect.width(), bar_height),
    );
    let stack = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom() + gap),
        egui::pos2(
            rect.right(),
            (footer.top() - gap).max(header.bottom() + gap),
        ),
    );
    let oscillator_count = oscillator_modules.len();

    for (bar, id) in [(header, "generator-group-header"), (footer, "group-output")] {
        ui.painter()
            .rect_filled(bar, 2.0, editor_theme::semantic().chrome);
        with_child(
            ui,
            bar.shrink2(egui::vec2(8.0, 2.0)),
            id,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let (label, detail) = if id == "generator-group-header" {
                    (
                        "GENERATOR GROUP",
                        format!(
                            "{oscillator_count} OSCILLATOR{}",
                            if oscillator_count == 1 { "" } else { "S" }
                        ),
                    )
                } else {
                    ("GROUP OUTPUT", "VOICE MIX".to_owned())
                };
                ui.label(
                    egui::RichText::new(label)
                        .font(editor_theme::font::label())
                        .color(editor_theme::semantic().text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(detail)
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                });
            },
        );
    }

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
                        let Some(oscillator) = OSCILLATORS.get(index).copied() else {
                            continue;
                        };
                        if visible != 0 {
                            ui.add_space(section_gap);
                        }
                        let (card, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), OSCILLATOR_CARD_HEIGHT),
                            egui::Sense::hover(),
                        );
                        draw_oscillator_row(ui, state, card, oscillator, *module_id, index, gap);
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
                            "The current DSP adapter has all three oscillator slots in use"
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
                            state.automate(OSCILLATORS[slot.index()].enabled, 1.0);
                        }
                    }
                });
        },
    );
}

fn draw_oscillator_row(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    oscillator: OscillatorUi,
    module_id: ModuleId,
    index: usize,
    gap: f32,
) {
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    let identity_width = (rect.width() * 28.0 / 700.0).clamp(18.0, 28.0);
    let mix_width = (rect.width() * 84.0 / 700.0).clamp(44.0, 84.0);
    let inner = rect.shrink2(egui::vec2(gap.max(3.0), gap.max(3.0)));
    let canvas_total = (inner.width() - identity_width - mix_width - gap * 4.0).max(120.0);
    let stereo_width = (canvas_total * 0.36).clamp(136.0_f32.min(canvas_total * 0.48), 220.0);
    let canvas_width = ((canvas_total - stereo_width) * 0.5).max(32.0);

    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let mix = egui::Rect::from_min_size(
        egui::pos2(identity.right() + gap, inner.top()),
        egui::vec2(mix_width, inner.height()),
    );
    let waveform = egui::Rect::from_min_size(
        egui::pos2(mix.right() + gap, inner.top()),
        egui::vec2(canvas_width, inner.height()),
    );
    let dual = egui::Rect::from_min_size(
        egui::pos2(waveform.right() + gap, inner.top()),
        egui::vec2(canvas_width, inner.height()),
    );
    let stereo = egui::Rect::from_min_max(
        egui::pos2(dual.right() + gap, inner.top()),
        egui::pos2(
            (dual.right() + gap + stereo_width).min(inner.right()),
            inner.bottom(),
        ),
    );

    ui.painter()
        .rect_filled(identity, 0.0, editor_theme::semantic().chrome);
    let enabled = state.get_param(oscillator.enabled) >= 0.5;
    let mut remove_requested = false;
    with_child(
        ui,
        identity,
        ("osc-identity", index),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(5.0);
            if index == 0 {
                param_toggle_dot(ui, state, oscillator.enabled, identity.width().min(22.0))
                    .on_hover_text("Turn Oscillator 1 on or off; its card remains in the group");
            } else {
                param_toggle_dot(ui, state, oscillator.enabled, identity.width().min(22.0))
                    .on_hover_text(format!("Turn Oscillator {} on or off", index + 1));
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
            if index != 0 {
                remove_requested = ui
                    .small_button("×")
                    .on_hover_text(format!("Remove Oscillator {} from this group", index + 1))
                    .clicked();
            }
        },
    );

    if remove_requested
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
    {
        state.automate(oscillator.enabled, 0.0);
    }

    draw_mix_controls(ui, state, mix, oscillator, index, enabled);
    draw_waveform(ui, state, waveform, oscillator, index, enabled);
    with_child(
        ui,
        dual,
        ("osc-unison-area", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            draw_dual_unison_canvas(ui, state, dual, oscillator.unison, index);
        },
    );
    with_child(
        ui,
        stereo,
        ("osc-stereo-area", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            draw_compact_stereo(ui, state, stereo, oscillator.unison, index, gap);
        },
    );
}

fn draw_mix_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    oscillator: OscillatorUi,
    index: usize,
    enabled: bool,
) {
    with_child(
        ui,
        rect,
        ("osc-mix", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            let cell_width = (rect.width() - 2.0) * 0.5;
            let cell_height = (rect.height() - 2.0) * 0.5;
            for row in [
                [(oscillator.transpose, "SEMI"), (oscillator.cents, "FINE")],
                [(oscillator.level, "LEVEL"), (oscillator.pan, "PAN")],
            ] {
                ui.horizontal(|ui| {
                    for (param, label) in row {
                        param_field_sized(ui, state, param, label, cell_width, cell_height);
                    }
                });
            }
        },
    );
}

fn draw_waveform(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    oscillator: OscillatorUi,
    index: usize,
    enabled: bool,
) {
    let header_height = (rect.height() * 0.2).clamp(22.0, 36.0);
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
    let graph = egui::Rect::from_min_max(
        egui::pos2(rect.left(), header.bottom() + 2.0),
        rect.right_bottom(),
    );
    with_child(
        ui,
        header,
        ("osc-wave-header", index),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            ui.spacing_mut().item_spacing.x = 2.0;
            let strip_width = (header.width() * 0.42).max(68.0);
            let pulse_width = (header.width() * 0.14).max(28.0);
            let mode_width = (header.width() * 0.22).max(34.0);
            let amount_width =
                (header.width() - strip_width - pulse_width - mode_width - 6.0).max(26.0);
            shape_morph_strip(
                ui,
                state,
                oscillator.shape,
                oscillator.custom_shape,
                strip_width,
                header.height(),
            );
            param_field_sized(
                ui,
                state,
                oscillator.pulse_width,
                "PW",
                pulse_width,
                header.height(),
            );
            enum_cycle_field(
                ui,
                state,
                oscillator.warp_mode,
                "WARP",
                &["NONE", "PWM", "BEND", "HARM"],
                mode_width,
                header.height(),
            );
            param_field_sized(
                ui,
                state,
                oscillator.warp_amount,
                "AMT",
                amount_width,
                header.height(),
            );
        },
    );
    with_child(
        ui,
        graph,
        ("osc-wave", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.30 });
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
        },
    );
}

fn draw_dual_unison_canvas(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    params: UnisonUiParams,
    oscillator: usize,
) {
    let toggle_height = (rect.height() * 0.14).clamp(20.0, 28.0);
    let toggle = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), toggle_height));
    let unison_button =
        egui::Rect::from_min_max(toggle.min, egui::pos2(toggle.center().x, toggle.bottom()));
    let shaper_button = egui::Rect::from_min_max(
        egui::pos2(toggle.center().x, toggle.top()),
        toggle.right_bottom(),
    );
    let mode_id = egui::Id::new(("osc-unison-mode", oscillator));
    let mut shaper = ui
        .data(|data| data.get_temp::<bool>(mode_id))
        .unwrap_or(false);
    if ui
        .interact(unison_button, mode_id.with("unison"), egui::Sense::click())
        .clicked()
    {
        shaper = false;
    }
    if ui
        .interact(shaper_button, mode_id.with("shaper"), egui::Sense::click())
        .clicked()
    {
        shaper = true;
    }
    ui.data_mut(|data| data.insert_temp(mode_id, shaper));
    for (button, active, label, active_color) in [
        (
            unison_button,
            !shaper,
            "UNISON",
            editor_theme::semantic().unison,
        ),
        (
            shaper_button,
            shaper,
            "PAN SHAPE",
            editor_theme::semantic().pan_shape,
        ),
    ] {
        ui.painter().rect_filled(
            button,
            1.0,
            if active {
                editor_theme::semantic().control
            } else {
                editor_theme::semantic().chrome
            },
        );
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            label,
            editor_theme::font::caption(),
            if active {
                active_color
            } else {
                editor_theme::semantic().text_muted
            },
        );
    }

    let canvas = egui::Rect::from_min_max(
        egui::pos2(rect.left(), toggle.bottom() + 2.0),
        rect.right_bottom(),
    );
    if shaper {
        with_child(
            ui,
            canvas,
            ("unison-underlay", oscillator),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_opacity(0.28);
                unison_view(
                    ui,
                    state,
                    canvas.width(),
                    canvas.height(),
                    params,
                    false,
                    true,
                );
            },
        );
        with_child(
            ui,
            canvas,
            ("shaper-overlay", oscillator),
            egui::Layout::top_down(egui::Align::Min),
            |ui| pan_shape_view(ui, state, canvas.width(), canvas.height(), params, false),
        );
    } else {
        with_child(
            ui,
            canvas,
            ("unison-canvas", oscillator),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                unison_view(
                    ui,
                    state,
                    canvas.width(),
                    canvas.height(),
                    params,
                    true,
                    true,
                );
            },
        );
    }
}

fn draw_compact_stereo(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    params: UnisonUiParams,
    oscillator: usize,
    gap: f32,
) {
    let side_stacks = rect.width() > rect.height() * 1.2;
    let (field, controls) = if side_stacks {
        let cell_width = (rect.width() * 0.22).clamp(22.0, 46.0);
        let cell_height = ((rect.height() - gap * 3.0) / 4.0).max(12.0);
        let field = egui::Rect::from_min_max(
            egui::pos2(rect.left() + cell_width + gap, rect.top()),
            egui::pos2(rect.right() - cell_width - gap, rect.bottom()),
        );
        let mut controls = Vec::with_capacity(7);
        for (row, (param, label)) in [
            (params.voices, "VOICES"),
            (params.jitter, "JITTER"),
            (params.detune, "RANGE"),
            (params.harmonic_align, "HARM"),
        ]
        .into_iter()
        .enumerate()
        {
            controls.push((
                egui::Rect::from_min_size(
                    egui::pos2(rect.left(), rect.top() + row as f32 * (cell_height + gap)),
                    egui::vec2(cell_width, cell_height),
                ),
                param,
                label,
            ));
        }
        let right_top = rect.top() + 0.5 * (cell_height + gap);
        for (row, (param, label)) in [
            (params.phase, "PHASE"),
            (params.jitter_rate, "RATE"),
            (params.stereo, "WIDTH"),
        ]
        .into_iter()
        .enumerate()
        {
            controls.push((
                egui::Rect::from_min_size(
                    egui::pos2(
                        rect.right() - cell_width,
                        right_top + row as f32 * (cell_height + gap),
                    ),
                    egui::vec2(cell_width, cell_height),
                ),
                param,
                label,
            ));
        }
        (field, controls)
    } else {
        let cell_height = (rect.height() * 0.22).clamp(22.0, 30.0);
        let cell_width = ((rect.width() - gap * 3.0) / 4.0).max(18.0);
        let field = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + cell_height + gap),
            egui::pos2(rect.right(), rect.bottom() - cell_height - gap),
        );
        let mut controls = Vec::with_capacity(7);
        for (column, (param, label)) in [
            (params.voices, "VOICES"),
            (params.jitter, "JITTER"),
            (params.detune, "RANGE"),
            (params.harmonic_align, "HARM"),
        ]
        .into_iter()
        .enumerate()
        {
            controls.push((
                egui::Rect::from_min_size(
                    egui::pos2(rect.left() + column as f32 * (cell_width + gap), rect.top()),
                    egui::vec2(cell_width, cell_height),
                ),
                param,
                label,
            ));
        }
        let bottom_left = rect.left() + 0.5 * (cell_width + gap);
        for (column, (param, label)) in [
            (params.phase, "PHASE"),
            (params.jitter_rate, "RATE"),
            (params.stereo, "WIDTH"),
        ]
        .into_iter()
        .enumerate()
        {
            controls.push((
                egui::Rect::from_min_size(
                    egui::pos2(
                        bottom_left + column as f32 * (cell_width + gap),
                        rect.bottom() - cell_height,
                    ),
                    egui::vec2(cell_width, cell_height),
                ),
                param,
                label,
            ));
        }
        (field, controls)
    };

    let mode_height = field.height().min(19.0);
    let mode = egui::Rect::from_min_size(field.min, egui::vec2(field.width(), mode_height));
    let mode_gap = 2.0_f32.min(mode.width() * 0.05);
    let jitter_mode = egui::Rect::from_min_max(
        mode.left_top(),
        egui::pos2(mode.center().x - mode_gap * 0.5, mode.bottom()),
    );
    let alignment_mode = egui::Rect::from_min_max(
        egui::pos2(mode.center().x + mode_gap * 0.5, mode.top()),
        mode.right_bottom(),
    );
    let square = egui::Rect::from_min_max(
        egui::pos2(field.left(), mode.bottom() + 2.0),
        field.right_bottom(),
    );
    with_child(
        ui,
        mode,
        ("jitter-mode", oscillator),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            enum_cycle_field(
                ui,
                state,
                params.jitter_mode,
                "JITTER",
                &["NOISE", "SINE"],
                jitter_mode.width(),
                jitter_mode.height(),
            );
        },
    );
    with_child(
        ui,
        alignment_mode,
        ("alignment-mode", oscillator),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            enum_cycle_field(
                ui,
                state,
                params.alignment_mode,
                "MODE",
                &["NOTE", "HARM", "ODD", "EVEN"],
                alignment_mode.width(),
                alignment_mode.height(),
            );
        },
    );
    with_child(
        ui,
        square,
        ("stereo-square", oscillator),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            if let Some(shaper) =
                stereo_square_view(ui, state, square.width(), square.height(), params)
            {
                ui.data_mut(|data| {
                    data.insert_temp(egui::Id::new(("osc-unison-mode", oscillator)), shaper);
                });
                editor_theme::request_display_repaint(ui);
            }
        },
    );
    for (index, (control, param, label)) in controls.into_iter().enumerate() {
        with_child(
            ui,
            control,
            ("stereo-control", oscillator, index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                param_field_sized(
                    ui,
                    state,
                    param,
                    compact_stereo_label(label, control.width(), control.height()),
                    control.width(),
                    control.height(),
                );
            },
        );
    }
}

fn compact_stereo_label(label: &'static str, width: f32, height: f32) -> &'static str {
    if width >= 38.0 && height >= 27.0 {
        return label;
    }
    match label {
        "VOICES" => "VCS",
        "JITTER" => "JIT",
        "RANGE" => "RNG",
        "HARM" => "HRM",
        "PHASE" => "PHS",
        "RATE" => "RTE",
        "WIDTH" => "WID",
        _ => label,
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
