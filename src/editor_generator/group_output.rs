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
    group_dropdown_readout, group_envelope_control, group_scalar_readout, output_pair_label,
};
use identity::draw_group_identity;

pub(super) use identity::clear_group_name_edit_state;

#[derive(Default)]
pub(super) struct GroupOutputInteraction {
    pub(super) remove: bool,
    pub(super) toggle_collapse: bool,
    pub(super) reorder: i8,
    pub(super) accent: Option<egui::Color32>,
    pub(super) output: Option<GroupOutput>,
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
    output: GroupOutput,
    group_accent: egui::Color32,
) -> GroupOutputInteraction {
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        group_accent.gamma_multiply(0.18),
    );
    let (controls, mut interaction) = draw_group_identity(
        ui,
        state,
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
    if collapsed {
        let routing_width = (editor_theme::title_height(ui) * 9.0).min(controls.width() * 0.44);
        let routing = egui::Rect::from_min_max(
            egui::pos2(controls.right() - routing_width, controls.top()),
            controls.right_bottom(),
        );
        let summary = egui::Rect::from_min_max(
            controls.min,
            egui::pos2(
                (routing.left() - editor_theme::space::XXS).max(controls.left()),
                controls.bottom(),
            ),
        );
        let summary_text = format!(
            "{} {}",
            module_count,
            if module_count == 1 {
                "MODULE"
            } else {
                "MODULES"
            }
        );
        let response = ui
            .interact(
                summary,
                egui::Id::new(("group-collapsed-summary", group_id.get())),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Double-click to expand this group");
        ui.painter().text(
            summary.left_center() + egui::vec2(editor_theme::space::XS, 0.0),
            egui::Align2::LEFT_CENTER,
            summary_text,
            editor_theme::font::caption(),
            if response.hovered() {
                group_accent
            } else {
                editor_theme::semantic().text_muted
            },
        );
        interaction.toggle_collapse |= response.double_clicked();
        let mut routed = interaction.output.unwrap_or(output);
        let before = routed;
        apply_host_automation_to_group(ui, state, group_id, &mut routed);
        draw_routing_button(
            ui,
            state,
            routing,
            group_id,
            &mut routed,
            before,
            group_accent,
        );
        restore_host_automated_group_controls(ui, state, group_id, before, &mut routed);
        if routed != output {
            interaction.output = Some(routed);
        }
    } else if controls.is_positive()
        && let Some(updated) = draw_group_controls(
            ui,
            state,
            controls,
            group_id,
            interaction.output.unwrap_or(output),
            group_accent,
        )
    {
        interaction.output = Some(updated);
    }
    interaction
}

pub(super) fn draw_group_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: crate::generators::GroupId,
    mut output: GroupOutput,
    group_accent: egui::Color32,
) -> Option<GroupOutput> {
    let accent = group_accent;
    let base_output = output;
    apply_host_automation_to_group(ui, state, group_id, &mut output);
    let before = output;
    let cells = weighted_cells(rect, [0.76, 0.76, 0.64, 0.76, 0.88, 0.88, 0.88]);
    let (attack_response, attack_curve_response) = group_envelope_control(
        ui,
        cells[0],
        (group_id.get(), "attack"),
        &mut output.attack,
        &mut output.attack_curve,
        "A",
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
    modulated_group_curve(
        ui,
        state,
        group_id,
        GroupControl::AttackCurve,
        &attack_curve_response,
        &mut output,
        before,
    );
    let (decay_response, decay_curve_response) = group_envelope_control(
        ui,
        cells[1],
        (group_id.get(), "decay"),
        &mut output.decay,
        &mut output.decay_curve,
        "D",
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
    modulated_group_curve(
        ui,
        state,
        group_id,
        GroupControl::DecayCurve,
        &decay_curve_response,
        &mut output,
        before,
    );
    with_child(
        ui,
        cells[2],
        ("group-output-sustain", group_id.get()),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (_, response) = group_scalar_readout(
                ui,
                &mut output.sustain,
                "S",
                0.0..=1.0,
                0.01,
                GroupOutput::default().sustain,
                cells[2].size(),
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
        cells[3],
        (group_id.get(), "release"),
        &mut output.release,
        &mut output.release_curve,
        "R",
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
    modulated_group_curve(
        ui,
        state,
        group_id,
        GroupControl::ReleaseCurve,
        &release_curve_response,
        &mut output,
        before,
    );
    let output_deck = cells[4].union(cells[6]).shrink2(egui::vec2(
        editor_theme::space::XXS,
        editor_theme::space::XXS,
    ));
    ui.painter().rect_filled(
        output_deck,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().masthead_ink.gamma_multiply(0.72),
    );
    for divider in [cells[5].left(), cells[6].left()] {
        ui.painter().line_segment(
            [
                egui::pos2(divider, output_deck.top() + editor_theme::space::XS),
                egui::pos2(divider, output_deck.bottom() - editor_theme::space::XS),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                editor_theme::semantic().grid.gamma_multiply(0.72),
            ),
        );
    }
    draw_gain(ui, state, cells[4], group_id, &mut output, before, accent);
    draw_pan(ui, state, cells[5], group_id, &mut output, before, accent);
    draw_routing_button(ui, state, cells[6], group_id, &mut output, before, accent);
    restore_host_automated_group_controls(ui, state, group_id, base_output, &mut output);
    if envelope_changed(output, base_output) {
        output.envelope_enabled = true;
    }
    (output != base_output).then_some(output)
}

fn draw_envelope_power(
    ui: &egui::Ui,
    rect: egui::Rect,
    group_id: GroupId,
    output: &mut GroupOutput,
    accent: egui::Color32,
) {
    let response = ui
        .interact(
            rect,
            egui::Id::new(("group-power", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if output.enabled {
            "Disable this group"
        } else {
            "Enable this group"
        });
    let keyboard = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    if response.clicked() || keyboard {
        output.enabled = !output.enabled;
    }
    let color = if output.enabled {
        accent
    } else {
        editor_theme::semantic().text_muted
    };
    crate::editor_widgets::paint_power_icon(ui, rect, color);
}

fn draw_gain(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    cell: egui::Rect,
    group_id: GroupId,
    output: &mut GroupOutput,
    before: GroupOutput,
    accent: egui::Color32,
) {
    with_child(
        ui,
        cell,
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
                cell.size(),
                format_gain,
                accent,
            );
            let response = response.on_hover_text(
                "Drag to set gain · Shift for fine control · Right-click to create a 1/16 trance gate",
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
            response.context_menu(|ui| {
                if ui
                    .button("CREATE 1/16 TRANCE GATE")
                    .on_hover_text("Adds a tempo-synced gate source and routes it to this group")
                    .clicked()
                {
                    if crate::editor_modulation::create_trance_gate(state, target).is_some() {
                        output.gain = 0.0;
                        if let Some((_, param, _)) = host_binding {
                            crate::editor::automate(state, param, 0.0);
                        }
                    }
                    ui.close();
                }
            });
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
}

fn draw_pan(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    cell: egui::Rect,
    group_id: GroupId,
    output: &mut GroupOutput,
    before: GroupOutput,
    accent: egui::Color32,
) {
    with_child(
        ui,
        cell,
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
                cell.size(),
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
}

fn draw_routing_button(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: GroupId,
    output: &mut GroupOutput,
    before: GroupOutput,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    let summary = if output.send_pair == 0 {
        output_pair_label(output.pair)
    } else {
        format!("{} + SEND", output_pair_label(output.pair))
    };
    let response = ui
        .interact(
            rect,
            egui::Id::new(("group-routing", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Choose MIDI input and host output. Optional parallel send controls are under Advanced.");
    let popup_id = egui::Popup::default_response_id(&response);
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    crate::editor_controls::paint_metric_readout_response(
        ui,
        rect,
        "ROUTE",
        &summary,
        if response.hovered() || popup_open {
            accent
        } else {
            palette.text
        },
        &response,
    );
    egui::Popup::from_toggle_button_response(&response)
        .kind(egui::PopupKind::Popup)
        .layout(egui::Layout::top_down(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width((rect.width() * 1.72).clamp(250.0, 380.0))
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = editor_theme::space::XXS;
            let row_height = (editor_theme::font::CAPTION_SIZE
                + editor_theme::font::VALUE_SIZE
                + editor_theme::space::XS)
                .max(editor_theme::title_height(ui));
            routing_dropdown_row(
                ui,
                row_height,
                group_id,
                output,
                accent,
                RoutingDropdown::Midi,
            );
            routing_dropdown_row(
                ui,
                row_height,
                group_id,
                output,
                accent,
                RoutingDropdown::Main,
            );

            let advanced_id = ui.id().with(("group-routing-advanced", group_id.get()));
            let mut advanced = ui
                .data(|data| data.get_temp::<bool>(advanced_id))
                .unwrap_or(output.send_pair != 0 || output.send > 0.0 || output.sidechain > 0.0);
            let advanced_response = ui
                .selectable_label(advanced, "ADVANCED PARALLEL SEND")
                .on_hover_text(
                    "Send a separate copy to another Bitwig output for external effects. The Rhythm Sidechain can gate only that copy.",
                );
            if advanced_response.clicked() {
                advanced = !advanced;
                ui.data_mut(|data| data.insert_temp(advanced_id, advanced));
            }
            if advanced {
                routing_dropdown_row(
                    ui,
                    row_height,
                    group_id,
                    output,
                    accent,
                    RoutingDropdown::Aux,
                );
                let (_, levels) =
                    ui.allocate_space(egui::vec2(ui.available_width(), row_height * 1.02));
                let cells = weighted_cells(levels, [1.0, 1.0, 1.0]);
                routing_level_control(
                    ui,
                    state,
                    cells[0],
                    group_id,
                    output,
                    before,
                    GroupControl::Dry,
                    "MAIN LEVEL",
                    "Level sent to Main Out",
                    accent,
                );
                routing_level_control(
                    ui,
                    state,
                    cells[1],
                    group_id,
                    output,
                    before,
                    GroupControl::Send,
                    "AUX SEND",
                    "Parallel copy sent to Send Out",
                    accent,
                );
                routing_level_control(
                    ui,
                    state,
                    cells[2],
                    group_id,
                    output,
                    before,
                    GroupControl::Sidechain,
                    "EXT GATE",
                    "Rhythm Sidechain envelope gates only the parallel send; Main Out is unchanged",
                    accent,
                );
            }
        });
    if output.send_pair != 0 && output.send_pair - 1 == output.pair {
        output.send_pair = 0;
    }
}

#[derive(Clone, Copy)]
enum RoutingDropdown {
    Midi,
    Main,
    Aux,
}

fn routing_dropdown_row(
    ui: &mut egui::Ui,
    height: f32,
    group_id: GroupId,
    output: &mut GroupOutput,
    accent: egui::Color32,
    control: RoutingDropdown,
) {
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
    match control {
        RoutingDropdown::Midi => {
            let response = group_dropdown_readout(
                ui,
                rect,
                ("group-midi-channel", group_id.get()),
                "MIDI IN",
                if output.receive_midi_channel == 0 {
                    "OMNI".to_owned()
                } else {
                    format!("CH {}", output.receive_midi_channel)
                },
                accent,
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
            if response.double_clicked() {
                output.receive_midi_channel = GroupOutput::default().receive_midi_channel;
            }
        }
        RoutingDropdown::Main => {
            let response = group_dropdown_readout(
                ui,
                rect,
                ("group-output-pair", group_id.get()),
                "MAIN OUT",
                output_pair_label(output.pair),
                accent,
                |ui| {
                    for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                        ui.selectable_value(&mut output.pair, pair, output_pair_label(pair));
                    }
                },
            );
            if response.double_clicked() {
                output.pair = GroupOutput::default().pair;
            }
        }
        RoutingDropdown::Aux => {
            let response = group_dropdown_readout(
                ui,
                rect,
                ("group-aux-output-pair", group_id.get()),
                "AUX OUT",
                if output.send_pair == 0 {
                    "OFF".to_owned()
                } else {
                    output_pair_label(output.send_pair - 1)
                },
                accent,
                |ui| {
                    ui.selectable_value(&mut output.send_pair, 0, "OFF");
                    for pair in 0..MAX_OUTPUT_PAIRS as u8 {
                        if pair != output.pair {
                            ui.selectable_value(
                                &mut output.send_pair,
                                pair + 1,
                                output_pair_label(pair),
                            );
                        }
                    }
                },
            );
            if response.double_clicked() {
                output.send_pair = GroupOutput::default().send_pair;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn routing_level_control(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    cell: egui::Rect,
    group_id: GroupId,
    output: &mut GroupOutput,
    before: GroupOutput,
    control: GroupControl,
    label: &str,
    help: &str,
    accent: egui::Color32,
) {
    with_child(
        ui,
        cell,
        ("group-routing-level", group_id.get(), control as u8),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let value = match control {
                GroupControl::Dry => &mut output.dry,
                GroupControl::Send => &mut output.send,
                GroupControl::Sidechain => &mut output.sidechain,
                _ => return,
            };
            let before_value = match control {
                GroupControl::Dry => before.dry,
                GroupControl::Send => before.send,
                GroupControl::Sidechain => before.sidechain,
                _ => return,
            };
            let default = control.normalized_value(GroupOutput::default());
            let (track, response) = group_scalar_readout(
                ui,
                value,
                label,
                0.0..=1.0,
                0.01,
                default,
                cell.size(),
                format_percent,
                accent,
            );
            let response = response.on_hover_text(help);
            let target = ModulationRouteTarget::group(group_id, control);
            let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
            if crate::editor_modulation::modular_owns_gesture(ui, state, target, &response) {
                *value = before_value;
            }
            crate::editor_modulation::modular_destination(
                ui,
                state,
                target,
                &response,
                *value,
                track,
                crate::editor_modulation::TrackAxis::Horizontal,
                1.0,
            );
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &response,
                    *value,
                    value.to_bits() != before_value.to_bits(),
                );
            }
        },
    );
}

fn envelope_changed(output: GroupOutput, base: GroupOutput) -> bool {
    output.attack.to_bits() != base.attack.to_bits()
        || output.attack_curve.to_bits() != base.attack_curve.to_bits()
        || output.decay.to_bits() != base.decay.to_bits()
        || output.decay_curve.to_bits() != base.decay_curve.to_bits()
        || output.sustain.to_bits() != base.sustain.to_bits()
        || output.release.to_bits() != base.release.to_bits()
        || output.release_curve.to_bits() != base.release_curve.to_bits()
}

fn apply_host_automation_to_group(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    output: &mut GroupOutput,
) {
    for control in GroupControl::ALL.iter().copied() {
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
    for control in GroupControl::ALL.iter().copied() {
        let target = ModulationRouteTarget::group(group_id, control);
        if crate::editor_modulation::host_automation_binding(ui, state, target).is_some() {
            control.apply_normalized(output, control.normalized_value(base));
        }
    }
}

fn modulated_group_curve(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    control: GroupControl,
    response: &egui::Response,
    output: &mut GroupOutput,
    before: GroupOutput,
) {
    let target = ModulationRouteTarget::group(group_id, control);
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
    if crate::editor_modulation::modular_owns_gesture(ui, state, target, response) {
        control.apply_normalized(output, control.normalized_value(before));
    }
    let normalized = control.normalized_value(*output);
    let track = egui::Rect::from_min_max(
        egui::pos2(
            response.rect.left(),
            response.rect.bottom() - response.rect.height() * 0.10,
        ),
        response.rect.right_bottom(),
    );
    crate::editor_modulation::modular_destination(
        ui,
        state,
        target,
        response,
        normalized,
        track,
        crate::editor_modulation::TrackAxis::Horizontal,
        1.0,
    );
    if let Some((_, param, _)) = host_binding {
        crate::editor_modulation::update_host_automation_gesture(
            state,
            param,
            response,
            normalized,
            normalized.to_bits() != control.normalized_value(before).to_bits(),
        );
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
