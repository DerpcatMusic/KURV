use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_oscillator::oscillator_waveform_view;
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, paint_vertical_selector,
    vertical_selector_value,
};
use crate::editor_widgets::with_child;
use crate::generators::{
    FilterConfig, GeneratorModMode, MAX_OSCILLATORS, ModuleId, ModuleKind,
    OscillatorEngineKind, OscillatorSlot, Patch,
};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::{KurvParams, editor_resynth, editor_theme};

use super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::{MODULE_IDENTITY_SHARE, clear_module_bindings, translucent};

mod readouts;

use readouts::{draw_oscillator_readouts, draw_unison_readouts};

#[derive(Clone, Copy)]
struct PhaseModSource(OscillatorSlot);

fn phase_mod_source_precedes(
    patch: &Patch,
    source: OscillatorSlot,
    target: OscillatorSlot,
) -> bool {
    patch.groups().iter().any(|group| {
        let source = group
            .modules()
            .iter()
            .position(|module| module.kind() == ModuleKind::Oscillator(source));
        let target = group
            .modules()
            .iter()
            .position(|module| module.kind() == ModuleKind::Oscillator(target));
        matches!((source, target), (Some(source), Some(target)) if source < target)
    })
}

fn clear_phase_mod_source(state: &PluginContext<KurvParams>, source: OscillatorSlot) {
    let encoded = source.index() as u8 + 1;
    for index in 0..MAX_OSCILLATORS {
        let slot = OscillatorSlot::from_index(index).expect("bounded oscillator slot");
        let mut config = state.generator_stack.oscillator_config(slot);
        if config.phase_mod_source == encoded {
            config.phase_mod_source = 0;
            config.phase_mod_amount = 0.0;
            state.generator_stack.set_oscillator_config(slot, config);
        }
    }
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
    apply_host_automation_to_oscillator(ui, state, module_id, slot, &mut config);
    let enabled = config.enabled;
    let is_resynth = config.engine == OscillatorEngineKind::Resynth;
    let is_noise = config.engine == OscillatorEngineKind::Noise;
    let engine_short = if is_resynth {
        "RES"
    } else if is_noise {
        "NOI"
    } else {
        "OSC"
    };
    let mut config_changed = false;
    let mut reset_requested = false;
    let panel_gap = (gap * 0.18).max(rect.height() * 0.006);
    let inner = rect.shrink(panel_gap * 0.45);
    let identity_width = inner.width() * MODULE_IDENTITY_SHARE;
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    let close_side = identity.width() * 0.42;
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - close_side * 0.42,
            identity.top() + close_side * 0.42,
        ),
        egui::Vec2::splat(close_side),
    );
    let grip_height = (identity.height() * 0.18)
        .max(editor_theme::space::MD)
        .min((identity.height() - close_side).max(0.0));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), identity.bottom() - grip_height),
        identity.right_bottom(),
    );
    let drag_handle = ui
        .interact(
            drag_rect,
            egui::Id::new(("oscillator-group-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag this grip to reorder or move the oscillator between groups.");
    let grip_color = if drag_handle.dragged() {
        editor_theme::semantic().text
    } else if drag_handle.hovered() || drag_handle.has_focus() {
        group_accent
    } else {
        editor_theme::semantic().text_muted.gamma_multiply(0.56)
    };
    let grip_gap = editor_theme::space::XXS;
    let grip_origin = drag_rect.center() - egui::vec2(grip_gap * 0.5, grip_gap);
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                grip_origin + egui::vec2(column as f32 * grip_gap, row as f32 * grip_gap),
                editor_theme::shape::STROKE,
                grip_color,
            );
        }
    }
    drag_handle.dnd_set_drag_payload(module_id);
    drag_handle.context_menu(|ui| {
        if ui.button("RESET OSCILLATOR").clicked() {
            reset_requested = true;
            ui.close();
        }
        if ui.button("REMOVE OSCILLATOR").clicked() {
            ui.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("oscillator-remove-menu", module_id.get())),
                    true,
                );
            });
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
                rect.size(),
                group_accent,
                &format!("{engine_short} {}", index + 1),
                if is_resynth {
                    "RESYNTH OSCILLATOR"
                } else if is_noise {
                    "NOISE OSCILLATOR"
                } else {
                    "VIRTUAL ANALOG"
                },
                GeneratorDragGhostKind::Oscillator,
            );
        }
    }
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + panel_gap, inner.top()),
        inner.right_bottom(),
    );
    let grain_card = is_resynth
        && state
            .resynth_assets
            .slot(index)
            .and_then(crate::resynth_state::ResynthSlotState::source_summary)
            .is_none_or(|summary| summary.selected != crate::oscillators::ResynthAlgorithm::Rich);
    let panels_width = (body.width() - panel_gap * 2.0).max(1.0);
    let oscillator_width = if is_resynth {
        panels_width * 0.50
    } else {
        panels_width * 0.40
    };
    let oscillator_panel =
        egui::Rect::from_min_size(body.min, egui::vec2(oscillator_width, body.height()));
    let unison_width = if is_resynth {
        (body.width() - oscillator_width - panel_gap).max(1.0)
    } else {
        panels_width * 0.40
    };
    let unison_panel = egui::Rect::from_min_size(
        egui::pos2(oscillator_panel.right() + panel_gap, body.top()),
        egui::vec2(unison_width, body.height()),
    );
    let pan_panel = if is_resynth {
        egui::Rect::from_min_size(body.max, egui::Vec2::ZERO)
    } else {
        egui::Rect::from_min_max(
            egui::pos2(unison_panel.right() + panel_gap, body.top()),
            body.right_bottom(),
        )
    };
    ui.painter()
        .rect_filled(body, 0.0, editor_theme::semantic().well);
    let oscillator_readout_height = body.height() * if is_resynth { 0.34 } else { 0.22 };
    let unison_readout_height = body.height() * 0.22;
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
    let waveform_rail_width = if is_resynth || is_noise {
        0.0
    } else {
        wave_label_width
            .min(oscillator_panel.width() * 0.10)
            .max(oscillator_panel.width() * 0.055)
    };
    let waveform_rail = egui::Rect::from_min_size(
        oscillator_panel.min,
        egui::vec2(waveform_rail_width, oscillator_panel.height()),
    );
    let oscillator_content = if is_resynth || is_noise {
        oscillator_panel
    } else {
        egui::Rect::from_min_max(
            egui::pos2(waveform_rail.right(), oscillator_panel.top()),
            oscillator_panel.max,
        )
    };
    let oscillator_plot = egui::Rect::from_min_max(
        oscillator_content.min,
        egui::pos2(
            oscillator_content.right(),
            oscillator_content.bottom() - oscillator_readout_height,
        ),
    );
    let oscillator_readouts = egui::Rect::from_min_max(
        egui::pos2(oscillator_content.left(), oscillator_plot.bottom()),
        oscillator_panel.right_bottom(),
    );
    let unison_plot = egui::Rect::from_min_max(
        unison_panel.min,
        egui::pos2(
            unison_panel.right(),
            unison_panel.bottom() - unison_readout_height,
        ),
    );
    let unison_readouts = egui::Rect::from_min_max(
        egui::pos2(unison_panel.left(), unison_plot.bottom()),
        unison_panel.right_bottom(),
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
                egui::RichText::new(engine_short)
                    .font(editor_theme::font::caption())
                    .color(
                        if drag_handle.hovered() || drag_handle.dragged() || drag_handle.has_focus()
                        {
                            group_accent
                        } else {
                            editor_theme::semantic().text_muted
                        },
                    ),
            );
            ui.label(
                egui::RichText::new((index + 1).to_string())
                    .font(editor_theme::font::title())
                    .color(
                        if drag_handle.hovered() || drag_handle.dragged() || drag_handle.has_focus()
                        {
                            group_accent
                        } else {
                            editor_theme::semantic().text
                        },
                    ),
            );
        },
    );
    let source_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.center().x,
            identity.bottom() - grip_height - close_side * 0.36,
        ),
        egui::Vec2::splat(close_side * 0.62),
    );
    let source_drag = ui
        .interact(
            source_rect,
            egui::Id::new(("oscillator-phase-source", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(
            "Drag this audio-rate source onto a later oscillator for PM, AM, ring or pan modulation.",
        );
    source_drag.dnd_set_drag_payload(PhaseModSource(slot));
    ui.painter().circle_stroke(
        source_rect.center(),
        source_rect.width() * 0.20,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            if source_drag.hovered() || source_drag.dragged() {
                group_accent
            } else {
                editor_theme::semantic().text_muted
            },
        ),
    );
    let remove_menu = ui.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::new(("oscillator-remove-menu", module_id.get())))
            .unwrap_or(false)
    });
    let identity_hot = ui.rect_contains_pointer(identity)
        || drag_handle.hovered()
        || drag_handle.dragged()
        || drag_handle.has_focus();
    let remove_response = ui
        .interact(
            remove_rect,
            egui::Id::new(("oscillator-remove", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Remove Oscillator {} from this group", index + 1));
    let remove_requested = remove_response.clicked() || remove_menu;
    let remove_pressed = remove_response.is_pointer_button_down_on();
    if identity_hot || remove_response.hovered() || remove_pressed {
        if remove_pressed {
            ui.painter().rect_filled(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(editor_theme::semantic().danger, 48),
            );
        }
        ui.painter().text(
            remove_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            editor_theme::font::caption(),
            if remove_pressed || remove_response.hovered() {
                editor_theme::semantic().danger
            } else {
                editor_theme::semantic().text_muted.gamma_multiply(0.72)
            },
        );
    }
    if is_resynth {
        config_changed |= editor_resynth::draw_resynth_body(
            ui,
            state,
            oscillator_plot,
            slot,
            module_id,
            &mut config,
        );
        let mode_readouts = egui::Rect::from_min_max(
            oscillator_readouts.min,
            egui::pos2(oscillator_readouts.right(), oscillator_readouts.center().y),
        );
        let shared_readouts = egui::Rect::from_min_max(
            egui::pos2(oscillator_readouts.left(), mode_readouts.bottom()),
            oscillator_readouts.max,
        );
        with_child(
            ui,
            mode_readouts,
            ("resynth-mode-controls", index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_opacity(if enabled { 1.0 } else { 0.38 });
                config_changed |= editor_resynth::draw_algorithm_controls_panel(
                    ui,
                    state,
                    slot,
                    module_id,
                    mode_readouts,
                );
            },
        );
        with_child(
            ui,
            shared_readouts,
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
                    shared_readouts,
                    false,
                );
            },
        );
        if grain_card {
            config_changed |=
                editor_resynth::draw_grain_shape_panel(ui, state, slot, module_id, unison_panel);
        } else {
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
                unison_readouts,
                ("compact-unison-controls", index),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_opacity(if enabled { 1.0 } else { 0.38 });
                    config_changed |= draw_unison_readouts(
                        ui,
                        state,
                        module_id,
                        slot,
                        &mut config,
                        unison_readouts,
                    );
                },
            );
        }
        let divider =
            egui::Stroke::new(1.0_f32, editor_theme::semantic().grid.gamma_multiply(0.52));
        let seams: &[f32] = if is_resynth {
            &[oscillator_panel.right()]
        } else {
            &[oscillator_panel.right(), unison_panel.right()]
        };
        for x in seams {
            ui.painter().line_segment(
                [
                    egui::pos2(x + panel_gap * 0.5, body.top()),
                    egui::pos2(x + panel_gap * 0.5, body.bottom()),
                ],
                divider,
            );
        }
    } else {
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
                    true,
                );
            },
        );

        if is_noise {
            config_changed |= draw_noise_panel(
                ui,
                state,
                oscillator_plot,
                module_id,
                slot,
                &mut config,
                enabled,
            );
        } else {
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
                        &mut config,
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
        }
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
        let divider =
            egui::Stroke::new(1.0_f32, editor_theme::semantic().grid.gamma_multiply(0.52));
        for x in [oscillator_panel.right(), unison_panel.right()] {
            ui.painter().line_segment(
                [
                    egui::pos2(x + panel_gap * 0.5, body.top()),
                    egui::pos2(x + panel_gap * 0.5, body.bottom()),
                ],
                divider,
            );
        }
    }
    {
        if config.phase_mod_source != 0
            && OscillatorSlot::from_index(usize::from(config.phase_mod_source - 1))
                .is_none_or(|source| {
                    !phase_mod_source_precedes(&state.generator_stack.snapshot(), source, slot)
                })
        {
            config.phase_mod_source = 0;
            config.phase_mod_amount = 0.0;
            config_changed = true;
        }
        let phase_target = ui.interact(
            oscillator_plot,
            egui::Id::new(("oscillator-phase-target", module_id.get())),
            egui::Sense::hover(),
        );
        if let Some(source) = phase_target.dnd_release_payload::<PhaseModSource>()
            && phase_mod_source_precedes(&state.generator_stack.snapshot(), source.0, slot)
            && (config.phase_mod_source == 0
                || config.phase_mod_source == source.0.index() as u8 + 1)
        {
            config.phase_mod_source = source.0.index() as u8 + 1;
            if is_resynth || is_noise {
                config.modulation_mode = GeneratorModMode::Amplitude;
            }
            if config.phase_mod_amount.abs() <= f32::EPSILON {
                config.phase_mod_amount = 0.25;
            }
            config_changed = true;
        }
        let valid_hover = phase_target
            .dnd_hover_payload::<PhaseModSource>()
            .is_some_and(|source| {
                phase_mod_source_precedes(&state.generator_stack.snapshot(), source.0, slot)
                    && (config.phase_mod_source == 0
                        || config.phase_mod_source == source.0.index() as u8 + 1)
            });
        if valid_hover {
            ui.painter().rect_stroke(
                oscillator_plot.shrink(1.0),
                editor_theme::shape::CONTROL_RADIUS,
                egui::Stroke::new(2.0_f32, group_accent),
                egui::StrokeKind::Inside,
            );
        }
        if config.phase_mod_source != 0 {
            let pm_rect = egui::Rect::from_min_size(
                oscillator_plot.min + egui::vec2(editor_theme::space::XS, editor_theme::space::XS),
                egui::vec2(
                    oscillator_plot.width() * 0.30,
                    editor_theme::title_height(ui),
                ),
            );
            let pm = ui
                .interact(
                    pm_rect,
                    egui::Id::new(("oscillator-phase-depth", module_id.get())),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text("Drag for bipolar audio-rate depth. Right-click to choose mode or clear.");
            let modulation_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::PhaseModAmount,
            );
            let before_amount = config.phase_mod_amount;
            let modulation_owns = crate::editor_modulation::modular_owns_gesture(
                ui,
                state,
                modulation_target,
                &pm,
            );
            if pm.dragged() && !modulation_owns {
                config.phase_mod_amount = (config.phase_mod_amount
                    + ui.input(|input| input.pointer.delta().x) / 160.0)
                    .clamp(-1.0, 1.0);
                config_changed = true;
            }
            pm.context_menu(|ui| {
                for (label, mode) in [
                    ("PHASE", GeneratorModMode::Phase),
                    ("AMPLITUDE", GeneratorModMode::Amplitude),
                    ("RING", GeneratorModMode::Ring),
                    ("PAN", GeneratorModMode::Pan),
                ] {
                    if ui
                        .selectable_label(config.modulation_mode == mode, label)
                        .clicked()
                    {
                        if config.modulation_mode != mode {
                            config.modulation_mode = mode;
                            config.phase_mod_amount = 0.0;
                            config_changed = true;
                        }
                        ui.close();
                    }
                }
                if ui.button("CLEAR ROUTE").clicked() {
                    config.phase_mod_source = 0;
                    config.phase_mod_amount = 0.0;
                    config_changed = true;
                    ui.close();
                }
            });
            let normalized = config.phase_mod_amount.mul_add(0.5, 0.5);
            if let Some((_, param, _)) =
                crate::editor_modulation::host_automation_binding(ui, state, modulation_target)
            {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &pm,
                    normalized,
                    config.phase_mod_amount != before_amount,
                );
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                modulation_target,
                &pm,
                normalized,
                egui::Rect::from_min_max(
                    egui::pos2(pm_rect.left(), pm_rect.bottom() - editor_theme::shape::STROKE * 2.0),
                    pm_rect.right_bottom(),
                ),
                crate::editor_modulation::TrackAxis::Horizontal,
                1.0,
            );
            ui.painter().rect_filled(
                pm_rect,
                editor_theme::shape::CONTROL_RADIUS,
                editor_theme::semantic().surface.gamma_multiply(0.92),
            );
            ui.painter().text(
                pm_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{} {}  {:+.0}%",
                    match config.modulation_mode {
                        GeneratorModMode::Phase => "PM",
                        GeneratorModMode::Amplitude => "AM",
                        GeneratorModMode::Ring => "RM",
                        GeneratorModMode::Pan => "PAN",
                    },
                    config.phase_mod_source,
                    config.phase_mod_amount * 100.0
                ),
                editor_theme::font::caption(),
                group_accent,
            );
        }
    }
    if reset_requested {
        if is_resynth || is_noise {
            let reset = crate::generators::OscillatorConfig::for_engine(config.engine);
            state.generator_stack.set_oscillator_config(slot, reset);
        } else {
            state.generator_stack.reset_oscillator(slot);
        }
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
        && let Ok(module) = state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id))
    {
        clear_module_bindings(state, module.id());
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
                clear_phase_mod_source(state, slot);
                let mut config = state.generator_stack.oscillator_config(slot);
                config.enabled = false;
                state.generator_stack.set_oscillator_config(slot, config);
                if let Some(asset) = state.resynth_assets.slot(slot.index()) {
                    asset.clear();
                }
            }
            ModuleKind::Filter(slot) => state
                .generator_stack
                .set_filter_config(slot, FilterConfig::default()),
        }
    }
}

