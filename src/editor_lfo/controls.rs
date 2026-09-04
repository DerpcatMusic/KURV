use super::*;
use crate::modulators::routing::{ModulationRouteTarget, ResolvedRouteSource};

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
    let cell_height = height / 3.0;
    let color = source_color(index);
    let gate = source_is_gate(state, index);
    let mut gate_config = state.params().modulator_rack.config(index);
    let mut gate_changed = false;
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
        if gate {
            gate_changed |= control_cell(ui, width, cell_height, |ui| {
                dynamic_value(
                    ui,
                    "SWING",
                    &mut gate_config.gate_swing,
                    0.0..=1.0,
                    0.0,
                    crate::editor_controls::ValueSemantic::Percent,
                    color,
                    format_dynamic_percent,
                )
            });
        } else {
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
        }
        control_cell(ui, width, cell_height, |ui| {
            metric_enum_readout(
                ui,
                state,
                params.shape,
                "SHAPE",
                &SHAPES,
                width,
                cell_height,
                color,
            )
        });
    });
    if gate_changed {
        state.params().modulator_rack.set_config(index, gate_config);
    }
}

pub(super) fn draw_keytrack_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let color = source_color(index);
    let cell_height = height / 2.0;
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    ui.vertical(|ui| {
        changed |= control_cell(ui, width, cell_height, |ui| {
            let changed = dynamic_value(
                ui,
                "ROOT",
                &mut config.keytrack_root,
                0.0..=127.0,
                60.0,
                crate::editor_controls::ValueSemantic::Semitones,
                color,
                format_midi_note,
            );
            config.keytrack_root = config.keytrack_root.round();
            changed
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            let mut polarity = u8::from(config.bipolar);
            let changed = dynamic_choice(ui, "POLARITY", &mut polarity, &["UNI", "BI"], 1, color);
            config.bipolar = polarity != 0;
            changed
        });
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

pub(super) fn draw_macro_pack_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    sources: &[usize],
    active: &mut u64,
    selected: &mut usize,
    width: f32,
    height: f32,
) {
    let palette = editor_theme::semantic();
    let columns = 4;
    let rows = MACRO_PACK_CAPACITY.div_ceil(columns);
    let gap = editor_theme::compact_gap(ui);
    let header_height = editor_theme::title_height(ui) * 0.72;
    let header = ui.allocate_space(egui::vec2(width, header_height)).1;
    ui.painter().text(
        header.left_center() + egui::vec2(editor_theme::space::XS, 0.0),
        egui::Align2::LEFT_CENTER,
        "MACROPACK",
        editor_theme::font::label(),
        palette.text,
    );
    ui.painter().text(
        header.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
        egui::Align2::RIGHT_CENTER,
        format!("{} / {MACRO_PACK_CAPACITY}", sources.len()),
        editor_theme::font::caption(),
        palette.text_muted,
    );
    ui.painter().line_segment(
        [header.left_bottom(), header.right_bottom()],
        egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
    );
    let height = (height - header_height - gap).max(1.0);
    let cell_width = (width - gap * (columns - 1) as f32) / columns as f32;
    let cell_height = (height - gap * (rows - 1) as f32) / rows as f32;
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    for row in 0..rows {
        ui.horizontal(|ui| {
            for column in 0..columns {
                let cell = row * columns + column;
                if cell >= MACRO_PACK_CAPACITY {
                    break;
                }
                let (rect, _) = ui
                    .allocate_exact_size(egui::vec2(cell_width, cell_height), egui::Sense::hover());
                if let Some(index) = sources
                    .iter()
                    .copied()
                    .find(|index| macro_pack_cell(*index) == cell)
                {
                    draw_macro_pack_cell(ui, state, index, active, selected, rect);
                } else {
                    draw_empty_macro_pack_cell(ui, state, active, selected, cell, rect);
                }
            }
        });
    }
}

fn draw_macro_pack_cell(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    active: &mut u64,
    selected: &mut usize,
    rect: egui::Rect,
) {
    let palette = editor_theme::semantic();
    let color = source_color(index);
    let mut config = state.params().modulator_rack.config(index);
    let button = config.kind == SourceKind::Button;
    let target = ModulationRouteTarget::macro_pack(index);
    let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
    let mut control_value = host_binding.map_or(config.value, |(_, _, value)| value);
    let action = editor_theme::title_height(ui) * 0.58;
    let handle = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(action));
    let remove = egui::Rect::from_min_size(
        egui::pos2(rect.right() - action, rect.top()),
        egui::Vec2::splat(action),
    );
    let control_response = ui
        .interact(
            rect,
            ui.id().with(("macro-pack-control", index)),
            if button {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        )
        .on_hover_cursor(if button {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::ResizeVertical
        })
        .on_hover_text(if button {
            "Toggle button; right-click for actions"
        } else {
            "Drag macro; Shift is fine; Ctrl snaps; right-click for polarity"
        });
    if button {
        if compact_action_activated(ui, &control_response) {
            control_value = f32::from(control_value < 0.5);
            if let Some((_, param, _)) = host_binding {
                crate::editor_modulation::update_host_automation_gesture(
                    state,
                    param,
                    &control_response,
                    control_value,
                    true,
                );
            } else {
                config.value = control_value;
                state.params().modulator_rack.set_config(index, config);
            }
        }
    } else {
        let changed = crate::editor_controls::update_custom_value_drag(
            ui,
            &control_response,
            &mut control_value,
            0.0..=1.0,
            1.0 / 150.0,
            0.0,
            crate::editor_controls::ValueSemantic::Percent,
        );
        if let Some((_, param, _)) = host_binding {
            crate::editor_modulation::update_host_automation_gesture(
                state,
                param,
                &control_response,
                control_value,
                changed,
            );
        } else if changed {
            config.value = control_value;
            state.params().modulator_rack.set_config(index, config);
        }
    }
    control_response.context_menu(|ui| {
        if !button
            && ui
                .button(if config.bipolar {
                    "Use unipolar"
                } else {
                    "Use bipolar"
                })
                .clicked()
        {
            config.bipolar = !config.bipolar;
            state.params().modulator_rack.set_config(index, config);
            ui.close();
        }
        ui.separator();
        crate::editor_modulation::host_automation_menu(ui, state, target, control_value);
        ui.separator();
        if ui.button("Remove").clicked() {
            state
                .params()
                .host_automation_targets
                .clear_macro_pack_source(index);
            crate::editor_modulation::clear_source(state, (index + 1) as u8);
            *active &= !(1_u64 << index);
            set_source_active(state, index, false, SourceKind::Macro);
            ui.close();
        }
    });
    let source_response = ui
        .interact(
            handle,
            ui.id().with(("macro-pack-source", index)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::editor_modulation::source_handle_for(
        ui,
        state,
        ResolvedRouteSource::Rack(index as u8),
        &format!("{} {}", if button { "BUTTON" } else { "MACRO" }, index + 1),
        &source_response,
    )
    .on_hover_text("Click to choose modulation targets");
    if source_response.clicked() {
        *selected = index;
    }
    let route_value = if button {
        f32::from(control_value >= 0.5)
    } else if config.bipolar {
        control_value.mul_add(2.0, -1.0)
    } else {
        control_value
    };
    let _ = crate::editor_modulation::source_route_display(
        ui,
        state,
        ResolvedRouteSource::Rack(index as u8),
        route_value,
    );
    let remove_response = ui
        .interact(
            remove,
            ui.id().with(("macro-pack-remove", index)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove this MacroPack cell and its routes");
    ui.painter().text(
        remove.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        editor_theme::font::caption(),
        if remove_response.hovered() {
            palette.danger
        } else {
            palette.text_muted
        },
    );
    if compact_action_activated(ui, &remove_response) {
        state
            .params()
            .host_automation_targets
            .clear_macro_pack_source(index);
        crate::editor_modulation::clear_source(state, (index + 1) as u8);
        *active &= !(1_u64 << index);
        set_source_active(state, index, false, SourceKind::Macro);
        return;
    }
    let display = format!("{:.0}", control_value * if button { 1.0 } else { 100.0 });
    let control_rect = egui::Rect::from_center_size(
        rect.center(),
        egui::Vec2::splat(rect.width().min(rect.height()) * 0.9),
    );
    if button {
        paint_macro_button(
            ui,
            control_rect,
            control_value >= 0.5,
            &control_response,
            color,
        );
    } else {
        paint_macro_knob(
            ui,
            control_rect,
            &display,
            control_value,
            &control_response,
            color,
        );
    }
    if !button && config.bipolar {
        ui.painter().text(
            rect.left_bottom() + egui::vec2(editor_theme::space::XXS, -editor_theme::space::XXS),
            egui::Align2::LEFT_BOTTOM,
            "BI",
            editor_theme::font::caption(),
            color,
        );
    }
    crate::editor_modulation::paint_host_automation_badge(ui, state, target, &control_response);
}

fn draw_empty_macro_pack_cell(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    active: &mut u64,
    selected: &mut usize,
    cell: usize,
    rect: egui::Rect,
) {
    let palette = editor_theme::semantic();
    let response = ui
        .interact(
            rect,
            ui.id().with(("macro-pack-add", cell)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a MacroPack control");
    if response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        })
    {
        egui::Popup::toggle_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        editor_theme::font::title(),
        if response.hovered() {
            palette.primary
        } else {
            palette.text_muted
        },
    );
    let mut kind = None;
    egui::Popup::menu(&response).show(|ui| {
        if ui.button("MACRO").clicked() {
            kind = Some(SourceKind::Macro);
            ui.close();
        }
        if ui.button("BUTTON").clicked() {
            kind = Some(SourceKind::Button);
            ui.close();
        }
    });
    let Some(kind) = kind else { return };
    let Some(index) = (LEGACY_MODULATION_SOURCES..MAX_MODULATION_SOURCES)
        .find(|index| *active & (1_u64 << index) == 0 && macro_pack_cell(*index) == cell)
    else {
        return;
    };
    let pack_mask = macro_pack_mask(state, *active);
    let order = state.params().modulator_rack.presentation_order();
    let insertion = order
        .iter()
        .rposition(|slot| pack_mask & (1_u64 << *slot) != 0)
        .map_or(0, |position| position + 1);
    state
        .params()
        .modulator_rack
        .move_source_slot(index, insertion);
    *active |= 1_u64 << index;
    set_source_active(state, index, true, kind);
    *selected = index;
}

const fn macro_pack_cell(index: usize) -> usize {
    (index - LEGACY_MODULATION_SOURCES) % MACRO_PACK_CAPACITY
}

fn paint_macro_button(
    ui: &egui::Ui,
    rect: egui::Rect,
    on: bool,
    response: &egui::Response,
    color: egui::Color32,
) {
    let palette = editor_theme::semantic();
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        if on {
            color.gamma_multiply(0.28)
        } else {
            palette.control
        },
    );
    ui.painter().rect_stroke(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            if on || response.hovered() {
                color
            } else {
                palette.grid
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if on { "ON" } else { "OFF" },
        editor_theme::font::value(),
        if on { color } else { palette.text_muted },
    );
}

fn paint_macro_knob(
    ui: &egui::Ui,
    rect: egui::Rect,
    value: &str,
    normalized: f32,
    response: &egui::Response,
    accent: egui::Color32,
) {
    let palette = editor_theme::semantic();
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.42;
    let start = std::f32::consts::PI * 0.75;
    let span = std::f32::consts::PI * 1.5;
    let arc = |end: f32| {
        (0..=24)
            .map(|index| {
                let angle = egui::lerp(start..=end, index as f32 / 24.0);
                center + egui::vec2(angle.cos(), angle.sin()) * radius
            })
            .collect::<Vec<_>>()
    };
    ui.painter().add(egui::Shape::line(
        arc(start + span),
        egui::Stroke::new(editor_theme::space::SM * 0.34, palette.grid),
    ));
    ui.painter().add(egui::Shape::line(
        arc(start + span * normalized.clamp(0.0, 1.0)),
        egui::Stroke::new(
            editor_theme::space::SM * 0.34,
            accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.82 }),
        ),
    ));
    ui.painter()
        .circle_filled(center, radius * 0.76, palette.control);
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        value,
        editor_theme::font::value(),
        accent,
    );
}

fn compact_action_activated(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.clicked()
        || (response.has_focus()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
            }))
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
    let cell_height = height / 5.0;
    let color = source_color(index);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.add_space(cell_height * 0.5);
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
    let cell_height = height / 3.0;
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
                    match config.rate_mode {
                        1 => crate::editor_controls::ValueSemantic::Time,
                        3 => crate::editor_controls::ValueSemantic::Keytrack,
                        _ => crate::editor_controls::ValueSemantic::Frequency,
                    },
                    color,
                    format,
                )
            }
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            if config.shape == crate::modulators::lfo::LfoShape::Gate as u8 {
                dynamic_value(
                    ui,
                    "SWING",
                    &mut config.gate_swing,
                    0.0..=1.0,
                    0.0,
                    crate::editor_controls::ValueSemantic::Percent,
                    color,
                    format_dynamic_percent,
                )
            } else {
                dynamic_choice(ui, "UNIT", &mut config.rate_mode, &RATE_MODES, 0, color)
            }
        });
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_choice(ui, "SHAPE", &mut config.shape, &SHAPES, 0, color)
        });
    });
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

