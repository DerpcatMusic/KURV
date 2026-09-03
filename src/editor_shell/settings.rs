use std::{sync::Arc, time::Duration};

use truce_core::editor::PluginContext;

use crate::editor_oscillator::{
    antialiasing_selector_compact, quality_selector_compact, resynth_quality_selector_compact,
};
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

#[derive(Clone, Default)]
pub(super) struct LicenseUi {
    status: Option<String>,
    repaint_listener: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsPage {
    #[default]
    Interface,
    Workspace,
    Quality,
    License,
}

impl SettingsPage {
    const ALL: [Self; 4] = [
        Self::Interface,
        Self::Workspace,
        Self::Quality,
        Self::License,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Interface => "INTERFACE",
            Self::Workspace => "WORKSPACE",
            Self::Quality => "QUALITY",
            Self::License => "LICENSE",
        }
    }
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

impl LicenseUi {
    fn ensure_repaint_listener(
        &mut self,
        activation: &crate::licensing::PluginActivation,
        ctx: &egui::Context,
    ) {
        if self.repaint_listener.is_none() {
            let repaint = ctx.clone();
            let listener: Arc<dyn Fn() + Send + Sync> = Arc::new(move || repaint.request_repaint());
            activation.subscribe_changes(Arc::clone(&listener));
            self.repaint_listener = Some(listener);
        }
    }
}

pub(super) use theme_state::read_persisted_theme;

pub(super) fn apply_active_theme(
    ui: &mut egui::Ui,
    themes: &mut ThemeUi,
    persisted: editor_theme::ThemeSettings,
) {
    themes.ensure_library(persisted);
    editor_theme::apply_with(ui, persisted);
}

pub(super) fn apply_ui_zoom(ui: &egui::Ui, state: &PluginContext<KurvParams>) {
    // Truce's Editor::set_scale_factor is host content DPI. This setting is
    // only the user's egui zoom and must not replace that host callback.
    let scale = state
        .params()
        .editor_state
        .lock()
        .map_or(100, |editor| editor.ui_scale);
    ui.ctx().set_zoom_factor(ui_zoom_factor(scale));
}

fn ui_zoom_factor(scale: u8) -> f32 {
    match scale {
        125 => 1.25,
        150 => 1.5,
        _ => 1.0,
    }
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    workspace: egui::Rect,
    open: &mut bool,
    themes: &mut ThemeUi,
    license: &mut LicenseUi,
    page: &mut SettingsPage,
    popup_was_open: bool,
) {
    license.ensure_repaint_listener(&state.params().activation, ui.ctx());
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
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = editor_theme::space::XS;
                        for candidate in SettingsPage::ALL {
                            ui.selectable_value(page, candidate, candidate.label());
                        }
                    });
                    ui.separator();
                    match *page {
                        SettingsPage::Interface => draw_interface(ui, state, themes),
                        SettingsPage::Workspace => draw_workspace(ui, state),
                        SettingsPage::Quality => draw_quality(ui, state),
                        SettingsPage::License => draw_license(ui, state, license),
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
        *open = false;
    }
}

fn section_label(ui: &mut egui::Ui, label: &'static str) {
    ui.label(
        egui::RichText::new(label)
            .font(editor_theme::font::caption())
            .color(editor_theme::semantic().text_muted),
    );
}