fn draw_noise_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    enabled: bool,
) -> bool {
    let controls_height = (rect.height() * 0.24).max(editor_theme::title_height(ui));
    let plot = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.bottom() - controls_height),
    );
    let controls = egui::Rect::from_min_max(egui::pos2(rect.left(), plot.bottom()), rect.max);
    let painter = ui.painter_at(plot);
    let graph = plot.shrink(editor_theme::graph_inset(ui).min(plot.height() * 0.18));
    painter.line_segment(
        [
            egui::pos2(graph.left(), graph.center().y),
            egui::pos2(graph.right(), graph.center().y),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, editor_theme::semantic().grid),
    );
    let count = (graph.width().round() as usize).clamp(64, 384);
    let mut noise = crate::oscillators::NoiseState::default();
    noise.reset(0x4e4f_4953_455f_5549 ^ slot.index() as u64);
    let texture = (config.pulse_width - 0.03) / 0.94;
    let mut left_points = Vec::with_capacity(count);
    let mut right_points = Vec::with_capacity(count);
    for index in 0..count {
        let (left, right) = noise.next(
            220.0 / 48_000.0,
            config.shape / 3.0,
            texture,
            config.phase_warp_amount,
            1,
            &[1.0],
            &[1.0],
        );
        let x = graph.left() + index as f32 / (count - 1) as f32 * graph.width();
        left_points.push(egui::pos2(
            x,
            graph.center().y - left.clamp(-1.0, 1.0) * graph.height() * 0.46,
        ));
        right_points.push(egui::pos2(
            x,
            graph.center().y - right.clamp(-1.0, 1.0) * graph.height() * 0.46,
        ));
    }
    painter.add(egui::Shape::line(
        right_points,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic()
                .unison
                .gamma_multiply(if enabled { 0.48 } else { 0.18 }),
        ),
    ));
    painter.add(egui::Shape::line(
        left_points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            editor_theme::semantic()
                .primary
                .gamma_multiply(if enabled { 0.86 } else { 0.28 }),
        ),
    ));

    let width = controls.width() / 3.0;
    let cells: [egui::Rect; 3] = std::array::from_fn(|index| {
        egui::Rect::from_min_max(
            egui::pos2(controls.left() + width * index as f32, controls.top()),
            egui::pos2(
                if index == 2 {
                    controls.right()
                } else {
                    controls.left() + width * (index + 1) as f32
                },
                controls.bottom(),
            ),
        )
    });
    let mut changed = false;
    changed |= noise_control(
        ui,
        state,
        cells[0],
        module_id,
        slot,
        "COLOR",
        OscillatorControl::Shape,
        &mut config.shape,
        0.0..=3.0,
        1.5,
        3.0,
    );
    changed |= noise_control(
        ui,
        state,
        cells[1],
        module_id,
        slot,
        "TEXTURE",
        OscillatorControl::PulseWidth,
        &mut config.pulse_width,
        0.03..=0.97,
        0.03,
        0.94,
    );
    changed |= noise_control(
        ui,
        state,
        cells[2],
        module_id,
        slot,
        "STEREO",
        OscillatorControl::PhaseWarpAmount,
        &mut config.phase_warp_amount,
        0.0..=1.0,
        1.0,
        1.0,
    );
    changed
}

