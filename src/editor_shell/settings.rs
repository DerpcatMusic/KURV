use std::time::Duration;

use truce_core::editor::PluginContext;

use crate::editor_oscillator::{antialiasing_selector_compact, quality_selector_compact};
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::{KurvParams, editor_theme, performance};

mod theme_state;

use theme_state::{read_editor_theme, write_editor_theme};

#[derive(Clone, Default)]
pub(super) struct ThemeUi {
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

    pub(super) fn flush(&mut self, now: f64, force: bool) {
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

pub(super) use theme_state::read_persisted_theme;

pub(super) fn apply_active_theme(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    themes: &mut ThemeUi,
    persisted: editor_theme::ThemeSettings,
) {
    let theme = themes.active_settings(persisted);
    if theme != persisted {
        write_editor_theme(state, theme);
    }
    editor_theme::apply_with(ui, theme);
}

pub(super) fn draw(
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
                        egui::RichText::new("MODULATION DISPLAY")
                            .font(editor_theme::font::caption())
                            .color(editor_theme::semantic().text_muted),
                    );
                    let mut persistent_cables = state
                        .params()
                        .editor_state
                        .lock()
                        .is_ok_and(|editor| editor.persistent_modulation_cables);
                    if ui
                        .checkbox(&mut persistent_cables, "Keep modulation cables visible")
                        .on_hover_text(
                            "Draw VCV/Cardinal-style cables for every visible route; source dragging and depth handles stay unchanged",
                        )
                        .changed()
                    {
                        if let Ok(mut editor) = state.params().editor_state.lock() {
                            editor.persistent_modulation_cables = persistent_cables;
                        }
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