fn draw_interface(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, themes: &mut ThemeUi) {
    let mut editor = state
        .params()
        .editor_state
        .lock()
        .map_or_else(|_| crate::KurvEditorState::default(), |value| value.clone());

    section_label(ui, "THEME");
    if let Some(library) = themes.library.as_mut() {
        let names = library.names();
        let active = library.active_name().to_owned();
        let mut selected = None;
        ui.horizontal(|ui| {
            ui.label("PRESET");
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
                    .hint_text("New theme")
                    .desired_width(ui.available_width() * 0.55),
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
            egui::RichText::new("AUTO-SAVED")
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

    let mut changed = false;
    egui::Grid::new("theme-colors")
        .num_columns(2)
        .spacing(egui::vec2(editor_theme::space::MD, editor_theme::space::XS))
        .show(ui, |ui| {
            for (label, rgb) in [
                (
                    "BACKGROUND",
                    &mut [
                        editor.background_red,
                        editor.background_green,
                        editor.background_blue,
                    ],
                ),
                (
                    "PRIMARY",
                    &mut [
                        editor.primary_red,
                        editor.primary_green,
                        editor.primary_blue,
                    ],
                ),
                (
                    "SECONDARY",
                    &mut [
                        editor.secondary_red,
                        editor.secondary_green,
                        editor.secondary_blue,
                    ],
                ),
                (
                    "TERTIARY",
                    &mut [
                        editor.tertiary_red,
                        editor.tertiary_green,
                        editor.tertiary_blue,
                    ],
                ),
            ] {
                ui.label(label);
                changed |= ui.color_edit_button_srgb(rgb).changed();
                match label {
                    "BACKGROUND" => {
                        [
                            editor.background_red,
                            editor.background_green,
                            editor.background_blue,
                        ] = *rgb;
                    }
                    "PRIMARY" => {
                        [
                            editor.primary_red,
                            editor.primary_green,
                            editor.primary_blue,
                        ] = *rgb;
                    }
                    "SECONDARY" => {
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
    let tint_changed = ui
        .add(egui::Slider::new(&mut tint, 0.0..=100.0).text("TINT"))
        .changed();
    let contrast_changed = ui
        .add(egui::Slider::new(&mut contrast, 50.0..=175.0).text("CONTRAST"))
        .changed();
    if ui.input(|input| input.modifiers.ctrl) {
        if tint_changed {
            tint = crate::editor_controls::semantic_snap(
                tint,
                crate::editor_controls::ValueSemantic::IntegerTens,
                true,
            );
        }
        if contrast_changed {
            contrast = crate::editor_controls::semantic_snap(
                contrast,
                crate::editor_controls::ValueSemantic::IntegerTens,
                true,
            );
        }
    }
    changed |= tint_changed || contrast_changed;
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
            *stored = editor.clone();
        }
        themes.changed(settings, ui.input(|input| input.time), ui.ctx());
        editor_theme::request_display_repaint(ui);
    }

    section_label(ui, "UI SCALE");
    let mut scale = match editor.ui_scale {
        125 | 150 => editor.ui_scale,
        _ => 100,
    };
    let mut scale_changed = false;
    ui.horizontal(|ui| {
        for percent in [100, 125, 150] {
            scale_changed |= ui
                .selectable_value(&mut scale, percent, format!("{percent}%"))
                .changed();
        }
    });
    if scale_changed {
        if let Ok(mut stored) = state.params().editor_state.lock() {
            stored.ui_scale = scale;
        }
        editor_theme::request_display_repaint(ui);
    }
    ui.label(
        egui::RichText::new("Host DPI stays unchanged")
            .font(editor_theme::font::caption())
            .color(editor_theme::semantic().text_muted),
    );
}

fn draw_workspace(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    section_label(ui, "LAYOUT");
    let mut responsive_density = state
        .params()
        .editor_state
        .lock()
        .map_or(100.0, |editor| f32::from(editor.responsive_density));
    if ui
        .add(egui::Slider::new(&mut responsive_density, 0.0..=100.0).text("DENSITY"))
        .on_hover_text("How strongly the split changes card height")
        .changed()
    {
        if ui.input(|input| input.modifiers.ctrl) {
            responsive_density = crate::editor_controls::semantic_snap(
                responsive_density,
                crate::editor_controls::ValueSemantic::IntegerTens,
                true,
            );
        }
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor.responsive_density = responsive_density.round() as u8;
        }
        editor_theme::request_display_repaint(ui);
    }

    section_label(ui, "MODULATION");
    let mut persistent_cables = state
        .params()
        .editor_state
        .lock()
        .is_ok_and(|editor| editor.persistent_modulation_cables);
    if ui
        .checkbox(&mut persistent_cables, "SHOW MODULATION CABLES")
        .on_hover_text("Keep cables visible for every visible route")
        .changed()
    {
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor.persistent_modulation_cables = persistent_cables;
        }
        editor_theme::request_display_repaint(ui);
    }
}

fn draw_quality(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    section_label(ui, "GENERATOR");
    let selector_gap = editor_theme::space::XS;
    let selector_width =
        ((ui.available_width() - selector_gap) * 0.5).max(editor_theme::title_height(ui) * 4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = selector_gap;
        antialiasing_selector_compact(ui, state, selector_width);
        quality_selector_compact(ui, state, selector_width);
    });

    section_label(ui, "RESYNTH");
    resynth_quality_selector_compact(ui, state, selector_width);

    section_label(ui, "CPU");
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !performance::calibration_running(),
                egui::Button::new("CALIBRATE"),
            )
            .on_hover_text("Benchmark spline kernels off the audio thread")
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

    let mut fast_modulation = state.params().fast_audio_rate_modulation();
    ui.horizontal(|ui| {
        ui.label("AUDIO-RATE");
        egui::ComboBox::from_id_salt("audio-rate-modulation-quality")
            .selected_text(if fast_modulation { "FAST" } else { "EXACT" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut fast_modulation, false, "EXACT");
                ui.selectable_value(&mut fast_modulation, true, "FAST");
            });
    })
    .response
    .on_hover_text("FAST lowers CPU; EXACT preserves the highest-quality renderer");
    if fast_modulation != state.params().fast_audio_rate_modulation() {
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor.fast_audio_rate_modulation = fast_modulation;
        }
        state
            .params()
            .set_fast_audio_rate_modulation(fast_modulation);
    }
    if performance::calibration_running() {
        editor_theme::request_display_repaint(ui);
    }
}

