use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_theme;
use crate::editor_widgets::with_child;
use crate::generators::{GroupId, GroupOutput, MAX_OUTPUT_PAIRS};
use crate::modulators::routing::{GroupControl, ModulationRouteTarget};

use super::weighted_cells;

mod controls;
mod identity;

use controls::{
    GroupEnvelopeCurveDirection, format_gain, format_pan_value, format_percent, format_seconds,
    group_dropdown_readout, group_envelope_control, group_envelope_preview, group_scalar_readout,
    output_pair_label,
};
use identity::draw_group_identity;

#[derive(Default)]
pub(super) struct GroupOutputInteraction {
    pub(super) remove: bool,
    pub(super) toggle_collapse: bool,
    pub(super) reorder: i8,
    pub(super) accent_cycle: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_group_header(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    group_index: usize,
    can_remove_group: bool,
    module_count: usize,
    group_size: egui::Vec2,
    collapsed: bool,
    mut output: GroupOutput,
    group_accent: egui::Color32,
) -> GroupOutputInteraction {
    let base_output = output;
    let (controls, interaction) = draw_group_identity(
        ui,
        rect,
        group_id,
        group_index,
        can_remove_group,
        module_count,
        group_size,
        collapsed,
        output,
        group_accent,
    );
    let midi_width = (editor_theme::title_height(ui) * 8.0).min(controls.width());
    let midi = egui::Rect::from_min_max(
        egui::pos2(controls.right() - midi_width, controls.top()),
        controls.right_bottom(),
    );
    let midi_response = group_dropdown_readout(
        ui,
        midi,
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
    if midi_response.double_clicked() {
        output.receive_midi_channel = GroupOutput::default().receive_midi_channel;
    }
    if output != base_output {
        state.generator_stack.set_group_output(group_id, output);
    }
    interaction
}

pub(super) fn draw_group_output(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    mut output: GroupOutput,
    group_accent: egui::Color32,
) {
    let accent = group_accent;
    let base_output = output;
    apply_host_automation_to_group(ui, state, group_id, &mut output);
    let before = output;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + editor_theme::space::SM, rect.top()),
            egui::pos2(rect.right() - editor_theme::space::SM, rect.top()),
        ],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().grid.gamma_multiply(0.42),
        ),
    );
    let cells = weighted_cells(
        rect.shrink2(egui::vec2(editor_theme::space::SM, 0.0)),
        [0.72, 1.0, 1.0, 0.78, 1.0, 0.72, 0.72, 1.0],
    );
    group_envelope_preview(ui, cells[0], output, accent);
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
