use truce_core::editor::PluginContext;

use crate::editor_controls::{fit_font_to_width, paint_metric_readout};
use crate::editor_oscillator::oscillator_waveform_view;
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, normalized_unison_rate,
    paint_vertical_selector, vertical_selector_value,
};
use crate::editor_widgets::{icon_font_ready, with_child};
use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::voices::SwarmMode;
use crate::{KurvParams, editor_theme};

use super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::{clear_module_bindings, config_scalar_drag, format_pan, translucent, weighted_cells};

fn phaseplant_readout_rects(rect: egui::Rect) -> [egui::Rect; 5] {
    let groups = weighted_cells(rect, [1.0; 4]);
    [
        groups[0],
        left_half(groups[1]),
        right_half(groups[1]),
        groups[2],
        groups[3],
    ]
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
        accent.gamma_multiply(0.64),
    );
    painter.text(
        egui::pos2(value_left, text_y),
        egui::Align2::LEFT_TOP,
        position_text,
        value_font.clone(),
        if position_active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
    painter.text(
        egui::pos2(value_left + position_width + separator, text_y),
        egui::Align2::LEFT_TOP,
        random_text,
        value_font,
        if random_active {
            ui.visuals().text_color()
        } else {
            accent
        },
    );
}

pub(super) fn draw_compact_oscillator(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    slot: OscillatorSlot,
    module_id: ModuleId,
    gap: f32,
    group_accent: egui::Color32,
) {
    let index = slot.index();
    let base_config = state.generator_stack.oscillator_config(slot);
    let mut config = base_config;
    apply_host_automation_to_oscillator(state, module_id, slot, &mut config);
    let enabled = config.enabled;
    let mut config_changed = false;
    let mut reset_requested = false;
    ui.painter()
        .rect_filled(rect, 2.0, editor_theme::semantic().surface);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().grid.gamma_multiply(0.34),
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [rect.left_top(), rect.left_bottom()],
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            group_accent.gamma_multiply(0.72),
        ),
    );
    let panel_gap = (gap * 0.18).max(rect.height() * 0.006);
    let inner = rect.shrink(panel_gap * 0.45);
    let identity_width = inner.width() * 0.055;
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), identity.top() + identity.height() * 0.55),
        identity.max,
    );
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - identity.width() * 0.20,
            identity.top() + identity.height() * 0.10,
        ),
        egui::vec2(identity.width() * 0.36, identity.height() * 0.16),
    );
    let drag_handle = ui
        .interact(
            drag_rect,
            egui::Id::new(("oscillator-group-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(
            "Drag to reorder. Hold Alt to drop inside another group; use the left lane to create a group.",
        );
    drag_handle.dnd_set_drag_payload(module_id);
    drag_handle.context_menu(|ui| {
        if ui.button("RESET OSCILLATOR").clicked() {
            reset_requested = true;
            ui.close();
        }
    });
    if drag_handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("oscillator", module_id.get()),
                pointer,
                rect.size() * 0.52,
                group_accent,
                &format!("OSC {}", index + 1),
                "VIRTUAL ANALOG",
                GeneratorDragGhostKind::Oscillator,
            );
        }
    }
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + panel_gap, inner.top()),
        inner.right_bottom(),
    );
    let panels_width = (body.width() - panel_gap * 2.0).max(1.0);
    let oscillator_width = panels_width * 0.40;
    let oscillator_panel =
        egui::Rect::from_min_size(body.min, egui::vec2(oscillator_width, body.height()));
    let unison_width = panels_width * 0.40;
    let unison_panel = egui::Rect::from_min_size(
        egui::pos2(oscillator_panel.right() + panel_gap, body.top()),
        egui::vec2(unison_width, body.height()),
    );
    let pan_panel = egui::Rect::from_min_max(
        egui::pos2(unison_panel.right() + panel_gap, body.top()),
        body.right_bottom(),
    );
    let readout_height = body.height() * 0.22;
    let wave_label_width = ui
        .painter()
        .layout_no_wrap(
            "WAVE".to_owned(),
            editor_theme::font::caption(),
            egui::Color32::WHITE,
        )
        .size()
        .x
        + editor_theme::space::XS * 2.0;
    let waveform_rail_width = wave_label_width
        .min(oscillator_panel.width() * 0.10)
        .max(oscillator_panel.width() * 0.055);
    let waveform_rail = egui::Rect::from_min_size(
        oscillator_panel.min,
        egui::vec2(waveform_rail_width, oscillator_panel.height()),
    );
    let oscillator_content = egui::Rect::from_min_max(
        egui::pos2(waveform_rail.right(), oscillator_panel.top()),
        oscillator_panel.max,
    );
    let oscillator_plot = egui::Rect::from_min_max(
        oscillator_content.min,
        egui::pos2(
            oscillator_content.right(),
            oscillator_content.bottom() - readout_height,
        ),
    );
    let oscillator_readouts = egui::Rect::from_min_max(
        egui::pos2(oscillator_content.left(), oscillator_plot.bottom()),
        oscillator_panel.right_bottom(),
    );
    let unison_plot = egui::Rect::from_min_max(
        unison_panel.min,
        egui::pos2(unison_panel.right(), unison_panel.bottom() - readout_height),
    );
    let unison_readouts = egui::Rect::from_min_max(
        egui::pos2(unison_panel.left(), unison_plot.bottom()),
        unison_panel.right_bottom(),
    );
    ui.painter()
        .rect_filled(body, 1.0, editor_theme::semantic().well);

    ui.painter().rect_filled(
        identity,
        0.0,
        if drag_handle.dragged() || drag_handle.is_pointer_button_down_on() {
            editor_theme::semantic().control_hover
        } else if drag_handle.hovered() {
            editor_theme::semantic().control
        } else {
            editor_theme::semantic().chrome
        },
    );
    let identity_content = identity.shrink2(egui::vec2(
        identity.width() * 0.08,
        identity.height() * 0.04,
    ));
    with_child(
        ui,
        identity_content,
        ("oscillator-identity", index),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            config_changed |= compact_toggle(ui, &mut config.enabled);
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
        },
    );
    let remove_response = ui
        .interact(
            remove_rect,
            egui::Id::new(("oscillator-remove", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Remove Oscillator {} from this group", index + 1));
    let remove_requested = remove_response.clicked();
    let remove_pressed = remove_response.is_pointer_button_down_on();
    if remove_response.hovered() || remove_pressed {
        ui.painter().rect_filled(
            remove_rect,
            editor_theme::shape::CONTROL_RADIUS,
            translucent(
                editor_theme::semantic().danger,
                if remove_pressed { 48 } else { 28 },
            ),
        );
    }
    ui.painter().text(
        remove_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::label(),
        if remove_pressed || remove_response.hovered() {
            editor_theme::semantic().text
        } else {
            editor_theme::semantic().text_muted
        },
    );
    with_child(
        ui,
        oscillator_readouts,
        ("oscillator-controls", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
            let readouts = phaseplant_readout_rects(oscillator_readouts);
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
                        let (changed, active) = config_field_drag(
                            ui,
                            state,
                            module_id,
                            slot,
                            &mut config,
                            field,
                            cell.size(),
                        );
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
        },
    );

    with_child(
        ui,
        waveform_rail,
        ("compact-wave-shape", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.35 });
            config_changed |= config_wave_field(
                ui,
                state,
                module_id,
                slot,
                &mut config.shape,
                waveform_rail.size(),
            );
        },
    );
    with_child(
        ui,
        oscillator_plot,
        ("compact-wave-cycle", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= oscillator_waveform_view(
                ui,
                state,
                oscillator_plot.width(),
                oscillator_plot.height(),
                module_id,
                slot,
                &mut config,
            );
        },
    );
    with_child(
        ui,
        unison_plot,
        ("compact-unison-distribution", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= custom_unison_distribution_view(
                ui,
                state,
                module_id,
                slot,
                unison_plot.width(),
                unison_plot.height(),
                &mut config,
                state.generator_stack.pan_shape_curve(slot),
            );
        },
    );
    with_child(
        ui,
        pan_panel,
        ("compact-pan-panel", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.28 });
            config_changed |= custom_pan_panel_view(
                ui,
                state,
                module_id,
                slot,
                pan_panel.width(),
                pan_panel.height(),
                &mut config,
                state.generator_stack.pan_shape_curve(slot),
            );
        },
    );
    with_child(
        ui,
        unison_readouts,
        ("compact-unison-controls", index),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_opacity(if enabled { 1.0 } else { 0.38 });
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
                        config_changed |= unison_config_readout(
                            ui,
                            state,
                            module_id,
                            slot,
                            &mut config,
                            field,
                            cell.size(),
                        );
                    },
                );
            }
        },
    );
    let divider = egui::Stroke::new(1.0_f32, editor_theme::semantic().grid.gamma_multiply(0.52));
    for x in [oscillator_panel.right(), unison_panel.right()] {
        ui.painter().line_segment(
            [
                egui::pos2(x + panel_gap * 0.5, body.top()),
                egui::pos2(x + panel_gap * 0.5, body.bottom()),
            ],
            divider,
        );
    }
    if drag_handle.dragged() {
        ui.painter().rect_filled(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            translucent(editor_theme::semantic().chrome, 148),
        );
        ui.painter().rect_stroke(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE,
                editor_theme::semantic().primary,
            ),
            egui::StrokeKind::Inside,
        );
    }
    if reset_requested {
        state.generator_stack.reset_oscillator(slot);
        return;
    }
    if config_changed {
        restore_host_automated_oscillator_controls(
            state,
            module_id,
            slot,
            base_config,
            &mut config,
        );
        state.generator_stack.set_oscillator_config(slot, config);
    }
    if remove_requested
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
    {
        clear_module_bindings(state, module_id);
        let mut removed = base_config;
        removed.enabled = false;
        state.generator_stack.set_oscillator_config(slot, removed);
    }
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

