use truce::params::FloatParamReadF32;
use truce_core::editor::PluginContext;

use crate::editor_controls::layout_metric_text;
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::{KurvParams, editor_theme};

use super::super::{config_scalar_drag, format_pan};

mod unison;

pub(super) use unison::draw_unison_readouts;

struct OscillatorReadoutRects {
    level: egui::Rect,
    semi: egui::Rect,
    cent: egui::Rect,
    pitch: egui::Rect,
    pan: egui::Rect,
    phase: egui::Rect,
}

fn oscillator_readout_rects(rect: egui::Rect, include_pan: bool) -> OscillatorReadoutRects {
    let cell_width = rect.width() / 4.0;
    let cells: [egui::Rect; 4] = std::array::from_fn(|index| {
        let left = rect.left() + cell_width * index as f32;
        let right = if index == 3 {
            rect.right()
        } else {
            left + cell_width
        };
        egui::Rect::from_x_y_ranges(left..=right, rect.y_range())
    });
    let pitch = cells[1];
    OscillatorReadoutRects {
        level: cells[0],
        semi: left_half(pitch),
        cent: right_half(pitch),
        pitch,
        pan: cells[2],
        phase: if include_pan {
            cells[3]
        } else {
            cells[2].union(cells[3])
        },
    }
}

fn left_half(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(rect.min, egui::pos2(rect.center().x, rect.bottom()))
}

fn right_half(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(rect.center().x, rect.top()), rect.max)
}

fn metric_readout_colors(
    ui: &egui::Ui,
    accent: egui::Color32,
    hovered: bool,
    active: bool,
) -> (egui::Color32, egui::Color32) {
    if !ui.is_enabled() {
        let disabled = editor_theme::semantic().disabled_text;
        return (disabled, disabled);
    }
    if active {
        return (accent, accent);
    }
    if hovered {
        return (accent.gamma_multiply(0.88), accent);
    }
    (accent.gamma_multiply(0.64), accent.gamma_multiply(0.88))
}

pub(super) fn paint_tinted_metric_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    accent: egui::Color32,
    hovered: bool,
    active: bool,
) {
    let painter = ui.painter_at(rect);
    let layout = layout_metric_text(ui, &painter, rect, label, value);
    let (label_color, value_color) = metric_readout_colors(ui, accent, hovered, active);
    painter.galley(layout.label_position, layout.label, label_color);
    painter.galley(layout.value_position, layout.value, value_color);
}

