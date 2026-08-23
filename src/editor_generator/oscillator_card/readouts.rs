use truce_core::editor::PluginContext;

use crate::editor_controls::layout_metric_text;
use crate::editor_widgets::with_child;
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

fn paint_tinted_metric_readout(
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
    position: f32,
    random: f32,
    position_hovered: bool,
    random_hovered: bool,
    position_active: bool,
    random_active: bool,
) {
    let painter = ui.painter_at(rect);
    let accent = editor_theme::semantic().primary;
    let position_text = format!("{position:.0}°");
    let random_text = format!("±{random:.0}°");
    let value_text = format!("{position_text} {random_text}");
    let layout = layout_metric_text(ui, &painter, rect, "PHASE", &value_text);
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
    let readouts = oscillator_readout_rects(oscillator_readouts, include_pan);
    let mut config_changed = false;
    let mut readout_active = [false; 6];
    let mut readout_hovered = [false; 6];
    let mut level = format!("{:.0} %", config.level * 100.0);
    let mut semi = format!("{:+.0}", config.transpose);
    let mut cents = format!("{:+.1}", config.cents);
    let mut pan = format_pan(config.pan);
    let phase_position = format!("{:.0}°", config.phase_position * 360.0);
    let phase_random = format!("±{:.0}°", config.phase_random * 360.0);
    let hits = [
        (
            readouts.level,
            ConfigField::Level,
            0,
            "LEVEL",
            level.as_str(),
        ),
        (readouts.semi, ConfigField::Semi, 1, "SEMI", semi.as_str()),
        (readouts.cent, ConfigField::Fine, 2, "CENT", cents.as_str()),
        (readouts.pan, ConfigField::Pan, 3, "PAN", pan.as_str()),
        (
            left_half(readouts.phase),
            ConfigField::PhasePosition,
            4,
            "PHASE",
            phase_position.as_str(),
        ),
        (
            right_half(readouts.phase),
            ConfigField::PhaseRandom,
            5,
            "PHASE",
            phase_random.as_str(),
        ),
    ];
    for (cell_index, (cell, field, readout_index, label, value_text)) in
        hits.into_iter().enumerate()
    {
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
        cents = format!("{:+.1}", config.cents);
        pan = format_pan(config.pan);
    }
    let accent = editor_theme::semantic().primary;
    for (index, (rect, label, value, active, hovered)) in [
        (
            readouts.level,
            "LEVEL",
            level.as_str(),
            readout_active[0],
            readout_hovered[0],
        ),
        (
            readouts.semi,
            "SEMI",
            semi.as_str(),
            readout_active[1],
            readout_hovered[1],
        ),
        (
            readouts.cent,
            "CENT",
            cents.as_str(),
            readout_active[2],
            readout_hovered[2],
        ),
        (
            readouts.pan,
            "PAN",
            pan.as_str(),
            readout_active[3],
            readout_hovered[3],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if index == 3 && !include_pan {
            continue;
        }
        paint_tinted_metric_readout(ui, rect, label, value, accent, hovered, active);
    }
    let painter = ui.painter_at(readouts.pitch);
    let pitch_layout = layout_metric_text(ui, &painter, readouts.semi, "SEMI", &semi);
    let pitch_active = readout_active[1] || readout_active[2];
    let pitch_hovered = readout_hovered[1] || readout_hovered[2];
    painter.circle_filled(
        egui::pos2(
            readouts.pitch.center().x,
            pitch_layout.value_position.y + pitch_layout.value.size().y * 0.5,
        ),
        editor_theme::shape::STROKE * 1.25,
        editor_theme::semantic()
            .primary
            .gamma_multiply(if pitch_active || pitch_hovered {
                1.0
            } else {
                0.64
            }),
    );
    paint_phaseplant_phase_readout(
        ui,
        readouts.phase,
        config.phase_position * 360.0,
        config.phase_random * 360.0,
        readout_hovered[4],
        readout_hovered[5],
        readout_active[4],
        readout_active[5],
    );
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
        response.hovered(),
    )
}
