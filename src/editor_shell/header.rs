use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_history::EditorHistory;
use crate::editor_presets::PresetEntry;
use crate::editor_widgets::{icon_font_ready, paint_vertical_label, with_child};
use crate::generators::{ModuleKind, OscillatorEngineKind};
use crate::oscillators::{
    Antialiasing, PhaseWarpMode, sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::WaveCurveRt;
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
    let section_gap = editor_theme::space::XS;
    let left_width = (width * 0.24).clamp(unit * 8.0, width * 0.27);
    let right_width = (width * 0.28).clamp(unit * 11.0, width * 0.31);
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(left_width, rect.height()));
    let right =
        egui::Rect::from_min_max(egui::pos2(rect.right() - right_width, rect.top()), rect.max);
    let center = egui::Rect::from_min_max(
        egui::pos2(left.right() + section_gap, rect.top()),
        egui::pos2(right.left() - section_gap, rect.bottom()),
    );

    let palette = editor_theme::semantic();
    let rail_width = (left.width() * 0.13).clamp(unit * 1.35, unit * 1.9);
    let rail = egui::Rect::from_min_size(left.min, egui::vec2(rail_width, left.height()));
    ui.painter().rect_filled(rail, 0.0, palette.masthead_ink);
    let settings_size = (unit * 1.32).min(rail.width() * 0.72);
    let settings_rect = egui::Rect::from_center_size(
        egui::pos2(rail.center().x, rail.bottom() - settings_size * 0.58),
        egui::Vec2::splat(settings_size),
    );
    let mark_center_y = settings_rect.top() - rail.height() * 0.09;
    let mark_size = egui::vec2(rail.width() * 0.62, rail.width() * 0.39);
    let mark_rect = egui::Rect::from_center_size(
        egui::pos2(rail.center().x, mark_center_y - mark_size.y * 0.5),
        mark_size,
    );
    let version = egui::Rect::from_min_max(rail.min, egui::pos2(rail.right(), mark_rect.top()));
    paint_vertical_label(
        ui,
        version,
        concat!("V", env!("CARGO_PKG_VERSION")),
        editor_theme::font::caption(),
        palette.masthead,
    );
    paint_matari_mark(ui, mark_rect, palette.masthead);

    with_child(
        ui,
        settings_rect,
        "header-settings",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let response = ui.add_sized(
                settings_rect.size(),
                egui::Button::new(
                    egui::RichText::new(if icon_font_ready(ui) {
                        egui_phosphor::regular::GEAR
                    } else {
                        ""
                    })
                    .color(palette.masthead),
                )
                .fill(palette.masthead_ink)
                .stroke(egui::Stroke::NONE)
                .selected(*settings_open),
            );
            if response
                .on_hover_text("Settings, theme, and generator quality")
                .clicked()
            {
                *settings_open = !*settings_open;
            }
        },
    );

    let brand = egui::Rect::from_min_max(
        egui::pos2(rail.right() + editor_theme::space::SM, left.top()),
        left.max,
    );
    let reset = ui
        .interact(
            brand,
            egui::Id::new("kurv-brand-reset"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Double-click for factory default");
    let brand_font = fit_font_to_width(
        ui.painter(),
        "KURV",
        egui::FontId::proportional(brand.height() * 0.76),
        brand.width() - unit * 2.0,
    );
    ui.painter().text(
        brand.left_center(),
        egui::Align2::LEFT_CENTER,
        "KURV",
        brand_font,
        palette.masthead_ink,
    );
    if reset.double_clicked() {
        editor::reset_to_defaults(state);
        history.commit(state);
        presets.selected = "Init".to_owned();
        presets.dirty = false;
        presets.error = None;
    }
    let toolbar = center.shrink2(egui::vec2(
        editor_theme::space::XS,
        editor_theme::space::XXS,
    ));
    with_child(
        ui,
        toolbar,
        "header-presets",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| draw_preset_deck(ui, state, history, presets, toolbar),
    );

    with_child(
        ui,
        right.shrink2(egui::vec2(editor_theme::space::SM, editor_theme::space::XS)),
        "header-performance",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing =
                egui::vec2(editor_theme::compact_gap(ui), editor_theme::compact_gap(ui));
            ui.spacing_mut().button_padding =
                egui::vec2(editor_theme::space::XS, editor_theme::space::XXS);
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.active.weak_bg_fill = palette.control;
            crate::editor_performance::performance_view(
                ui,
                state,
                ui.available_width(),
                ui.available_height(),
            );
        },
    );
}