fn paint_phaseplant_phase_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    position: f32,
    random: f32,
    position_hovered: bool,
    random_hovered: bool,
    position_active: bool,
    random_active: bool,
    accent: egui::Color32,
) {
    let painter = ui.painter_at(rect);
    let position_text = format!("{position:.0}°");
    let random_text = format!("±{random:.0}°");
    let value_text = format!("{position_text} {random_text}");
    let layout = layout_metric_text(ui, &painter, rect, label, &value_text);
    let value_font = layout.value_font;
    let position_width = painter
        .layout_no_wrap(
            position_text.clone(),
            value_font.clone(),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    let separator = painter
        .layout_no_wrap(" ".into(), value_font.clone(), egui::Color32::WHITE)
        .size()
        .x;
    let value_left = layout.value_position.x;
    let (_, position_value_color) =
        metric_readout_colors(ui, accent, position_hovered, position_active);
    let (_, random_value_color) = metric_readout_colors(ui, accent, random_hovered, random_active);
    let (label_color, _) = metric_readout_colors(
        ui,
        accent,
        position_hovered || random_hovered,
        position_active || random_active,
    );
    painter.galley(layout.label_position, layout.label, label_color);
    painter.text(
        egui::pos2(value_left, layout.value_position.y),
        egui::Align2::LEFT_TOP,
        position_text,
        value_font.clone(),
        position_value_color,
    );
    painter.text(
        egui::pos2(
            value_left + position_width + separator,
            layout.value_position.y,
        ),
        egui::Align2::LEFT_TOP,
        random_text,
        value_font,
        random_value_color,
    );
}

pub(super) fn draw_oscillator_readouts(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    oscillator_readouts: egui::Rect,
    include_pan: bool,
) -> bool {
    let index = slot.index();
    let audio_rate_drag = crate::editor_modulation::generator_source_drag_active(ui);
    let level_label = if audio_rate_drag { "AM" } else { "LEVEL" };
    let pitch_label = if audio_rate_drag {
        "EXP FM"
    } else {
        tuning_mode_label(config.tuning_mode)
    };
    let pan_label = "PAN";
    let phase_label = if audio_rate_drag { "PM" } else { "PHASE" };
    let readouts = oscillator_readout_rects(oscillator_readouts, include_pan);
    let mut config_changed = false;
    let mut readout_active = [false; 7];
    let mut readout_hovered = [false; 7];
    let mut level = format!("{:.0} %", config.level * 100.0);
    let mut semi = format!("{:+.0}", config.transpose);
    let mut cents = format!("{:+.0}c", config.cents);
    let mut pitch = format_mode_pitch(config);
    let mut pan = format_pan(config.pan);
    let phase_position = format!("{:.0}°", config.phase_position * 360.0);
    let phase_random = format!("±{:.0}°", config.phase_random * 360.0);
    let pitch_primary = if config.tuning_mode == crate::generators::OscillatorTuningMode::Semicent {
        (
            readouts.semi,
            ConfigField::Semi,
            1,
            pitch_label,
            semi.as_str(),
        )
    } else {
        (
            readouts.pitch,
            ConfigField::Pitch,
            1,
            pitch_label,
            pitch.as_str(),
        )
    };
    let pitch_secondary = (config.tuning_mode == crate::generators::OscillatorTuningMode::Semicent)
        .then_some((readouts.cent, ConfigField::Fine, 2, "", cents.as_str()));
    let hits = [
        Some((
            if audio_rate_drag {
                left_half(readouts.level)
            } else {
                readouts.level
            },
            ConfigField::Level,
            0,
            level_label,
            level.as_str(),
        )),
        Some((right_half(readouts.level), ConfigField::Ring, 6, "RM", "")),
        Some(pitch_primary),
        pitch_secondary,
        Some((readouts.pan, ConfigField::Pan, 3, pan_label, pan.as_str())),
        Some((
            left_half(readouts.phase),
            ConfigField::PhasePosition,
            4,
            phase_label,
            phase_position.as_str(),
        )),
        Some((
            right_half(readouts.phase),
            ConfigField::PhaseRandom,
            5,
            phase_label,
            phase_random.as_str(),
        )),
    ];
    for (cell_index, hit) in hits.into_iter().enumerate() {
        let Some((cell, field, readout_index, label, value_text)) = hit else {
            continue;
        };
        if matches!(field, ConfigField::Ring) && !audio_rate_drag {
            continue;
        }
        if !include_pan && matches!(field, ConfigField::Pan) {
            continue;
        }
        with_child(
            ui,
            cell,
            ("compact-config", index, cell_index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let (changed, active, hovered) = config_field_drag(
                    ui,
                    state,
                    module_id,
                    slot,
                    config,
                    field,
                    label,
                    value_text,
                    cell.size(),
                );
                config_changed |= changed;
                readout_active[readout_index] |= active;
                readout_hovered[readout_index] |= hovered;
            },
        );
    }
    if config_changed {
        level = format!("{:.0} %", config.level * 100.0);
        semi = format!("{:+.0}", config.transpose);
        cents = format!("{:+.0}c", config.cents);
        pitch = format_mode_pitch(config);
        pan = format_pan(config.pan);
    }
    let accent = if audio_rate_drag {
        editor_theme::semantic().pan_shape
    } else {
        editor_theme::semantic().primary
    };
    let level_rect = if audio_rate_drag {
        left_half(readouts.level)
    } else {
        readouts.level
    };
    for (index, (rect, label, value, active, hovered)) in [
        (
            level_rect,
            level_label,
            level.as_str(),
            readout_active[0],
            readout_hovered[0],
        ),
        (
            readouts.pan,
            pan_label,
            pan.as_str(),
            readout_active[3],
            readout_hovered[3],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if index == 1 && !include_pan {
            continue;
        }
        paint_tinted_metric_readout(ui, rect, label, value, accent, hovered, active);
    }
    if audio_rate_drag {
        paint_tinted_metric_readout(
            ui,
            right_half(readouts.level),
            "RM",
            "",
            accent,
            readout_hovered[6],
            readout_active[6],
        );
    }
    config_changed |= paint_pitch_readout(
        ui,
        readouts.pitch,
        pitch_label,
        &semi,
        &cents,
        &pitch,
        config,
        readout_hovered[1],
        readout_hovered[2],
        readout_active[1],
        readout_active[2],
        accent,
        audio_rate_drag,
        index,
    );
    paint_phaseplant_phase_readout(
        ui,
        readouts.phase,
        phase_label,
        config.phase_position * 360.0,
        config.phase_random * 360.0,
        readout_hovered[4],
        readout_hovered[5],
        readout_active[4],
        readout_active[5],
        accent,
    );
    config_changed
}

#[derive(Clone, Copy)]
enum ConfigField {
    Level,
    Ring,
    Pan,
    Fine,
    Semi,
    Pitch,
    PhasePosition,
    PhaseRandom,
}

fn config_field_drag(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    field: ConfigField,
    label: &str,
    value_text: &str,
    size: egui::Vec2,
) -> (bool, bool, bool) {
    let before = *config;
    let defaults = crate::generators::OscillatorConfig::default();
    let (changed, response) = match field {
        ConfigField::Level => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.level,
                0.0..=1.0,
                0.01,
                defaults.level,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
        ConfigField::Ring => {
            let (_, response) = ui.allocate_exact_size(size, egui::Sense::hover());
            (false, response)
        }
        ConfigField::Pan => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.pan,
                -1.0..=1.0,
                0.02,
                defaults.pan,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
        ConfigField::Fine => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.cents,
                -100.0..=100.0,
                0.5,
                defaults.cents,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
        ConfigField::Semi => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.transpose,
                -48.0..=48.0,
                0.125,
                defaults.transpose,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
        ConfigField::Pitch => pitch_field_drag(
            ui,
            config,
            label,
            value_text,
            size,
            state.params().global_tuning_hz.value(),
        ),
        ConfigField::PhasePosition => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.phase_position,
                0.0..=1.0,
                0.01,
                defaults.phase_position,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
        ConfigField::PhaseRandom => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.phase_random,
                0.0..=1.0,
                0.01,
                defaults.phase_random,
                label,
                value_text,
                size,
            );
            (changed, response)
        }
    };
    let (control, span, internal_modulation) = match field {
        ConfigField::Level => (OscillatorControl::Level, 1.0, true),
        ConfigField::Ring => (OscillatorControl::RingModAmount, 1.0, true),
        ConfigField::Pan => (OscillatorControl::Pan, 0.5, true),
        ConfigField::Fine => (OscillatorControl::Cents, 0.5, true),
        ConfigField::Semi => (OscillatorControl::Transpose, 0.5, true),
        ConfigField::Pitch => (OscillatorControl::Transpose, 0.5, true),
        ConfigField::PhasePosition => (OscillatorControl::PhasePosition, 1.0, true),
        ConfigField::PhaseRandom => (OscillatorControl::PhaseRandom, 1.0, false),
    };
    let mut changed = changed;
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    let legacy_cents_target = matches!(field, ConfigField::Pitch)
        .then(|| ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::Cents));
    if internal_modulation
        && (crate::editor_modulation::modular_owns_gesture(ui, state, target, &response)
            || legacy_cents_target.is_some_and(|target| {
                crate::editor_modulation::modular_owns_gesture(ui, state, target, &response)
            }))
    {
        *config = before;
        changed = false;
    }
    let base = control.normalized_value(*config);
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
        && !matches!(
            field,
            ConfigField::Pitch
                if config.tuning_mode != crate::generators::OscillatorTuningMode::Semicent
        )
    {
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
        if let Some(target) = legacy_cents_target {
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                0.5,
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                span,
            );
        }
    } else {
        crate::editor_modulation::host_automation_destination(ui, state, target, &response, base);
    }
    (
        changed,
        response.is_pointer_button_down_on() || response.dragged(),
        response.hovered(),
    )
}

