use std::time::Duration;

use truce_core::editor::PluginContext;

use crate::editor_history::EditorHistory;
use crate::editor_oscillator::{antialiasing_selector_compact, quality_selector_compact};
use crate::editor_presets::PresetStore;
use crate::editor_widgets::{icon_font_ready, with_child};
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
    crate::editor_generator::show(ui, state, left, gap, section_gap);

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
    let palette = editor_theme::semantic();
    ui.painter().rect(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        palette.surface,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.72),
        ),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(editor_theme::space::XS);
    let heading_height = editor_theme::title_height(ui) + editor_theme::compact_gap(ui);
    let heading = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(
            inner.right(),
            (inner.top() + heading_height).min(inner.bottom()),
        ),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(inner.left(), heading.bottom()),
        inner.right_bottom(),
    );
    ui.painter().rect_filled(
        body,
        editor_theme::shape::CONTROL_RADIUS,
        palette.well.gamma_multiply(0.92),
    );
    ui.painter().line_segment(
        [heading.left_bottom(), heading.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.42),
        ),
    );

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
            crate::editor::performance_view(ui, state, inner.width(), inner.height());
        },
    );
}
