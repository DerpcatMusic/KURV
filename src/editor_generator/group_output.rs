use truce_core::editor::PluginContext;

use crate::editor_controls::fit_font_to_width;
use crate::editor_widgets::with_child;
use crate::generators::{GroupId, GroupOutput, MAX_OUTPUT_PAIRS};
use crate::modulators::routing::{GroupControl, ModulationRouteTarget};
use crate::{KurvParams, editor_theme};

use super::drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
use super::{translucent, weighted_cells};

mod controls;

use controls::{
    GroupEnvelopeCurveDirection, format_gain, format_pan_value, format_percent, format_seconds,
    group_dropdown_readout, group_envelope_control, group_scalar_readout, output_pair_label,
};

#[derive(Default)]
pub(super) struct GroupOutputInteraction {
    pub(super) remove: bool,
    pub(super) toggle_collapse: bool,
    pub(super) reorder: i8,
    pub(super) dragging: bool,
}

pub(super) fn draw_group_output(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    group_index: usize,
    can_remove_group: bool,
    module_count: usize,
    collapsed: bool,
    mut output: GroupOutput,
    group_accent: egui::Color32,
) -> GroupOutputInteraction {
    let palette = editor_theme::semantic();
    let accent = palette.primary;
    let base_output = output;
    apply_host_automation_to_group(ui, state, group_id, &mut output);
    let before = output;
    let inset = rect.shrink2(egui::vec2(
        editor_theme::space::SM.min(rect.width() * 0.008),
        editor_theme::space::XXS,
    ));
    let group_label = if collapsed {
        format!(
            "G{} · {module_count} MODULE{}",
            group_index + 1,
            if module_count == 1 { "" } else { "S" }
        )
    } else {
        format!("G{}", group_index + 1)
    };
    let label_width = ui
        .painter()
        .layout_no_wrap(
            group_label.clone(),
            editor_theme::font::label(),
            palette.text,
        )
        .size()
        .x
        + editor_theme::space::SM;
    let action_count = if can_remove_group { 3.0 } else { 2.0 };
    let action_cell = inset.height().min(inset.width() / action_count);
    let action_width = action_cell * action_count;
    let identity_width = (label_width + action_width).min(inset.width());
    let identity = egui::Rect::from_min_size(inset.min, egui::vec2(identity_width, inset.height()));
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::XS, inset.top()),
        inset.max,
    );
    let remove_width = if can_remove_group { action_cell } else { 0.0 };
    let collapse_rect =
        egui::Rect::from_min_size(identity.min, egui::vec2(action_cell, identity.height()));
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), identity.top()),
        egui::pos2(collapse_rect.right() + action_cell, identity.bottom()),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(drag_rect.right(), identity.top()),
        egui::pos2(identity.right() - remove_width, identity.bottom()),
    );
    let remove_rect =
        egui::Rect::from_min_max(egui::pos2(label_rect.right(), identity.top()), identity.max);
    let collapse_response = ui
        .interact(
            collapse_rect,
            egui::Id::new(("generator-group-collapse", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand this group"
        } else {
            "Collapse this group"
        });
    if collapse_response.has_focus() {
        ui.painter().rect_stroke(
            collapse_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, group_accent),
            egui::StrokeKind::Inside,
        );
    }
    let group_drag = ui
        .interact(
            drag_rect,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move this whole group; arrow keys reorder");
    let reorder = if group_drag.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                group_drag.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        ui.input(|input| {
            i8::from(input.key_pressed(egui::Key::ArrowDown))
                - i8::from(input.key_pressed(egui::Key::ArrowUp))
        })
    } else {
        0
    };
    group_drag.dnd_set_drag_payload(group_id);
    if group_drag.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.painter().rect_filled(
            identity,
            1.0,
            translucent(
                group_accent,
                (identity.height() * 0.10).clamp(0.0, 255.0) as u8,
            ),
        );
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("group", group_id.get()),
                pointer,
                egui::vec2(
                    rect.width() * 0.42,
                    rect.height() * (1.8 + module_count.min(3) as f32),
                ),
                group_accent,
                &format!("GROUP {}", group_index + 1),
                &output_pair_label(output.pair),
                GeneratorDragGhostKind::Group { module_count },
            );
        }
    }
    if group_drag.has_focus() {
        ui.painter().rect_stroke(
            drag_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, group_accent),
            egui::StrokeKind::Inside,
        );
    }
    let marker_side = collapse_rect.height() * 0.14;
    let marker_center = collapse_rect.center();
    let marker_points = if collapsed {
        vec![
            marker_center + egui::vec2(-marker_side * 0.42, -marker_side * 0.72),
            marker_center + egui::vec2(marker_side * 0.42, 0.0),
            marker_center + egui::vec2(-marker_side * 0.42, marker_side * 0.72),
        ]
    } else {
        vec![
            marker_center + egui::vec2(-marker_side * 0.72, -marker_side * 0.42),
            marker_center + egui::vec2(0.0, marker_side * 0.42),
            marker_center + egui::vec2(marker_side * 0.72, -marker_side * 0.42),
        ]
    };
    ui.painter().add(egui::Shape::line(
        marker_points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            if collapse_response.hovered()
                || collapse_response.is_pointer_button_down_on()
                || collapse_response.has_focus()
            {
                palette.text
            } else {
                group_accent
            },
        ),
    ));
    let grip_dot = editor_theme::shape::STROKE;
    let grip_gap = editor_theme::space::XXS;
    let grip_origin = drag_rect.center() - egui::vec2(grip_gap * 0.5, grip_gap);
    let grip_color = if group_drag.dragged() {
        palette.text
    } else if group_drag.hovered() {
        group_accent
    } else {
        palette.text_muted.gamma_multiply(0.56)
    };
    for column in 0..2 {
        for row in 0..3 {
            ui.painter().circle_filled(
                grip_origin + egui::vec2(column as f32 * grip_gap, row as f32 * grip_gap),
                grip_dot,
                grip_color,
            );
        }
    }
    ui.painter().text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        &group_label,
        fit_font_to_width(
            ui.painter(),
            &group_label,
            editor_theme::font::label(),
            label_rect.width() * 0.92,
        ),
        palette.text,
    );
    let remove_confirm_id = egui::Id::new(("generator-group-remove-confirm", group_id.get()));
    let mut remove_armed = module_count > 0
        && ui
            .data(|data| data.get_temp::<bool>(remove_confirm_id))
            .unwrap_or(false);
    let remove_response = can_remove_group.then(|| {
        ui.interact(
            remove_rect,
            egui::Id::new(("generator-group-remove", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if remove_armed {
            "Click again to remove this group and its modules"
        } else {
            "Remove this group and its modules"
        })
    });
    let keyboard_activate = |response: &egui::Response| {
        response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            })
    };
    let toggle_collapse = collapse_response.clicked() || keyboard_activate(&collapse_response);
    let mut remove = false;
    if let Some(response) = &remove_response {
        let activate = response.clicked() || keyboard_activate(response);
        if module_count == 0 {
            remove = activate;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if remove_armed && activate {
            remove = true;
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if activate {
            remove_armed = true;
            ui.data_mut(|data| data.insert_temp(remove_confirm_id, true));
        } else if remove_armed
            && ((!response.hovered() && !response.has_focus())
                || ui.input(|input| input.key_pressed(egui::Key::Escape)))
        {
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        }
        let pressed = response.is_pointer_button_down_on();
        if remove_armed || response.hovered() || pressed {
            ui.painter().rect_filled(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(
                    palette.danger,
                    if pressed {
                        64
                    } else if remove_armed {
                        48
                    } else {
                        28
                    },
                ),
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, palette.danger),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            remove_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            editor_theme::font::label(),
            if remove_armed || pressed || response.hovered() {
                palette.text
            } else {
                palette.text_muted
            },
        );
    }
    if remove_response.is_none() {
        ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
    }

    ui.painter().line_segment(
        [
            egui::pos2(identity.right(), identity.top() + editor_theme::space::XXS),
            egui::pos2(
                identity.right(),
                identity.bottom() - editor_theme::space::XXS,
            ),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.48),
        ),
    );

    let cells = weighted_cells(controls, [0.92, 1.12, 1.12, 0.82, 1.12, 0.78, 0.76, 1.16]);
    group_dropdown_readout(
        ui,
        cells[0],
        ("group-midi-channel", group_id.get()),
        "MIDI IN",
        if output.receive_midi_channel == 0 {
            "OMNI".to_owned()
        } else {
            format!("CH {}", output.receive_midi_channel)
        },
        group_accent,
        |ui| {
            ui.selectable_value(&mut output.receive_midi_channel, 0, "OMNI");
            for channel in 1..=16 {
                ui.selectable_value(
                    &mut output.receive_midi_channel,
                    channel,
                    format!("CH {channel}"),
                );
            }
        },
    );
    let (attack_response, attack_curve_response) = group_envelope_control(
        ui,
        cells[1],
        (group_id.get(), "attack"),
        &mut output.attack,
        &mut output.attack_curve,
        "ATTACK",
        GroupEnvelopeCurveDirection::Rise,
        GroupOutput::default().attack,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Attack,
        &attack_response,
        output,
        output.attack.to_bits() != before.attack.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::AttackCurve,
        &attack_curve_response,
        output,
        output.attack_curve.to_bits() != before.attack_curve.to_bits(),
    );
    let (decay_response, decay_curve_response) = group_envelope_control(
        ui,
        cells[2],
        (group_id.get(), "decay"),
        &mut output.decay,
        &mut output.decay_curve,
        "DECAY",
        GroupEnvelopeCurveDirection::Fall,
        GroupOutput::default().decay,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Decay,
        &decay_response,
        output,
        output.decay.to_bits() != before.decay.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::DecayCurve,
        &decay_curve_response,
        output,
        output.decay_curve.to_bits() != before.decay_curve.to_bits(),
    );
    with_child(
        ui,
        cells[3],
        ("group-output-sustain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (_, response) = group_scalar_readout(
                ui,
                &mut output.sustain,
                "SUSTAIN",
                0.0..=1.0,
                0.01,
                GroupOutput::default().sustain,
                cells[3].size(),
                format_percent,
                accent,
            );
            host_group_control(
                ui,
                state,
                group_id,
                GroupControl::Sustain,
                &response,
                output,
                output.sustain.to_bits() != before.sustain.to_bits(),
            );
        },
    );
    let (release_response, release_curve_response) = group_envelope_control(
        ui,
        cells[4],
        (group_id.get(), "release"),
        &mut output.release,
        &mut output.release_curve,
        "RELEASE",
        GroupEnvelopeCurveDirection::Fall,
        GroupOutput::default().release,
        format_seconds,
        accent,
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::Release,
        &release_response,
        output,
        output.release.to_bits() != before.release.to_bits(),
    );
    host_group_control(
        ui,
        state,
        group_id,
        GroupControl::ReleaseCurve,
        &release_curve_response,
        output,
        output.release_curve.to_bits() != before.release_curve.to_bits(),
    );
    with_child(
        ui,
        cells[5],
        ("group-output-gain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (track, response) = group_scalar_readout(
                ui,
                &mut output.gain,
                "GAIN",
                0.0..=2.0,
                0.01,
                GroupOutput::default().gain,
                cells[5].size(),
                format_gain,
                accent,
            );
            let target = ModulationRouteTarget::group(group_id, GroupControl::Gain);
            let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
            if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
                output.gain = before.gain;
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                output.gain * 0.5,
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                1.0,
            );
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &response,
                    output.gain * 0.5,
                    output.gain.to_bits() != before.gain.to_bits(),
                );
            }
        },
    );
    with_child(
        ui,
        cells[6],
        ("group-output-pan", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (track, response) = group_scalar_readout(
                ui,
                &mut output.pan,
                "PAN",
                -1.0..=1.0,
                0.01,
                GroupOutput::default().pan,
                cells[6].size(),
                format_pan_value,
                accent,
            );
            let target = ModulationRouteTarget::group(group_id, GroupControl::Pan);
            let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
            if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
                output.pan = before.pan;
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                output.pan.mul_add(0.5, 0.5),
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                0.5,
            );
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &response,
                    output.pan.mul_add(0.5, 0.5),
                    output.pan.to_bits() != before.pan.to_bits(),
                );
            }
        },
    );
    let send_response = group_dropdown_readout(
        ui,
        cells[7],
        ("group-output-pair", group_id.get()),
        "SEND TO",
        output_pair_label(output.pair),
        accent,
        |ui| {
            for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                ui.selectable_value(&mut output.pair, pair, output_pair_label(pair));
            }
        },
    );
    if send_response.double_clicked() {
        output.pair = GroupOutput::default().pair;
    }
    restore_host_automated_group_controls(ui, state, group_id, base_output, &mut output);
    if output != base_output {
        state.generator_stack.set_group_output(group_id, output);
    }
    GroupOutputInteraction {
        remove,
        toggle_collapse,
        reorder,
        dragging: group_drag.dragged(),
    }
}