#[allow(clippy::too_many_arguments)]
fn noise_control(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    module_id: ModuleId,
    slot: OscillatorSlot,
    label: &str,
    control: OscillatorControl,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    span: f32,
) -> bool {
    let before = *value;
    let normalized = if control == OscillatorControl::PulseWidth {
        (*value - 0.03) / 0.94
    } else {
        *value / span
    };
    let value_text = format!("{:.0}%", normalized.clamp(0.0, 1.0) * 100.0);
    let (_, response, mut changed) = super::config_scalar_drag(
        ui,
        value,
        range,
        span * 0.01,
        default,
        label,
        &value_text,
        rect.size(),
    );
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
        *value = before;
        changed = false;
    }
    let normalized = if control == OscillatorControl::PulseWidth {
        (*value - 0.03) / 0.94
    } else {
        *value / span
    };
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
    {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, &response, normalized, changed,
        );
        changed = false;
    }
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        &response,
        normalized,
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - rect.height() * 0.08),
            rect.right_bottom(),
        ),
        crate::editor_modulation::TrackAxis::Horizontal,
        1.0,
    );
    changed
}

fn apply_host_automation_to_oscillator(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
) {
    for control in OscillatorControl::ALL.iter().copied() {
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
    for control in OscillatorControl::ALL.iter().copied() {
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

const CANONICAL_WAVE_POSITIONS: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

fn soft_snap_wave_position(position: f32, custom_positions: &[f32], threshold: f32) -> f32 {
    CANONICAL_WAVE_POSITIONS
        .iter()
        .chain(custom_positions)
        .copied()
        .map(|candidate| (candidate, (candidate - position).abs()))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= threshold)
        .map_or(position, |(candidate, _)| candidate)
}

fn config_wave_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
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
    let table = state.params().generator_stack.va_table(slot).snapshot();
    let legacy_table = !table.frames.is_empty() && !table.is_positioned();
    let target = ModulationRouteTarget::oscillator(
        module_id,
        slot,
        if legacy_table {
            OscillatorControl::TablePosition
        } else {
            OscillatorControl::Shape
        },
    );
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
    let positioned_wave = table.is_positioned();
    let owns_modulation = !positioned_wave
        && crate::editor_modulation::modular_owns_gesture(ui, state, target, &response);
    let before = if legacy_table {
        config.custom_shape
    } else {
        config.shape / 3.0
    };
    let mut normalized = before;
    if response.dragged() && !owns_modulation {
        normalized = if ui.input(|input| input.modifiers.shift) {
            (normalized - response.drag_motion().y * 0.001).clamp(0.0, 1.0)
        } else if let Some(pointer) = response.interact_pointer_pos() {
            vertical_selector_value(selector_rect, pointer)
        } else {
            normalized
        };
        if !legacy_table && !ui.input(|input| input.modifiers.alt) {
            let threshold =
                (editor_theme::space::SM / selector_rect.height().max(1.0)).clamp(0.015, 0.05);
            normalized = soft_snap_wave_position(normalized, &table.positions, threshold);
        }
    } else if !owns_modulation && (response.double_clicked() || response.secondary_clicked()) {
        normalized = if legacy_table {
            0.0
        } else {
            crate::generators::OscillatorConfig::default().shape / 3.0
        };
    }
    if legacy_table {
        config.custom_shape = normalized;
    } else {
        config.shape = normalized * 3.0;
    }

    paint_vertical_selector(
        &ui.painter_at(rect),
        selector_rect,
        normalized,
        editor_theme::semantic().primary,
    );
    let painter = ui.painter_at(rect);
    let paint_mark = |position: f32, half_width: f32, color: egui::Color32| {
        let y = selector_rect.bottom() - position.clamp(0.0, 1.0) * selector_rect.height();
        painter.line_segment(
            [
                egui::pos2(selector_rect.center().x - half_width, y),
                egui::pos2(selector_rect.center().x + half_width, y),
            ],
            egui::Stroke::new(editor_theme::shape::STROKE, color),
        );
    };
    if legacy_table {
        for frame in 0..table.frames.len() {
            paint_mark(
                crate::oscillators::position_for_frame(frame, table.frames.len()),
                selector_rect.width() * 0.17,
                editor_theme::semantic().primary.gamma_multiply(0.58),
            );
        }
    } else {
        for position in CANONICAL_WAVE_POSITIONS {
            paint_mark(
                position,
                selector_rect.width() * 0.22,
                editor_theme::semantic().text_muted.gamma_multiply(0.62),
            );
        }
        for position in &table.positions {
            paint_mark(
                *position,
                selector_rect.width() * 0.31,
                editor_theme::semantic().primary,
            );
        }
    }
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
    let footer = if legacy_table {
        if normalized <= f32::EPSILON {
            "BASE".to_owned()
        } else {
            let frame = crate::oscillators::nearest_frame_index(normalized, table.frames.len()) + 1;
            format!("VA {frame}/{}", table.frames.len())
        }
    } else if let Some(frame) = table.frame_index_at_position(normalized) {
        format!("KEY {}", frame + 1)
    } else {
        ["SIN", "TRI", "SAW", "PLS"][(normalized * 3.0).round().clamp(0.0, 3.0) as usize].to_owned()
    };
    painter.text(
        rect.center_bottom() - egui::vec2(0.0, editor_theme::space::XXS),
        egui::Align2::CENTER_BOTTOM,
        &footer,
        fit_font_to_width(
            &painter,
            &footer,
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
    // Internal routing cannot carry the complete positioned selection yet;
    // host automation still drives the unified WAVE/Shape parameter.
    if !positioned_wave {
        crate::editor_modulation::modular_destination(
            ui,
            state,
            target,
            &response,
            normalized,
            track,
            crate::editor_modulation::TrackAxis::Vertical,
            1.0,
        );
    }
    let changed = normalized.to_bits() != before.to_bits();
    if let Some((_, param, _)) = host_binding {
        let edit_id = response.id.with("wave-axis-host-edit");
        if response.drag_started() {
            ui.data_mut(|store| store.insert_temp(edit_id, true));
        }
        crate::editor_modulation::update_host_automation_gesture(
            state, param, &response, normalized, changed,
        );
        if response.drag_stopped() {
            ui.data_mut(|store| store.remove::<bool>(edit_id));
        } else if ui
            .data(|store| store.get_temp::<bool>(edit_id))
            .unwrap_or(false)
            && crate::editor_controls::pointer_gesture_aborted(ui)
        {
            ui.data_mut(|store| store.remove::<bool>(edit_id));
            crate::editor::end_edit(state, param);
        }
    }
    response.on_hover_text(if legacy_table {
        "Drag vertically through this legacy VA table."
    } else {
        "Drag through SIN, TRI, SAW, and PLS. Nearby shapes and custom keys softly snap; hold Alt to bypass."
    });
    changed && host_binding.is_none()
}

#[cfg(test)]
mod wave_axis_tests {
    use super::soft_snap_wave_position;

    #[test]
    fn wave_axis_soft_snap_chooses_nearest_factory_or_custom_key() {
        assert!((soft_snap_wave_position(0.34, &[], 0.02) - 1.0 / 3.0).abs() < f32::EPSILON);
        assert!((soft_snap_wave_position(0.49, &[0.5], 0.02) - 0.5).abs() < f32::EPSILON);
        assert!((soft_snap_wave_position(0.42, &[0.5], 0.02) - 0.42).abs() < f32::EPSILON);
    }
}
