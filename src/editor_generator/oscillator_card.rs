use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_oscillator::oscillator_waveform_view;
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, paint_vertical_selector,
    vertical_selector_value,
};
use crate::editor_widgets::with_child;
use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::{KurvParams, editor_theme};

use super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::{clear_module_bindings, translucent};

mod readouts;

use readouts::{draw_oscillator_readouts, draw_unison_readouts};

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
    apply_host_automation_to_oscillator(ui, state, module_id, slot, &mut config);
    let enabled = config.enabled;
    let mut config_changed = false;
    let mut reset_requested = false;
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
            "Drag to reorder or move between groups; use the left lane to create a group.",
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
            config_changed |= draw_oscillator_readouts(
                ui,
                state,
                module_id,
                slot,
                &mut config,
                oscillator_readouts,
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
            config_changed |=
                draw_unison_readouts(ui, state, module_id, slot, &mut config, unison_readouts);
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
            ui,
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
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(ui, state, target)
        {
            control.apply_normalized(config, normalized);
        }
    }
}

fn restore_host_automated_oscillator_controls(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    base: crate::generators::OscillatorConfig,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OSCILLATOR_HOST_CONTROLS {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        if crate::editor_modulation::host_automation_binding(ui, state, target).is_some() {
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
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
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
