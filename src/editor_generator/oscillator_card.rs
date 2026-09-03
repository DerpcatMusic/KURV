use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_oscillator::oscillator_waveform_view;
use crate::editor_unison::{
    custom_pan_panel_view, custom_unison_distribution_view, paint_vertical_selector,
    vertical_selector_value,
};
use crate::editor_widgets::{paint_power_icon, paint_vertical_label, with_child};
use crate::generators::{FilterConfig, ModuleId, ModuleKind, OscillatorEngineKind, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl, ResolvedRouteSource};
use crate::{KurvParams, editor_resynth, editor_theme};

use super::{MODULE_IDENTITY_SHARE, clear_module_bindings, format_pan, translucent};

mod readouts;

use readouts::{draw_oscillator_readouts, draw_unison_readouts, paint_tinted_metric_readout};

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
    let engine_label = if is_resynth {
        "RESYNTH"
    } else if is_noise {
        "NOISE"
    } else {
        "OSCILLATOR"
    };
    let mut config_changed = false;
    let mut reset_requested = false;
    let panel_gap = (gap * 0.18).max(rect.height() * 0.006);
    let inner = rect.shrink(panel_gap * 0.45);
    let identity_width = inner.width() * MODULE_IDENTITY_SHARE;
    let identity = egui::Rect::from_min_size(inner.min, egui::vec2(identity_width, inner.height()));
    ui.painter()
        .rect_filled(identity, 0.0, editor_theme::semantic().control);
    ui.painter().line_segment(
        [identity.right_top(), identity.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::GROUP_STROKE,
            group_accent.gamma_multiply(0.72),
        ),
    );
    let close_side = identity.width() * 0.42;
    let remove_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - close_side * 0.42,
            identity.top() + close_side * 0.42,
        ),
        egui::Vec2::splat(close_side),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(identity.right() + panel_gap, inner.top()),
        inner.right_bottom(),
    );
    let resynth_algorithm = is_resynth.then(|| {
        state
            .resynth_assets
            .slot(index)
            .and_then(crate::resynth_state::ResynthSlotState::source_summary)
            .map_or(crate::oscillators::ResynthAlgorithm::Grain, |summary| {
                summary.selected
            })
    });
    let grain_card = resynth_algorithm
        .is_some_and(|algorithm| algorithm != crate::oscillators::ResynthAlgorithm::Rich);
    let panels_width = (body.width() - panel_gap * 2.0).max(1.0);
    let oscillator_width = if is_resynth {
        panels_width * 0.50
    } else if is_noise {
        panels_width * 0.58
    } else {
        panels_width * 0.40
    };
    let oscillator_panel =
        egui::Rect::from_min_size(body.min, egui::vec2(oscillator_width, body.height()));
    let unison_width = if is_resynth || is_noise {
        (body.width() - oscillator_width - panel_gap).max(1.0)
    } else {
        panels_width * 0.40
    };
    let unison_panel = egui::Rect::from_min_size(
        egui::pos2(oscillator_panel.right() + panel_gap, body.top()),
        egui::vec2(unison_width, body.height()),
    );
    let pan_panel = if is_resynth || is_noise {
        egui::Rect::from_min_size(body.max, egui::Vec2::ZERO)
    } else {
        egui::Rect::from_min_max(
            egui::pos2(unison_panel.right() + panel_gap, body.top()),
            body.right_bottom(),
        )
    };
    ui.painter()
        .rect_filled(body, 0.0, editor_theme::semantic().well);
    let oscillator_readout_height = body.height()
        * if is_resynth {
            0.22
        } else if is_noise {
            0.0
        } else {
            0.22
        };
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
    let action_side = (close_side * 0.62).min(identity_content.width() * 0.76);
    let gap = editor_theme::space::XS;
    let power_row = egui::Rect::from_center_size(
        egui::pos2(
            identity_content.center().x,
            identity_content.top() + action_side * 0.5,
        ),
        egui::vec2(identity_content.width(), action_side),
    );
    let source_row = egui::Rect::from_center_size(
        egui::pos2(
            identity_content.center().x,
            identity_content.bottom() - action_side * 0.5,
        ),
        egui::vec2(identity_content.width(), action_side),
    );
    let name_row = egui::Rect::from_min_max(
        egui::pos2(identity_content.left(), power_row.bottom() + gap),
        egui::pos2(identity_content.right(), source_row.top() - gap),
    );
    let source_rect =
        egui::Rect::from_center_size(source_row.center(), egui::Vec2::splat(action_side));
    let vertical_rect = name_row;
    with_child(
        ui,
        power_row,
        ("oscillator-identity-power", index),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            config_changed |= compact_toggle(ui, &mut config.enabled);
        },
    );
    let drag_handle = ui
        .interact(
            vertical_rect,
            egui::Id::new(("oscillator-group-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag the oscillator name to move; hold Ctrl to duplicate");
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
        ui.ctx()
            .set_cursor_icon(if ui.input(|input| input.modifiers.ctrl) {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
    }
    let vertical_text = format!("{engine_label} {}", index + 1);
    let label_color = if !config.enabled {
        editor_theme::semantic().disabled_text
    } else if drag_handle.hovered() || drag_handle.dragged() || drag_handle.has_focus() {
        group_accent
    } else {
        editor_theme::semantic().text
    };
    paint_vertical_label(
        ui,
        vertical_rect,
        &vertical_text,
        fit_font_to_width(
            ui.painter(),
            &vertical_text,
            editor_theme::font::title(),
            identity_content.height() * 0.90,
        ),
        label_color,
    );
    let source_response = ui
        .interact(
            source_rect,
            egui::Id::new(("oscillator-source", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to choose modulation targets");
    let _ = crate::editor_modulation::source_handle_for(
        ui,
        state,
        ResolvedRouteSource::Generator(slot.index() as u8),
        &format!("OSC {}", slot.index() + 1),
        &source_response,
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
        with_child(
            ui,
            oscillator_readouts,
            ("resynth-primary-controls", index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_opacity(if enabled { 1.0 } else { 0.38 });
                config_changed |= editor_resynth::draw_resynth_primary_controls(
                    ui,
                    state,
                    slot,
                    module_id,
                    oscillator_readouts,
                    &mut config,
                );
            },
        );
        let secondary_content = egui::Rect::from_min_max(
            egui::pos2(
                unison_panel.left(),
                unison_panel.top() + editor_theme::title_height(ui),
            ),
            unison_panel.max,
        );
        if grain_card {
            config_changed |= editor_resynth::draw_grain_shape_panel(
                ui,
                state,
                slot,
                module_id,
                secondary_content,
            );
        } else if resynth_algorithm.is_some() {
            config_changed |= editor_resynth::draw_algorithm_controls_panel(
                ui,
                state,
                slot,
                module_id,
                secondary_content,
            );
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
        if !is_noise {
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
        }

        if is_noise {
            config_changed |= draw_noise_panel(
                ui,
                state,
                oscillator_plot,
                unison_panel,
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
        if !is_noise {
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
        let seams: &[f32] = if is_noise {
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
        clear_module_bindings(state, &module);
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
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
            ModuleKind::Aux(slot) => state
                .generator_stack
                .set_aux_config(slot, crate::generators::AuxConfig::default()),
        }
    }
}

fn draw_noise_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    plot: egui::Rect,
    controls: egui::Rect,
    module_id: ModuleId,
    slot: OscillatorSlot,
    config: &mut crate::generators::OscillatorConfig,
    enabled: bool,
) -> bool {
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
    let mut left_samples = Vec::with_capacity(count);
    let mut right_samples = Vec::with_capacity(count);
    for _ in 0..count {
        let (left, right) = noise.next(
            440.0 / 48_000.0,
            config.shape / 3.0,
            texture,
            config.phase_warp_amount,
            1,
            &[1.0],
            &[1.0],
        );
        left_samples.push(left);
        right_samples.push(right);
    }
    let peak = left_samples
        .iter()
        .chain(&right_samples)
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        .max(0.001);
    let scale = config.level * 0.46 / peak;
    paint_noise_channel(
        &painter,
        graph,
        &right_samples,
        scale,
        editor_theme::semantic().unison,
        enabled,
    );
    paint_noise_channel(
        &painter,
        graph,
        &left_samples,
        scale,
        editor_theme::semantic().primary,
        enabled,
    );

    let width = controls.width() / 5.0;
    let cells: [egui::Rect; 5] = std::array::from_fn(|index| {
        egui::Rect::from_min_max(
            egui::pos2(controls.left() + width * index as f32, controls.top()),
            egui::pos2(
                if index == 4 {
                    controls.right()
                } else {
                    controls.left() + width * (index + 1) as f32
                },
                controls.bottom(),
            ),
        )
    });
    let mut changed = false;
    for (index, (label, control)) in [
        ("LEVEL", OscillatorControl::Level),
        ("PAN", OscillatorControl::Pan),
        ("TILT", OscillatorControl::Shape),
        ("GAPS", OscillatorControl::PulseWidth),
        ("STEREO", OscillatorControl::PhaseWarpAmount),
    ]
    .into_iter()
    .enumerate()
    {
        with_child(
            ui,
            cells[index],
            ("noise-control", slot.index(), index),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_opacity(if enabled { 1.0 } else { 0.38 });
                changed |= noise_control(
                    ui,
                    state,
                    cells[index],
                    module_id,
                    slot,
                    label,
                    control,
                    config,
                );
            },
        );
    }
    changed
}

fn paint_noise_channel(
    painter: &egui::Painter,
    graph: egui::Rect,
    samples: &[f32],
    scale: f32,
    color: egui::Color32,
    enabled: bool,
) {
    let points = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            egui::pos2(
                egui::lerp(graph.x_range(), index as f32 / (samples.len() - 1) as f32),
                graph.center().y - sample * scale * graph.height(),
            )
        })
        .collect::<Vec<_>>();
    let edge = egui::Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        if enabled { 76 } else { 20 },
    );
    let transparent = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let mut fill = egui::Mesh::default();
    fill.reserve_vertices(points.len().saturating_sub(1) * 4);
    fill.reserve_triangles(points.len().saturating_sub(1) * 2);
    for pair in points.windows(2) {
        let base = fill.vertices.len() as u32;
        fill.colored_vertex(pair[0], edge);
        fill.colored_vertex(pair[1], edge);
        fill.colored_vertex(egui::pos2(pair[1].x, graph.center().y), transparent);
        fill.colored_vertex(egui::pos2(pair[0].x, graph.center().y), transparent);
        fill.add_triangle(base, base + 1, base + 2);
        fill.add_triangle(base, base + 2, base + 3);
    }
    painter.add(fill);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            color.gamma_multiply(if enabled { 0.86 } else { 0.28 }),
        ),
    ));
}

fn noise_control(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    module_id: ModuleId,
    slot: OscillatorSlot,
    label: &str,
    control: OscillatorControl,
    config: &mut crate::generators::OscillatorConfig,
) -> bool {
    let before = *config;
    let defaults = crate::generators::OscillatorConfig::for_engine(OscillatorEngineKind::Noise);
    let value_text = noise_value_text(*config, control);
    let (control_rect, response, mut changed) = match control {
        OscillatorControl::Level => {
            let (rect, response, changed) = super::config_scalar_drag(
                ui,
                &mut config.level,
                0.0..=1.0,
                0.01,
                defaults.level,
                label,
                &value_text,
                rect.size(),
            );
            (rect, response, changed)
        }
        OscillatorControl::Pan => {
            let (rect, response, changed) = super::config_scalar_drag(
                ui,
                &mut config.pan,
                -1.0..=1.0,
                0.02,
                defaults.pan,
                label,
                &value_text,
                rect.size(),
            );
            (rect, response, changed)
        }
        OscillatorControl::Shape => {
            let (rect, response, changed) = super::config_scalar_drag(
                ui,
                &mut config.shape,
                0.0..=3.0,
                0.03,
                defaults.shape,
                label,
                &value_text,
                rect.size(),
            );
            (rect, response, changed)
        }
        OscillatorControl::PulseWidth => {
            let (rect, response, changed) = super::config_scalar_drag(
                ui,
                &mut config.pulse_width,
                0.03..=0.97,
                0.0094,
                defaults.pulse_width,
                label,
                &value_text,
                rect.size(),
            );
            (rect, response, changed)
        }
        OscillatorControl::PhaseWarpAmount => {
            let (rect, response, changed) = super::config_scalar_drag(
                ui,
                &mut config.phase_warp_amount,
                0.0..=1.0,
                0.01,
                defaults.phase_warp_amount,
                label,
                &value_text,
                rect.size(),
            );
            (rect, response, changed)
        }
        _ => unreachable!(),
    };
    let target = ModulationRouteTarget::oscillator(module_id, slot, control);
    if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
        *config = before;
        changed = false;
    }
    let normalized = control.normalized_value(*config);
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
        if control == OscillatorControl::Pan {
            0.5
        } else {
            1.0
        },
    );
    paint_tinted_metric_readout(
        ui,
        control_rect,
        label,
        &noise_value_text(*config, control),
        editor_theme::semantic().primary,
        response.hovered(),
        response.is_pointer_button_down_on() || response.dragged(),
    );
    changed
}

fn noise_value_text(
    config: crate::generators::OscillatorConfig,
    control: OscillatorControl,
) -> String {
    match control {
        OscillatorControl::Level => format!("{:.0}%", config.level * 100.0),
        OscillatorControl::Pan => format_pan(config.pan),
        OscillatorControl::Shape => format!("{:+.0}%", (config.shape / 1.5 - 1.0) * 100.0),
        OscillatorControl::PulseWidth => {
            format!("{:.0}%", (config.pulse_width - 0.03) / 0.94 * 100.0)
        }
        OscillatorControl::PhaseWarpAmount => {
            format!("{:.0}%", config.phase_warp_amount * 100.0)
        }
        _ => unreachable!(),
    }
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
    let rect = egui::Rect::from_center_size(ui.max_rect().center(), egui::Vec2::splat(extent));
    let response = ui.interact(rect, ui.id().with("toggle"), egui::Sense::click());
    let clicked = response.clicked();
    if clicked {
        *enabled = !*enabled;
    }
    let color = if *enabled {
        editor_theme::palette().accent
    } else {
        editor_theme::semantic().grid
    };
    paint_power_icon(ui, rect, color);
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