const GROUP_HOST_CONTROLS: [GroupControl; 9] = [
    GroupControl::Gain,
    GroupControl::Pan,
    GroupControl::Attack,
    GroupControl::AttackCurve,
    GroupControl::Decay,
    GroupControl::DecayCurve,
    GroupControl::Sustain,
    GroupControl::Release,
    GroupControl::ReleaseCurve,
];

fn apply_host_automation_to_group(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    output: &mut GroupOutput,
) {
    for control in GROUP_HOST_CONTROLS {
        let target = ModulationRouteTarget::group(group_id, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(ui, state, target)
        {
            control.apply_normalized(output, normalized);
        }
    }
}

fn restore_host_automated_group_controls(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    base: GroupOutput,
    output: &mut GroupOutput,
) {
    for control in GROUP_HOST_CONTROLS {
        let target = ModulationRouteTarget::group(group_id, control);
        if crate::editor_modulation::host_automation_binding(ui, state, target).is_some() {
            control.apply_normalized(output, control.normalized_value(base));
        }
    }
}

fn host_group_control(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    control: GroupControl,
    response: &egui::Response,
    output: GroupOutput,
    changed: bool,
) {
    let target = ModulationRouteTarget::group(group_id, control);
    let normalized = control.normalized_value(output);
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
    crate::editor_modulation::host_automation_destination(ui, state, target, response, normalized);
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, response, normalized, changed,
        );
    }
}
