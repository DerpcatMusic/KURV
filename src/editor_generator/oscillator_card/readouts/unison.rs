use truce_core::editor::PluginContext;

use crate::editor_controls::layout_metric_text;
use crate::editor_unison::normalized_unison_rate;
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::voices::SwarmMode;
use crate::{KurvParams, editor_theme};

use super::super::super::config_scalar_drag;
use super::{metric_readout_colors, paint_tinted_metric_readout};

pub(in crate::editor_generator::oscillator_card) fn draw_unison_readouts(
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
enum UnisonConfigField {
    Voices,
    Range,
    Jitter,
    Rate,
    Width,
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
                0.25,
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
                0.01,
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
                0.01,
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
    let value_text = format!("{:.0}%", config.unison_jitter * 100.0);
    let (rect, response, amount_changed) = config_scalar_drag(
        ui,
        &mut config.unison_jitter,
        0.0..=1.0,
        0.01,
        defaults.unison_jitter,
        "JITTR",
        &value_text,
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
        response.hovered(),
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
    hovered: bool,
    active: bool,
) {
    let painter = ui.painter_at(rect);
    let accent = editor_theme::semantic().unison;
    let value = format!("{:.0}%", amount * 100.0);
    let layout = layout_metric_text(ui, &painter, rect, "JITTR", &value);
    let icon_side = layout.label.size().y * 0.82;
    let (label_color, value_color) = metric_readout_colors(ui, accent, hovered, active);
    let gap = icon_side * 0.22;
    let group_width = layout.label.size().x + gap + icon_side;
    let label_pos = egui::pos2(rect.center().x - group_width * 0.5, layout.label_position.y);
    painter.galley(label_pos, layout.label, label_color);
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

    painter.galley(layout.value_position, layout.value, value_color);
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
    let mut value_text = format_value(*value);
    let (rect, response, changed) =
        config_scalar_drag(ui, value, range, speed, default, label, &value_text, size);
    if changed {
        value_text = format_value(*value);
    }
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_tinted_metric_readout(
        ui,
        rect,
        label,
        &value_text,
        editor_theme::semantic().unison,
        response.hovered(),
        active,
    );
    (changed, response)
}