fn draw_preset_deck(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    history: &mut EditorHistory,
    presets: &mut PresetUi,
    rect: egui::Rect,
) {
    let palette = editor_theme::semantic();
    let deck = rect.shrink2(egui::vec2(0.0, editor_theme::space::XXS));
    let bank_width = (deck.height() * 0.34).clamp(
        editor_theme::title_height(ui) * 0.88,
        editor_theme::title_height(ui) * 1.28,
    );
    let left_bank = egui::Rect::from_min_size(deck.min, egui::vec2(bank_width, deck.height()));
    let right_bank =
        egui::Rect::from_min_max(egui::pos2(deck.right() - bank_width, deck.top()), deck.max);
    let scope = egui::Rect::from_min_max(
        egui::pos2(left_bank.right(), deck.top()),
        egui::pos2(right_bank.left(), deck.bottom()),
    );
    ui.painter().rect_filled(
        deck,
        editor_theme::shape::CONTROL_RADIUS,
        palette.masthead_ink,
    );
    ui.painter().rect_stroke(
        deck,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.masthead.gamma_multiply(0.34),
        ),
        egui::StrokeKind::Inside,
    );
    for divider in [left_bank.right(), right_bank.left()] {
        ui.painter().line_segment(
            [
                egui::pos2(divider, deck.top() + editor_theme::space::XS),
                egui::pos2(divider, deck.bottom() - editor_theme::space::XS),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                palette.masthead.gamma_multiply(0.28),
            ),
        );
    }
    paint_patch_scope(ui, state, scope.shrink(editor_theme::space::XXS));
    presets.ensure_store();
    let selected_name = presets.selected.clone();
    let entries = presets.store.as_mut().map_or_else(
        || Ok(Vec::new()),
        |store| store.entries().map(<[PresetEntry]>::to_vec),
    );
    let entries = match entries {
        Ok(entries) => entries,
        Err(error) => {
            presets.error = Some(error.to_string());
            Vec::new()
        }
    };
    let current = entries
        .iter()
        .position(|entry| entry.name() == selected_name)
        .unwrap_or(0);
    let bank_cell_height = deck.height() / 3.0;
    let left_cell = |index: usize| {
        egui::Rect::from_min_max(
            egui::pos2(
                left_bank.left(),
                left_bank.top() + bank_cell_height * index as f32,
            ),
            egui::pos2(
                left_bank.right(),
                left_bank.top() + bank_cell_height * (index + 1) as f32,
            ),
        )
    };
    let right_cell = |index: usize| {
        egui::Rect::from_min_max(
            egui::pos2(
                right_bank.left(),
                right_bank.top() + bank_cell_height * index as f32,
            ),
            egui::pos2(
                right_bank.right(),
                right_bank.top() + bank_cell_height * (index + 1) as f32,
            ),
        )
    };
    let mut chosen_index = None;
    if deck_icon_button(
        ui,
        left_cell(0),
        !entries.is_empty(),
        egui_phosphor::regular::CARET_LEFT,
        palette.masthead,
        "Previous preset",
    )
    .clicked()
    {
        chosen_index = Some(
            current
                .checked_sub(1)
                .unwrap_or(entries.len().saturating_sub(1)),
        );
    }
    if deck_icon_button(
        ui,
        right_cell(0),
        !entries.is_empty(),
        egui_phosphor::regular::CARET_RIGHT,
        palette.masthead,
        "Next preset",
    )
    .clicked()
    {
        chosen_index = Some((current + 1) % entries.len().max(1));
    }
    let selected = if presets.selected.is_empty() {
        "Init".to_owned()
    } else if presets.dirty {
        format!("{} *", presets.selected)
    } else {
        presets.selected.clone()
    };
    let preset_font = editor_theme::font::label();
    let preset_width = ui
        .painter()
        .layout_no_wrap(selected.clone(), preset_font.clone(), palette.text)
        .size()
        .x;
    let picker = egui::Rect::from_min_size(
        scope.min + egui::vec2(editor_theme::space::SM, editor_theme::space::XS),
        egui::vec2(
            (preset_width + editor_theme::title_height(ui) * 1.65)
                .clamp(editor_theme::title_height(ui) * 3.4, scope.width() * 0.30),
            editor_theme::title_height(ui),
        ),
    );
    ui.painter().rect_filled(
        picker,
        editor_theme::shape::CONTROL_RADIUS,
        palette.masthead_ink.gamma_multiply(0.96),
    );
    with_child(
        ui,
        picker,
        "header-preset-picker",
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for visuals in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                visuals.bg_fill = egui::Color32::TRANSPARENT;
                visuals.weak_bg_fill = egui::Color32::TRANSPARENT;
                visuals.bg_stroke = egui::Stroke::NONE;
            }
            egui::ComboBox::from_id_salt("kurv-preset-picker")
                .selected_text(
                    egui::RichText::new(selected)
                        .font(preset_font)
                        .color(palette.text),
                )
                .width(picker.width())
                .show_ui(ui, |ui| {
                    for (index, entry) in entries.iter().enumerate() {
                        if ui
                            .selectable_label(selected_name == entry.name(), entry.name())
                            .clicked()
                        {
                            chosen_index = Some(index);
                        }
                    }
                });
        },
    );
    if deck_icon_button(
        ui,
        left_cell(1),
        history.can_undo(),
        egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE,
        palette.masthead,
        "Undo — Ctrl/Cmd+Z",
    )
    .clicked()
    {
        presets.dirty |= history.undo(state);
    }
    if deck_icon_button(
        ui,
        left_cell(2),
        history.can_redo(),
        egui_phosphor::regular::ARROW_CLOCKWISE,
        palette.masthead,
        "Redo — Ctrl/Cmd+Shift+Z",
    )
    .clicked()
    {
        presets.dirty |= history.redo(state);
    }
    if deck_icon_button(
        ui,
        right_cell(2),
        true,
        egui_phosphor::regular::FLOPPY_DISK,
        palette.masthead,
        "Save as a named preset",
    )
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
    if deck_icon_button(
        ui,
        right_cell(1),
        presets.dirty,
        egui_phosphor::regular::FLOPPY_DISK_BACK,
        palette.masthead,
        "Save changes to this preset",
    )
    .clicked()
    {
        presets.save_open = true;
        presets.save_focus = true;
        presets.save_name = if presets.selected.is_empty() || presets.selected == "Init" {
            "New Preset".to_owned()
        } else {
            presets.selected.clone()
        };
    }
    if let Some(entry) = chosen_index.and_then(|index| entries.get(index)).cloned()
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
    if let Some(error) = presets.error.as_deref() {
        let error_rect = egui::Rect::from_min_size(
            scope.right_top()
                + egui::vec2(
                    -editor_theme::title_height(ui) * 5.4,
                    editor_theme::space::XS,
                ),
            egui::vec2(
                editor_theme::title_height(ui) * 5.0,
                editor_theme::title_height(ui),
            ),
        );
        ui.painter().text(
            error_rect.right_center(),
            egui::Align2::RIGHT_CENTER,
            "PRESET ERROR",
            editor_theme::font::caption(),
            palette.danger,
        );
        ui.interact(
            error_rect,
            egui::Id::new("preset-error"),
            egui::Sense::hover(),
        )
        .on_hover_text(error);
    }
}