fn tuning_mode_label(mode: crate::generators::OscillatorTuningMode) -> &'static str {
    match mode {
        crate::generators::OscillatorTuningMode::Semicent => "SEMI",
        crate::generators::OscillatorTuningMode::Hertz => "HZ",
        crate::generators::OscillatorTuningMode::Ratio => "RATIO",
    }
}

fn format_mode_pitch(config: &crate::generators::OscillatorConfig) -> String {
    match config.tuning_mode {
        crate::generators::OscillatorTuningMode::Semicent => String::new(),
        crate::generators::OscillatorTuningMode::Hertz => {
            format!("{:+.2} Hz", config.frequency_offset_hz)
        }
        crate::generators::OscillatorTuningMode::Ratio => {
            format!("{:.3}×", config.frequency_ratio)
        }
    }
}

fn pitch_field_drag(
    ui: &mut egui::Ui,
    config: &mut crate::generators::OscillatorConfig,
    label: &str,
    value_text: &str,
    size: egui::Vec2,
    reference_hz: f32,
) -> (bool, egui::Response) {
    let minimum = editor_theme::font::VALUE_SIZE + editor_theme::font::CAPTION_SIZE;
    let (id, rect) = ui.allocate_space(egui::vec2(size.x.max(minimum), size.y.max(minimum)));
    let layout = layout_metric_text(ui, &ui.painter_at(rect), rect, label, value_text);
    let value_rect = egui::Rect::from_min_size(layout.value_position, layout.value.size())
        .expand(editor_theme::space::XXS)
        .intersect(rect);
    let response = ui
        .interact(value_rect, id.with("pitch-value"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Drag vertically to change. Hold Shift for fine control or Ctrl for semantic snap; double-click to reset.",
        );
    let changed = match config.tuning_mode {
        crate::generators::OscillatorTuningMode::Semicent => false,
        crate::generators::OscillatorTuningMode::Hertz => {
            let fine = ui.input(|input| input.modifiers.shift && !input.modifiers.ctrl);
            crate::editor_controls::update_custom_value_drag(
                ui,
                &response,
                &mut config.frequency_offset_hz,
                -10_000.0..=10_000.0,
                1.0,
                0.0,
                if fine {
                    crate::editor_controls::ValueSemantic::Continuous
                } else {
                    crate::editor_controls::ValueSemantic::FrequencyOffset(reference_hz)
                },
            )
        }
        crate::generators::OscillatorTuningMode::Ratio => {
            crate::editor_controls::update_custom_pitch_ratio_drag(
                ui,
                &response,
                &mut config.frequency_ratio,
            )
        }
    };
    (changed, response)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the pitch painter keeps the two independent gesture states visible"
)]
fn paint_pitch_readout(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    semi: &str,
    cents: &str,
    mode_value: &str,
    config: &mut crate::generators::OscillatorConfig,
    semi_hovered: bool,
    cents_hovered: bool,
    semi_active: bool,
    cents_active: bool,
    accent: egui::Color32,
    audio_rate_drag: bool,
    oscillator: usize,
) -> bool {
    let semicent = config.tuning_mode == crate::generators::OscillatorTuningMode::Semicent;
    let value = if semicent {
        format!("{semi} {cents}")
    } else {
        mode_value.to_owned()
    };
    let painter = ui.painter_at(rect);
    let layout = layout_metric_text(ui, &painter, rect, label, &value);
    let icon_side = layout.label.size().y * 0.82;
    let icon_gap = icon_side * 0.22;
    let show_mode = !audio_rate_drag;
    let header_width = layout.label.size().x + if show_mode { icon_gap + icon_side } else { 0.0 };
    let label_position = egui::pos2(
        rect.center().x - header_width * 0.5,
        layout.label_position.y,
    );
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            label_position.x + layout.label.size().x + icon_gap,
            label_position.y,
        ),
        egui::vec2(icon_side, icon_side),
    );
    let mode_response = show_mode.then(|| {
        ui.interact(
            icon_rect.expand(editor_theme::space::XXS),
            ui.id().with(("oscillator-tuning-mode", oscillator)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "Tuning mode: {}. Click to switch.",
            tuning_mode_label(config.tuning_mode)
        ))
    });
    let mode_hovered = mode_response.as_ref().is_some_and(egui::Response::hovered);
    let mode_active = mode_response
        .as_ref()
        .is_some_and(egui::Response::is_pointer_button_down_on);
    let (label_color, value_color) = metric_readout_colors(
        ui,
        accent,
        semi_hovered || cents_hovered || mode_hovered,
        semi_active || cents_active || mode_active,
    );
    painter.galley(label_position, layout.label, label_color);
    if show_mode {
        if semicent {
            paint_filled_note_icon(&painter, icon_rect, label_color);
        } else if icon_font_ready(ui) {
            let icon = match config.tuning_mode {
                crate::generators::OscillatorTuningMode::Semicent => unreachable!(),
                crate::generators::OscillatorTuningMode::Hertz => egui_phosphor::regular::WAVE_SINE,
                crate::generators::OscillatorTuningMode::Ratio => egui_phosphor::regular::DIVIDE,
            };
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(icon_side),
                label_color,
            );
        } else {
            paint_mode_icon_fallback(&painter, icon_rect, label_color);
        }
    }
    if semicent {
        let value_font = layout.value_font;
        let semi_width = painter
            .layout_no_wrap(semi.to_owned(), value_font.clone(), egui::Color32::WHITE)
            .size()
            .x;
        let gap_width = painter
            .layout_no_wrap(" ".into(), value_font.clone(), egui::Color32::WHITE)
            .size()
            .x;
        let (_, semi_color) = metric_readout_colors(ui, accent, semi_hovered, semi_active);
        let (_, cents_color) = metric_readout_colors(ui, accent, cents_hovered, cents_active);
        painter.text(
            layout.value_position,
            egui::Align2::LEFT_TOP,
            semi,
            value_font.clone(),
            semi_color,
        );
        painter.text(
            egui::pos2(
                layout.value_position.x + semi_width + gap_width,
                layout.value_position.y,
            ),
            egui::Align2::LEFT_TOP,
            cents,
            value_font,
            cents_color,
        );
    } else {
        painter.galley(layout.value_position, layout.value, value_color);
    }
    let keyboard = mode_response.as_ref().is_some_and(|response| {
        response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            })
    });
    if mode_response.as_ref().is_some_and(egui::Response::clicked) || keyboard {
        config.tuning_mode = match config.tuning_mode {
            crate::generators::OscillatorTuningMode::Semicent => {
                crate::generators::OscillatorTuningMode::Hertz
            }
            crate::generators::OscillatorTuningMode::Hertz => {
                crate::generators::OscillatorTuningMode::Ratio
            }
            crate::generators::OscillatorTuningMode::Ratio => {
                crate::generators::OscillatorTuningMode::Semicent
            }
        };
        true
    } else {
        false
    }
}

