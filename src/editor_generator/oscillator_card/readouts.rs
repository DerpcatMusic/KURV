use truce_core::editor::PluginContext;

use crate::editor_controls::{fit_font_to_width, paint_metric_readout};
use crate::editor_unison::normalized_unison_rate;
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::voices::SwarmMode;
use crate::{KurvParams, editor_theme};

use super::super::{config_scalar_drag, format_pan, weighted_cells};

fn phaseplant_readout_rects(rect: egui::Rect) -> [egui::Rect; 5] {
    weighted_cells(rect, [1.0; 5])
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
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let position_hovered = pointer.is_some_and(|pointer| left_half(rect).contains(pointer));
    let random_hovered = pointer.is_some_and(|pointer| right_half(rect).contains(pointer));
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
        accent.gamma_multiply(if position_hovered || random_hovered {
            1.0
        } else {
            0.64
        }),
    );
    painter.text(
        egui::pos2(value_left, text_y),
        egui::Align2::LEFT_TOP,
        position_text,
        value_font.clone(),
        if position_active {
            ui.visuals().text_color()
        } else if position_hovered {
            accent
        } else {
            accent.gamma_multiply(0.88)
        },
    );
    painter.text(
        egui::pos2(value_left + position_width + separator, text_y),
        egui::Align2::LEFT_TOP,
        random_text,
        value_font,
        if random_active {
            ui.visuals().text_color()
        } else if random_hovered {
            accent
        } else {
            accent.gamma_multiply(0.88)
        },
    );
}

pub(super) fn draw_oscillator_readouts(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    oscillator_readouts: egui::Rect,
) -> bool {
    let index = slot.index();
    let readouts = phaseplant_readout_rects(oscillator_readouts);
    let mut config_changed = false;
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
                    config_field_drag(ui, state, module_id, slot, config, field, cell.size());
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
    config_changed
}

pub(super) fn draw_unison_readouts(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    unison_readouts: egui::Rect,
) -> bool {
    let index = slot.index();
    let mut config_changed = false;
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
                    unison_config_readout(ui, state, module_id, slot, config, field, cell.size());
            },
        );
    }
    config_changed
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
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
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
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
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
    let hovered = ui.rect_contains_pointer(rect);
    let icon_side = rect.height().min(rect.width()) * 0.22;
    let label_font = fit_font_to_width(
        &painter,
        "JITTR",
        editor_theme::font::caption(),
        rect.width() - icon_side * 1.5,
    );
    let label_color = if active {
        ui.visuals().text_color()
    } else {
        accent.gamma_multiply(if hovered { 1.0 } else { 0.68 })
    };
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
        } else if hovered {
            accent
        } else {
            accent.gamma_multiply(0.88)
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