fn deck_icon_button(
    ui: &egui::Ui,
    rect: egui::Rect,
    enabled: bool,
    icon: &'static str,
    color: egui::Color32,
    tooltip: &str,
) -> egui::Response {
    let response = ui
        .interact(
            rect,
            egui::Id::new(("preset-deck-icon", tooltip)),
            if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_text(tooltip);
    let palette = editor_theme::semantic();
    if response.hovered() && enabled {
        ui.painter().rect_filled(
            rect,
            editor_theme::shape::CONTROL_RADIUS,
            palette.control_hover,
        );
    }
    if icon_font_ready(ui) {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            editor_theme::font::title(),
            if enabled {
                color
            } else {
                palette.masthead.gamma_multiply(0.24)
            },
        );
    }
    response
}

fn paint_patch_scope(ui: &egui::Ui, state: &PluginContext<KurvParams>, rect: egui::Rect) {
    let slot = state
        .generator_stack
        .snapshot()
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .find_map(|module| match module.kind() {
            ModuleKind::Oscillator(slot) => Some(slot),
            ModuleKind::Filter(_) => None,
        });
    let Some(slot) = slot else { return };
    let config = state.generator_stack.oscillator_config(slot);
    if config.engine != OscillatorEngineKind::Va {
        return;
    }
    let selection = state
        .generator_stack
        .va_table(slot)
        .snapshot()
        .compile_rt()
        .select(
            WaveCurveRt::default(),
            config.custom_shape,
            (config.shape / 3.0).clamp(0.0, 1.0),
        );
    let plot = rect.shrink2(egui::vec2(rect.width() * 0.04, rect.height() * 0.10));
    let points = (0..=96)
        .map(|index| {
            let phase = index as f64 / 96.0;
            let sample = sample_custom_shape_with_antialiasing_warped(
                selection.shape,
                phase,
                0.0,
                config.pulse_width,
                Antialiasing::SplineOptimized,
                PhaseWarpMode::from_index(config.phase_warp_mode),
                config.phase_warp_amount,
                selection.curve,
                selection.mix,
            ) * config.level;
            egui::pos2(
                egui::lerp(plot.x_range(), index as f32 / 96.0),
                egui::lerp(plot.y_range(), (1.0 - sample) * 0.5),
            )
        })
        .collect::<Vec<_>>();
    let color = editor_theme::semantic().pan_shape;
    let mut fill = egui::Mesh::default();
    let edge = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 48);
    let clear = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    for pair in points.windows(2) {
        let base = fill.vertices.len() as u32;
        fill.colored_vertex(pair[0], edge);
        fill.colored_vertex(pair[1], edge);
        fill.colored_vertex(egui::pos2(pair[1].x, plot.center().y), clear);
        fill.colored_vertex(egui::pos2(pair[0].x, plot.center().y), clear);
        fill.add_triangle(base, base + 1, base + 2);
        fill.add_triangle(base, base + 2, base + 3);
    }
    ui.painter().add(fill);
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::GROUP_STROKE, color),
    ));
}

fn paint_matari_mark(ui: &egui::Ui, rect: egui::Rect, tint: egui::Color32) {
    if !icon_font_ready(ui) {
        return;
    }
    let cache_id = egui::Id::new("kurv-matari-mark");
    let texture = ui.data(|data| data.get_temp::<egui::TextureHandle>(cache_id));
    let texture = texture.unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [96, 60],
            include_bytes!("../../assets/matari-mark-96x60.rgba"),
        );
        let texture = ui
            .ctx()
            .load_texture("matari-mark", image, egui::TextureOptions::LINEAR);
        ui.data_mut(|data| data.insert_temp(cache_id, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        tint,
    );
}