fn paint_filled_note_icon(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let radius = rect.height() * 0.21;
    let head = egui::pos2(
        rect.left() + rect.width() * 0.38,
        rect.bottom() - radius * 1.1,
    );
    let stem_x = head.x + radius * 0.72;
    painter.circle_filled(head, radius, color);
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(stem_x, rect.top() + rect.height() * 0.16),
            egui::pos2(stem_x + radius * 0.38, head.y),
        ),
        0.0,
        color,
    );
    painter.line_segment(
        [
            egui::pos2(stem_x, rect.top() + rect.height() * 0.18),
            egui::pos2(
                rect.right() - rect.width() * 0.12,
                rect.top() + rect.height() * 0.28,
            ),
        ],
        egui::Stroke::new(radius * 0.42, color),
    );
}

fn paint_mode_icon_fallback(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let stroke = egui::Stroke::new((rect.height() * 0.08).max(1.0), color);
    let left = rect.left() + rect.width() * 0.18;
    let right = rect.right() - rect.width() * 0.18;
    for (y, reverse) in [
        (rect.top() + rect.height() * 0.35, false),
        (rect.bottom() - rect.height() * 0.35, true),
    ] {
        let (start, end) = if reverse {
            (right, left)
        } else {
            (left, right)
        };
        painter.line_segment([egui::pos2(start, y), egui::pos2(end, y)], stroke);
        let direction = if reverse { 1.0 } else { -1.0 };
        painter.line_segment(
            [
                egui::pos2(end, y),
                egui::pos2(
                    end + direction * rect.width() * 0.18,
                    y - rect.height() * 0.16,
                ),
            ],
            stroke,
        );
    }
}
