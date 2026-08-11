use super::*;

fn control_cell<R>(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::Min),
        add,
    )
    .inner
}

pub(super) fn draw_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    if index >= LEGACY_MODULATION_SOURCES {
        draw_dynamic_lfo_controls(ui, state, index, width, height);
        return;
    }
    let params = lfo_params(index);
    let cell_height = height / 5.0;
    let color = source_color(index);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        if rate_mode(state, params.rate_mode) == 2 {
            control_cell(ui, width, cell_height, |ui| {
                metric_param_readout(
                    ui,
                    state,
                    params.sync,
                    "RATE",
                    &state.format_param(params.sync),
                    width,
                    cell_height,
                    color,
                )
            });
        } else {
            let text = rate_text(state, index, params.rate_mode);
            control_cell(ui, width, cell_height, |ui| {
                metric_param_readout(
                    ui,
                    state,
                    params.rate,
                    "RATE",
                    &text,
                    width,
                    cell_height,
                    color,
                )
            });
        }
        control_cell(ui, width, cell_height, |ui| {
            metric_enum_readout(
                ui,
                state,
                params.rate_mode,
                "UNIT",
                &RATE_MODES,
                width,
                cell_height,
                color,
            )
        });
        control_cell(ui, width, cell_height, |ui| {
            metric_enum_readout(
                ui,
                state,
                params.mode,
                "MODE",
                &MODES,
                width,
                cell_height,
                color,
            )
        });
        control_cell(ui, width, cell_height, |ui| {
            metric_param_readout(
                ui,
                state,
                params.phase,
                "PHASE",
                &state.format_param(params.phase),
                width,
                cell_height,
                color,
            )
        });
        control_cell(ui, width, cell_height, |ui| {
            metric_enum_readout(
                ui,
                state,
                params.bipolar,
                "POLAR",
                &["UNI", "BI"],
                width,
                cell_height,
                color,
            )
        });
    });
}

pub(super) fn draw_envelope_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    if index >= LEGACY_MODULATION_SOURCES {
        draw_dynamic_envelope_controls(ui, state, index, width, height);
        return;
    }
    let params = envelope_params(index);
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let values = [
        format_envelope_time(attack),
        format_envelope_time(decay),
        format!("{:.0}%", sustain * 100.0),
        format_envelope_time(release),
    ];
    let cell_height = height / 4.0;
    let color = source_color(index);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        for ((param, label), value) in [
            (params.attack, "ATTACK"),
            (params.decay, "DECAY"),
            (params.sustain, "SUSTAIN"),
            (params.release, "RELEASE"),
        ]
        .into_iter()
        .zip(values)
        {
            control_cell(ui, width, cell_height, |ui| {
                metric_param_readout(ui, state, param, label, &value, width, cell_height, color)
            });
        }
    });
}

fn draw_dynamic_lfo_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let color = source_color(index);
    ui.set_min_size(egui::vec2(width, height));
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    let cell_height = height / 5.0;
    ui.vertical(|ui| {
        changed |= control_cell(ui, width, cell_height, |ui| {
            if config.rate_mode == 2 {
                dynamic_choice(
                    ui,
                    "RATE",
                    &mut config.sync_division,
                    &SYNC_DIVISIONS,
                    4,
                    color,
                )
            } else {
                let format: fn(f32) -> String = match config.rate_mode {
                    1 => format_dynamic_milliseconds,
                    3 => format_dynamic_keytrack,
                    _ => format_dynamic_rate,
                };
                dynamic_value(
                    ui,
                    "RATE",
                    &mut config.rate_hz,
                    0.01..=20_000.0,
                    if config.rate_mode == 1 { 1_000.0 } else { 1.0 },
                    color,
                    format,
                )
            }
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_choice(ui, "UNIT", &mut config.rate_mode, &RATE_MODES, 0, color)
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_choice(ui, "MODE", &mut config.mode, &MODES, 0, color)
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "PHASE",
                &mut config.phase_offset,
                0.0..=1.0,
                0.0,
                color,
                format_dynamic_phase,
            )
        });
        let mut polar = u8::from(config.bipolar);
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_choice(ui, "POLAR", &mut polar, &["UNI", "BI"], 1, color)
        });
        config.bipolar = polar != 0;
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

fn draw_dynamic_envelope_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let color = source_color(index);
    ui.set_min_size(egui::vec2(width, height));
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    let cell_height = height / 4.0;
    ui.vertical(|ui| {
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "ATTACK",
                &mut config.attack,
                0.0..=8.0,
                0.01,
                color,
                format_envelope_time,
            )
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "DECAY",
                &mut config.decay,
                0.0..=8.0,
                0.1,
                color,
                format_envelope_time,
            )
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "SUSTAIN",
                &mut config.sustain,
                0.0..=1.0,
                0.8,
                color,
                format_dynamic_percent,
            )
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "RELEASE",
                &mut config.release,
                0.0..=12.0,
                0.2,
                color,
                format_envelope_time,
            )
        });
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