fn draw_license(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, license: &mut LicenseUi) {
    use crate::licensing::LicenseState;

    section_label(ui, "ACTIVATION");
    let activation = &state.params().activation;
    let access = activation.license_state();
    let status = if activation.access_check_pending() {
        "Checking open beta access…".to_owned()
    } else {
        match access {
            LicenseState::Activated => "Activated".to_owned(),
            LicenseState::Trial { seconds_left, .. } => {
                format!("Open beta · {} days left", seconds_left.div_ceil(86_400))
            }
            LicenseState::Expired { reason } => reason.title().to_owned(),
            _ => "Activation required".to_owned(),
        }
    };
    ui.label(
        egui::RichText::new(status)
            .font(editor_theme::font::label())
            .color(if access.is_usable() {
                editor_theme::semantic().primary
            } else {
                editor_theme::semantic().danger
            }),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("COPY REQUEST").clicked() {
            match activation.activation_request() {
                Ok(request) => {
                    ui.ctx().copy_text(request);
                    license.status = Some("Activation request copied".to_owned());
                }
                Err(error) => license.status = Some(error),
            }
        }
        if ui.button("ACTIVATE").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(
                "https://matari-audio.com/activate?productId=kurv",
            ));
        }
        if ui.button("LOAD LICENSE").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Load KURV License")
                .add_filter("KURV activation license", &["dgmlic"])
                .pick_file()
        {
            license.status = Some(
                activation
                    .import_license_file(&path)
                    .unwrap_or_else(|error| error),
            );
        }
        if !access.is_usable()
            && !activation.access_check_pending()
            && ui.button("RETRY BETA").clicked()
        {
            license.status = Some(
                activation
                    .start_trial()
                    .map_or_else(|error| error, |()| "Checking beta access…".to_owned()),
            );
        }
    });
    if let Some(status) = license.status.as_deref() {
        ui.label(
            egui::RichText::new(status)
                .font(editor_theme::font::caption())
                .color(editor_theme::semantic().text_muted),
        );
    }
}
