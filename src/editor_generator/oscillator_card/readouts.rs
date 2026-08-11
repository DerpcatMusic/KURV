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

fn oscillator_readout_rects(rect: egui::Rect) -> OscillatorReadoutRects {
    let cell_width = rect.width() / 5.0;
    let cells: [egui::Rect; 5] = std::array::from_fn(|index| {
        let left = rect.left() + cell_width * index as f32;
        let right = if index == 4 {
            rect.right()
        } else {
            left + cell_width
        };
        egui::Rect::from_x_y_ranges(left..=right, rect.y_range())
    });
    OscillatorReadoutRects {
        level: cells[0],
        semi: cells[1],
        cent: cells[2],
        pitch: egui::Rect::from_min_max(cells[1].min, cells[2].max),
        pan: cells[3],
        phase: cells[4],
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
) -> bool {
    let index = slot.index();
    let readouts = oscillator_readout_rects(oscillator_readouts);
    let mut config_changed = false;
    let mut readout_active = [false; 6];
    let mut readout_hovered = [false; 6];
    let hits = [
        (readouts.level, ConfigField::Level, 0),
        (readouts.semi, ConfigField::Semi, 1),
        (readouts.cent, ConfigField::Fine, 2),
        (readouts.pan, ConfigField::Pan, 3),
        (left_half(readouts.phase), ConfigField::PhasePosition, 4),
        (right_half(readouts.phase), ConfigField::PhaseRandom, 5),
    ];
    for (cell_index, (cell, field, readout_index)) in hits.into_iter().enumerate() {
        with_child(
            ui,
            cell,
            ("compact-config", index, cell_index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let (changed, active, hovered) =
                    config_field_drag(ui, state, module_id, slot, config, field, cell.size());
                config_changed |= changed;
                readout_active[readout_index] |= active;
                readout_hovered[readout_index] |= hovered;
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
    let accent = editor_theme::semantic().primary;
    for (rect, label, value, active, hovered) in [
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
    ] {
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
        phase_position,
        phase_random,
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
    size: egui::Vec2,
) -> (bool, bool, bool) {
    let before = *config;
    let defaults = crate::generators::OscillatorConfig::default();
    let (changed, response) = match field {
        ConfigField::Level => {
            let (_, response, changed) =
                config_scalar_drag(ui, &mut config.level, 0.0..=1.0, 0.01, defaults.level, size);
            (changed, response)
        }
        ConfigField::Pan => {
            let (_, response, changed) =
                config_scalar_drag(ui, &mut config.pan, -1.0..=1.0, 0.02, defaults.pan, size);
            (changed, response)
        }
        ConfigField::Fine => {
            let (_, response, changed) = config_scalar_drag(
                ui,
                &mut config.cents,
                -100.0..=100.0,
                0.5,
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
                0.125,
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
                0.01,
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
                0.01,
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
        response.hovered(),
    )
}