pub(super) fn draw_lfo_header_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    rect: egui::Rect,
) {
    let color = source_color(index);
    let stops = [0.0, 0.27, 0.72, 1.0];
    let cells: [egui::Rect; 3] = std::array::from_fn(|cell| {
        let left = egui::lerp(rect.left()..=rect.right(), stops[cell]);
        let right = egui::lerp(rect.left()..=rect.right(), stops[cell + 1]);
        egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        )
    });
    for divider in [cells[1].left(), cells[2].left()] {
        ui.painter().line_segment(
            [
                egui::pos2(divider, rect.top() + editor_theme::space::XS),
                egui::pos2(divider, rect.bottom() - editor_theme::space::XS),
            ],
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                editor_theme::on_accent(color).gamma_multiply(0.34),
            ),
        );
    }
    if index < LEGACY_MODULATION_SOURCES {
        let params = lfo_params(index);
        header_param(
            ui,
            state,
            params.phase,
            HeaderLabel::Phase,
            &state.format_param(params.phase),
            cells[0],
            color,
        );
        header_enum(
            ui,
            state,
            params.mode,
            HeaderLabel::Text,
            &MODES,
            cells[1],
            color,
        );
        header_enum(
            ui,
            state,
            params.bipolar,
            HeaderLabel::Polarity,
            &["UNI", "BI"],
            cells[2],
            color,
        );
        return;
    }

    let mut config = state.params().modulator_rack.config(index);
    let mut changed = false;
    let mut polar = u8::from(config.bipolar);
    changed |= header_dynamic_value(
        ui,
        index,
        HeaderLabel::Phase,
        &mut config.phase_offset,
        0.0,
        cells[0],
        color,
    );
    changed |= header_dynamic_choice(
        ui,
        (index, 1),
        HeaderLabel::Text,
        &mut config.mode,
        &MODES,
        0,
        cells[1],
        color,
    );
    changed |= header_dynamic_choice(
        ui,
        (index, 2),
        HeaderLabel::Polarity,
        &mut polar,
        &["UNI", "BI"],
        1,
        cells[2],
        color,
    );
    config.bipolar = polar != 0;
    if changed {
        state.params().modulator_rack.set_config(index, config);
    }
}