const OSCILLATOR_HOST_CONTROLS: [OscillatorControl; 23] = [
    OscillatorControl::Shape,
    OscillatorControl::TablePosition,
    OscillatorControl::PulseWidth,
    OscillatorControl::Transpose,
    OscillatorControl::Cents,
    OscillatorControl::Level,
    OscillatorControl::Pan,
    OscillatorControl::PhasePosition,
    OscillatorControl::PhaseRandom,
    OscillatorControl::PhaseWarpAmount,
    OscillatorControl::UnisonVoices,
    OscillatorControl::UnisonRange,
    OscillatorControl::UnisonAmount,
    OscillatorControl::UnisonCurve,
    OscillatorControl::UnisonJitter,
    OscillatorControl::UnisonRate,
    OscillatorControl::UnisonWidth,
    OscillatorControl::UnisonWeight,
    OscillatorControl::UnisonAlignment,
    OscillatorControl::UnisonPanCurve,
    OscillatorControl::UnisonPanCenter,
    OscillatorControl::UnisonStereoPosition,
    OscillatorControl::UnisonStereoAlternate,
];

fn apply_host_automation_to_oscillator(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(state, target)
        {
            control.apply_normalized(config, normalized);
        }
    }
}

fn restore_host_automated_oscillator_controls(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    base: crate::generators::OscillatorConfig,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if crate::editor_modulation::host_automation_binding(state, target).is_some() {
            control.apply_normalized(config, control.normalized_value(base));
        }
    }
}