fn dynamic_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    color: egui::Color32,
    format: fn(f32) -> String,
) -> bool {
    let size = egui::vec2(ui.available_width(), ui.available_height().max(1.0));
    let (id, rect) = ui.allocate_space(size);
    let displayed = format(*value);
    let response = ui.interact(
        metric_text_bounds(ui, rect, label, &displayed),
        id.with("metric-value"),
        egui::Sense::click_and_drag(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(format!(
            "Drag {}. Hold Shift for fine control; double-click to reset.",
            label.to_lowercase()
        ));
    lock_metric_arrow_focus(ui, &response);
    let before = *value;
    if response.dragged() {
        let delta = -ui.input(|input| input.pointer.delta().y)
            * if ui.input(|input| input.modifiers.shift) {
                0.1
            } else {
                1.0
            };
        let start = *range.start();
        let end = *range.end();
        if start > 0.0 && end / start >= 100.0 {
            *value = (*value * (delta * 0.02).exp2()).clamp(start, end);
        } else {
            *value = (*value + delta * (end - start) / 150.0).clamp(start, end);
        }
    } else if response.double_clicked() {
        *value = default.clamp(*range.start(), *range.end());
    } else if response.has_focus() && !response.is_pointer_button_down_on() {
        let direction = ui.input(|input| {
            i8::from(
                input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::ArrowRight),
            ) - i8::from(
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowLeft),
            )
        });
        if direction != 0 {
            let start = *range.start();
            let end = *range.end();
            let fine = ui.input(|input| input.modifiers.shift);
            if start > 0.0 && end / start >= 100.0 {
                let octave_step = if fine { 0.01 } else { 0.1 };
                *value = (*value * (f32::from(direction) * octave_step).exp2()).clamp(start, end);
            } else {
                let step = (end - start) * if fine { 0.001 } else { 0.01 };
                *value = (*value + f32::from(direction) * step).clamp(start, end);
            }
        }
    }
    paint_metric_readout_response(ui, rect, label, &format(*value), color, &response);
    value.to_bits() != before.to_bits()
}

fn dynamic_choice(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u8,
    values: &[&str],
    default: u8,
    color: egui::Color32,
) -> bool {
    debug_assert!(!values.is_empty());
    let size = egui::vec2(ui.available_width(), ui.available_height().max(1.0));
    let current = usize::from(*value).min(values.len() - 1);
    let (id, rect) = ui.allocate_space(size);
    let response = ui.interact(
        metric_text_bounds(ui, rect, label, values[current]),
        id.with("metric-value"),
        egui::Sense::click(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{label}: click to cycle; arrow keys adjust; double-click resets"
        ));
    lock_metric_arrow_focus(ui, &response);
    let keyboard_step = response.has_focus().then(|| {
        ui.input(|input| {
            i8::from(
                input.key_pressed(egui::Key::ArrowUp)
                    || input.key_pressed(egui::Key::ArrowRight)
                    || input.key_pressed(egui::Key::Enter)
                    || input.key_pressed(egui::Key::Space),
            ) - i8::from(
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowLeft),
            )
        })
    });
    let changed = if response.double_clicked() {
        *value = usize::from(default).min(values.len() - 1) as u8;
        usize::from(*value) != current
    } else if response.clicked() || keyboard_step == Some(1) {
        *value = ((current + 1) % values.len()) as u8;
        true
    } else if keyboard_step == Some(-1) {
        *value = ((current + values.len() - 1) % values.len()) as u8;
        true
    } else {
        false
    };
    let displayed = usize::from(*value).min(values.len() - 1);
    paint_metric_readout_response(ui, rect, label, values[displayed], color, &response);
    changed
}

fn lock_metric_arrow_focus(ui: &mut egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
}

fn format_dynamic_rate(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.2} kHz", hz / 1_000.0)
    } else if hz >= 100.0 {
        format!("{hz:.0} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

fn format_dynamic_milliseconds(milliseconds: f32) -> String {
    if milliseconds < 10.0 {
        format!("{milliseconds:.2} ms")
    } else {
        format!("{milliseconds:.0} ms")
    }
}

fn format_dynamic_keytrack(value: f32) -> String {
    format!("{:.2}×", crate::modulators::lfo::keytrack_multiplier(value))
}

fn format_dynamic_percent(value: f32) -> String {
    format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0)
}

fn format_dynamic_phase(value: f32) -> String {
    format!("{:.0}°", value.rem_euclid(1.0) * 360.0)
}

fn format_envelope_time(seconds: f32) -> String {
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1_000.0)
    } else {
        format!("{seconds:.2} s")
    }
}

pub(super) fn collapsed_source_summary(
    state: &PluginContext<KurvParams>,
    index: usize,
    envelope: bool,
) -> String {
    if envelope {
        let [attack, decay, sustain, release] = envelope_values(state.params(), index);
        return format!(
            "A {} · D {} · S {:.0}% · R {}",
            format_envelope_time(attack),
            format_envelope_time(decay),
            sustain.clamp(0.0, 1.0) * 100.0,
            format_envelope_time(release),
        );
    }
    if index >= LEGACY_MODULATION_SOURCES {
        let config = state.params().modulator_rack.config(index);
        let rate = match config.rate_mode {
            1 => format_dynamic_milliseconds(config.rate_hz),
            2 => SYNC_DIVISIONS[usize::from(config.sync_division).min(SYNC_DIVISIONS.len() - 1)]
                .to_owned(),
            3 => format_dynamic_keytrack(config.rate_hz),
            _ => format_dynamic_rate(config.rate_hz),
        };
        return format!(
            "{} · {}",
            rate,
            MODES[usize::from(config.mode).min(MODES.len() - 1)],
        );
    }
    let params = lfo_params(index);
    let rate = if rate_mode(state, params.rate_mode) == 2 {
        state.format_param(params.sync)
    } else {
        rate_text(state, index, params.rate_mode)
    };
    let mode = (state.get_param(params.mode).clamp(0.0, 1.0) * 3.0).round() as usize;
    format!("{rate} · {}", MODES[mode.min(MODES.len() - 1)])
}