#[derive(Clone, Copy)]
enum HeaderLabel {
    Text,
    Phase,
    Polarity,
}

fn header_param(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: HeaderLabel,
    value: &str,
    rect: egui::Rect,
    color: egui::Color32,
) {
    let response = ui
        .interact(
            rect.shrink(editor_theme::space::XXS),
            ui.id().with(("lfo-header-value", u32::from(id))),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let modulation = crate::editor_modulation::owns_gesture(ui, state, id, &response);
    let normalized = if modulation {
        state.get_param(id)
    } else {
        crate::editor_controls::update_parameter_drag(ui, state, id, "PHASE", &response)
    };
    paint_header_control(ui, rect, label, value, color, &response);
    crate::editor_modulation::destination(
        ui,
        state,
        id,
        &response,
        normalized,
        rect,
        crate::editor_modulation::TrackAxis::Horizontal,
    );
}

fn header_enum(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: HeaderLabel,
    values: &[&str],
    rect: egui::Rect,
    color: egui::Color32,
) {
    let last = values.len().saturating_sub(1) as f32;
    let mut current = (state.get_param(id).clamp(0.0, 1.0) * last).round() as usize;
    let response = ui
        .interact(
            rect.shrink(editor_theme::space::XXS),
            ui.id().with(("lfo-header-choice", u32::from(id))),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::editor_shell::register_parameter_hover(ui, id.into(), response.hovered());
    if response.double_clicked() {
        if let Some(info) = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(id))
        {
            let default = info.range.normalize(info.default_plain);
            crate::editor::automate(state, id, default);
            current = (default as f32 * last).round() as usize;
        }
    } else if response.clicked() {
        current = (current + 1) % values.len();
        crate::editor::automate(state, id, current as f64 / f64::from(last.max(1.0)));
    }
    paint_header_control(ui, rect, label, values[current], color, &response);
}

fn header_dynamic_value(
    ui: &mut egui::Ui,
    index: usize,
    label: HeaderLabel,
    value: &mut f32,
    default: f32,
    rect: egui::Rect,
    color: egui::Color32,
) -> bool {
    let response = ui
        .interact(
            rect.shrink(editor_theme::space::XXS),
            ui.id().with(("lfo-header-dynamic-value", index)),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let changed = crate::editor_controls::update_custom_value_drag(
        ui,
        &response,
        value,
        0.0..=1.0,
        1.0 / 150.0,
        default,
        crate::editor_controls::ValueSemantic::Phase,
    );
    paint_header_control(
        ui,
        rect,
        label,
        &format_dynamic_phase(*value),
        color,
        &response,
    );
    changed
}

#[allow(
    clippy::too_many_arguments,
    reason = "compact dynamic enum keeps layout inputs local"
)]
fn header_dynamic_choice(
    ui: &mut egui::Ui,
    id: (usize, usize),
    label: HeaderLabel,
    value: &mut u8,
    choices: &[&str],
    default: u8,
    rect: egui::Rect,
    color: egui::Color32,
) -> bool {
    let response = ui
        .interact(
            rect.shrink(editor_theme::space::XXS),
            ui.id().with(("lfo-header-dynamic-choice", id)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let before = *value;
    if response.double_clicked() {
        *value = default;
    } else if response.clicked() {
        *value = (*value + 1) % choices.len() as u8;
    }
    paint_header_control(
        ui,
        rect,
        label,
        choices[usize::from(*value).min(choices.len() - 1)],
        color,
        &response,
    );
    *value != before
}

fn paint_header_control(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: HeaderLabel,
    value: &str,
    color: egui::Color32,
    response: &egui::Response,
) {
    let painter = ui.painter_at(rect);
    let ink = editor_theme::on_accent(color);
    if response.hovered() || response.is_pointer_button_down_on() {
        painter.rect_filled(
            rect.shrink(editor_theme::space::XXS),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 24),
        );
    }
    let left = rect.left_center() + egui::vec2(editor_theme::space::XS, 0.0);
    let label_color = ink.gamma_multiply(if response.hovered() { 1.0 } else { 0.72 });
    match label {
        HeaderLabel::Text => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                value,
                editor_theme::font::value(),
                ink,
            );
            return;
        }
        HeaderLabel::Phase => paint_phase_icon(&painter, left, label_color),
        HeaderLabel::Polarity => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                value,
                editor_theme::font::value(),
                ink,
            );
            return;
        }
    }
    painter.text(
        rect.right_center() - egui::vec2(editor_theme::space::XS, 0.0),
        egui::Align2::RIGHT_CENTER,
        value,
        editor_theme::font::value(),
        ink,
    );
}