fn compact_toggle(ui: &mut egui::Ui, enabled: &mut bool) -> bool {
    let extent = ui
        .available_width()
        .min(ui.available_height())
        .min(ui.spacing().interact_size.y)
        .max(ui.spacing().interact_size.y * 0.42)
        * 0.72;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(extent, extent), egui::Sense::click());
    let clicked = response.clicked();
    if clicked {
        *enabled = !*enabled;
    }
    let color = if *enabled {
        editor_theme::palette().accent
    } else {
        editor_theme::semantic().grid
    };
    ui.painter()
        .circle_filled(rect.center(), extent * 0.28, color);
    response.on_hover_text(if *enabled {
        "Disable oscillator"
    } else {
        "Enable oscillator"
    });
    clicked
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
    if let Some((_, param, _)) = crate::editor_modulation::host_automation_binding(state, target) {
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
    let host_binding = crate::editor_modulation::host_automation_binding(state, target);
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

fn config_wave_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    value: &mut f32,
    size: egui::Vec2,
) -> bool {
    let minimum = editor_theme::title_height(ui) * 0.8;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(minimum), size.y.max(minimum)),
        egui::Sense::click_and_drag(),
    );
    let label_inset =
        (editor_theme::font::caption().size + editor_theme::space::XXS).min(rect.height() * 0.24);
    let selector_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), (rect.top() + label_inset).min(rect.bottom())),
        egui::pos2(rect.right(), (rect.bottom() - label_inset).max(rect.top())),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let before = *value;
    let target = ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::Shape);
    let host_binding = crate::editor_modulation::host_automation_binding(state, target);
    let owns_modulation =
        crate::editor_modulation::modular_owns_gesture(ui, state, target, &response);
    if response.dragged() && !owns_modulation {
        if ui.input(|input| input.modifiers.shift) {
            *value = (*value - response.drag_motion().y * 0.001).clamp(0.0, 3.0);
        } else if let Some(pointer) = response.interact_pointer_pos() {
            *value = vertical_selector_value(selector_rect, pointer) * 3.0;
        }
    } else if !owns_modulation && (response.double_clicked() || response.secondary_clicked()) {
        *value = crate::generators::OscillatorConfig::default().shape;
    }
    paint_vertical_selector(
        &ui.painter_at(rect),
        selector_rect,
        *value / 3.0,
        editor_theme::semantic().primary,
    );
    let painter = ui.painter_at(rect);
    let shape_name = ["SIN", "TRI", "SAW", "PLS"][value.round().clamp(0.0, 3.0) as usize];
    painter.text(
        rect.center_top() + egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_TOP,
        "WAVE",
        fit_font_to_width(
            &painter,
            "WAVE",
            editor_theme::font::caption(),
            rect.width() * 0.88,
        ),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        rect.center_bottom() - egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_BOTTOM,
        shape_name,
        fit_font_to_width(
            &painter,
            shape_name,
            editor_theme::font::caption(),
            rect.width() * 0.88,
        ),
        editor_theme::semantic().primary,
    );
    let track = egui::Rect::from_min_max(
        egui::pos2(
            selector_rect.center().x - selector_rect.width() * 0.06,
            selector_rect.top(),
        ),
        egui::pos2(
            selector_rect.center().x + selector_rect.width() * 0.06,
            selector_rect.bottom(),
        ),
    );
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        *value / 3.0,
        track,
        crate::editor_modulation::TrackAxis::Vertical,
        1.0,
    );
    let changed = value.to_bits() != before.to_bits();
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state,
            param,
            &response,
            *value / 3.0,
            changed,
        );
    }
    response.on_hover_text("Drag vertically to select the VA waveform; double-click for saw.");
    changed && host_binding.is_none()
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
    let icon_side = rect.height().min(rect.width()) * 0.22;
    let label_font = fit_font_to_width(
        &painter,
        "JITTR",
        editor_theme::font::caption(),
        rect.width() - icon_side * 1.5,
    );
    let label_color = accent.gamma_multiply(if active { 1.0 } else { 0.68 });
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
        } else {
            accent
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