fn paint_phase_icon(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let radius = editor_theme::font::CAPTION_SIZE * 0.42;
    let center = center + egui::vec2(radius, 0.0);
    let stroke = egui::Stroke::new(editor_theme::shape::STROKE, color);
    painter.circle_stroke(center, radius, stroke);
    painter.line_segment(
        [center, center + egui::vec2(radius * 0.72, -radius * 0.72)],
        stroke,
    );
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
    let cell_height = height / 5.0;
    ui.vertical(|ui| {
        ui.add_space(cell_height * 0.5);
        changed |= control_cell(ui, width, cell_height, |ui| {
            dynamic_value(
                ui,
                "ATTACK",
                &mut config.attack,
                0.0..=8.0,
                0.01,
                crate::editor_controls::ValueSemantic::Time,
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
                crate::editor_controls::ValueSemantic::Time,
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
                crate::editor_controls::ValueSemantic::Percent,
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
                crate::editor_controls::ValueSemantic::Time,
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
    semantic: crate::editor_controls::ValueSemantic,
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
            "Drag {}. Hold Shift for fine control or Ctrl for semantic snap; double-click to reset.",
            label.to_lowercase()
        ));
    lock_metric_arrow_focus(ui, &response);
    let before = *value;
    let start = *range.start();
    let end = *range.end();
    if start > 0.0 && end / start >= 100.0 {
        let mut normalized = crate::editor_filter::normalized_log(*value, start, end);
        if crate::editor_controls::update_custom_value_drag(
            ui,
            &response,
            &mut normalized,
            0.0..=1.0,
            0.02 / (end / start).log2(),
            crate::editor_filter::normalized_log(default, start, end),
            crate::editor_controls::ValueSemantic::Continuous,
        ) {
            *value = crate::editor_controls::semantic_snap(
                crate::editor_filter::denormalized_log(normalized, start, end),
                semantic,
                ui.input(|input| input.modifiers.ctrl),
            )
            .clamp(start, end);
        }
    } else {
        crate::editor_controls::update_custom_value_drag(
            ui,
            &response,
            value,
            range.clone(),
            (end - start) / 150.0,
            default,
            semantic,
        );
    }
    if response.has_focus() && !response.is_pointer_button_down_on() {
        let direction = ui.input(|input| {
            i8::from(
                input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::ArrowRight),
            ) - i8::from(
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowLeft),
            )
        });
        if direction != 0 {
            let fine = ui.input(|input| input.modifiers.shift);
            if start > 0.0 && end / start >= 100.0 {
                let octave_step = if fine { 0.01 } else { 0.1 };
                *value = (*value * (f32::from(direction) * octave_step).exp2()).clamp(start, end);
            } else {
                let step = (end - start) * if fine { 0.001 } else { 0.01 };
                *value = (*value + f32::from(direction) * step).clamp(start, end);
            }
            *value = crate::editor_controls::semantic_snap(
                *value,
                semantic,
                ui.input(|input| input.modifiers.ctrl),
            )
            .clamp(start, end);
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

pub(super) fn format_midi_note(note: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let note = note.round().clamp(0.0, 127.0) as i32;
    format!("{}{}", NAMES[note as usize % 12], note / 12 - 2)
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
    let config = state.params().modulator_rack.config(index);
    if config.kind == SourceKind::Macro {
        let value = if config.bipolar {
            config.value.mul_add(2.0, -1.0)
        } else {
            config.value
        };
        return if config.bipolar {
            format!("{:+.0}%", value * 100.0)
        } else {
            format_dynamic_percent(value)
        };
    }
    if config.kind == SourceKind::Button {
        return if config.value >= 0.5 { "ON" } else { "OFF" }.to_owned();
    }
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
        let rate = match config.rate_mode {
            1 => format_dynamic_milliseconds(config.rate_hz),
            2 => SYNC_DIVISIONS[usize::from(config.sync_division).min(SYNC_DIVISIONS.len() - 1)]
                .to_owned(),
            3 => format_dynamic_keytrack(config.rate_hz),
            _ => format_dynamic_rate(config.rate_hz),
        };
        return format!(
            "{} · {} · {}",
            rate,
            MODES[usize::from(config.mode).min(MODES.len() - 1)],
            SHAPES[usize::from(config.shape).min(SHAPES.len() - 1)],
        );
    }
    let params = lfo_params(index);
    let rate = if rate_mode(state, params.rate_mode) == 2 {
        state.format_param(params.sync)
    } else {
        rate_text(state, index, params.rate_mode)
    };
    let mode = (state.get_param(params.mode).clamp(0.0, 1.0) * 3.0).round() as usize;
    let shape = (state.get_param(params.shape).clamp(0.0, 1.0) * 3.0).round() as usize;
    format!(
        "{rate} · {} · {}",
        MODES[mode.min(MODES.len() - 1)],
        SHAPES[shape.min(SHAPES.len() - 1)]
    )
}
