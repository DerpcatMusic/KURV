use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::PluginContext;

use crate::editor_controls::{
    metric_param_readout, metric_text_bounds, paint_metric_readout_response,
};
use crate::{KurvParams, P, editor_theme};

mod wheels;
mod xy_source;

use wheels::{mod_wheel_sized, pitch_wheel_sized};
use xy_source::xy_source_pad;

const FIELD_COLUMNS: f32 = 4.0;

pub(crate) fn performance_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    ui.set_min_size(egui::vec2(width, height));
    let body_size = egui::vec2(
        ui.available_width().max(editor_theme::shape::STROKE),
        ui.available_height().max(editor_theme::shape::STROKE),
    );
    let pod = egui::Rect::from_min_size(ui.cursor().min, body_size);
    ui.painter()
        .rect_filled(pod, 0.0, editor_theme::semantic().masthead_ink);
    ui.painter().rect_stroke(
        pod,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().primary.gamma_multiply(0.28),
        ),
        egui::StrokeKind::Inside,
    );
    ui.allocate_ui_with_layout(
        body_size,
        egui::Layout::left_to_right(egui::Align::Max),
        |ui| {
            let painter = ui.painter();
            let label_width = |label: &str| {
                painter
                    .layout_no_wrap(
                        label.to_owned(),
                        editor_theme::font::caption(),
                        editor_theme::semantic().text_muted,
                    )
                    .size()
                    .x
            };
            let rail_gap = editor_theme::space::XS;
            let section_gap = editor_theme::space::XS;
            let rail_min_width = editor_theme::space::LG + editor_theme::space::SM;
            let desired_rail_width = label_width("PITCH")
                .max(label_width("MOD") + editor_theme::space::SM)
                .max(rail_min_width);
            let rail_width = desired_rail_width
                .max(body_size.y * 0.34)
                .min(body_size.x * 0.13);
            let strip_width = rail_width * 2.0 + rail_gap;
            let xy_width =
                (body_size.y * 0.82).clamp(editor_theme::space::LG * 1.8, body_size.x * 0.22);
            ui.spacing_mut().item_spacing.x = section_gap;
            let strip = ui.allocate_ui_with_layout(
                egui::vec2(strip_width, body_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| wheel_strip(ui, state, body_size.y, rail_width, rail_gap),
            );
            let divider_x = strip.response.rect.right() + section_gap * 0.5;
            let divider_rect = strip
                .response
                .rect
                .shrink2(egui::vec2(0.0, editor_theme::compact_gap(ui)));
            ui.painter().line_segment(
                [
                    egui::pos2(divider_x, divider_rect.top()),
                    egui::pos2(divider_x, divider_rect.bottom()),
                ],
                egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    editor_theme::semantic().grid.gamma_multiply(0.42),
                ),
            );
            xy_source_pad(ui, state, xy_width, body_size.y);
            let fields = egui::vec2(
                (body_size.x - strip_width - xy_width - section_gap * 2.0)
                    .max(editor_theme::shape::STROKE),
                body_size.y,
            );
            ui.allocate_ui_with_layout(fields, egui::Layout::top_down(egui::Align::Min), |ui| {
                performance_field_grid(ui, state, fields.y)
            });
        },
    );
}

fn wheel_strip(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    height: f32,
    width: f32,
    gap: f32,
) {
    ui.spacing_mut().item_spacing.x = gap;
    let range_height = (height * 0.28).max(editor_theme::title_height(ui));
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.y = editor_theme::space::XXS;
            pitch_wheel_sized(
                ui,
                state,
                width,
                (height - range_height - editor_theme::space::XXS).max(1.0),
            );
            pitch_range_field(ui, state, width, range_height);
        },
    );
    mod_wheel_sized(ui, state, width, height);
}

fn performance_field_grid(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32) {
    let width = ui.available_width();
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    let row_height = (height * 0.5).max(editor_theme::shape::STROKE);
    let field_width = (width / FIELD_COLUMNS).max(editor_theme::shape::STROKE);
    let row_size = egui::vec2(width, row_height);
    let grid = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, height));
    let stroke = egui::Stroke::new(
        editor_theme::shape::STROKE,
        editor_theme::semantic().grid.gamma_multiply(0.42),
    );
    for column in 1..FIELD_COLUMNS as usize {
        let x = grid.left() + field_width * column as f32;
        ui.painter().line_segment(
            [egui::pos2(x, grid.top()), egui::pos2(x, grid.bottom())],
            stroke,
        );
    }
    ui.painter()
        .line_segment([grid.left_center(), grid.right_center()], stroke);
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            voice_mode_selector(ui, state, field_width, row_height);
            octave_semitone_field(ui, state, field_width, row_height);
            performance_param_field(
                ui,
                state,
                P::MpeBendRange,
                "MPE RANGE",
                field_width,
                row_height,
            );
            global_tuning_field(ui, state, field_width, row_height);
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            performance_param_field(ui, state, P::GlideTime, "GLIDE", field_width, row_height)
                .on_hover_text("Used by LEGATO mode");
            performance_param_field(
                ui,
                state,
                P::VelocityAmount,
                "VELOCITY",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::PressureAmount,
                "PRESSURE",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::TimbreAmount,
                "TIMBRE",
                field_width,
                row_height,
            );
        },
    );
}

fn octave_semitone_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let total = state.params().octave_shift.value_i32() * 12 + state.params().transpose.value_i32();
    let sign = if total < 0 { '-' } else { '+' };
    let absolute = total.abs();
    let value = format!("{sign}{}.{:02}", absolute / 12, absolute % 12);
    let (id, rect) = ui.allocate_space(egui::vec2(width.max(1.0), height.max(1.0)));
    let response = ui
        .interact(
            metric_text_bounds(ui, rect, "OCT", &value),
            id.with("octave-semitone"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Octaves.semitones. Drag vertically by octaves; hold Shift for semitones.");
    let drag_id = response.id.with("drag");
    if response.drag_started() {
        crate::editor::begin_edit(state, P::OctaveShift);
        crate::editor::begin_edit(state, P::Transpose);
        ui.data_mut(|data| data.insert_temp(drag_id, total as f32));
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let mut raw = ui
            .data(|data| data.get_temp::<f32>(drag_id))
            .unwrap_or(total as f32);
        raw = (raw - response.drag_motion().y / 8.0 * if fine { 1.0 } else { 12.0 })
            .clamp(-60.0, 60.0);
        ui.data_mut(|data| data.insert_temp(drag_id, raw));
        let semitones = if fine {
            raw.round()
        } else {
            (raw / 12.0).round() * 12.0
        } as i32;
        let octave = (semitones / 12).clamp(-4, 4);
        let remainder = (semitones - octave * 12).clamp(-12, 12);
        state.set_param(P::OctaveShift, f64::from(octave + 4) / 8.0);
        state.set_param(P::Transpose, f64::from(remainder + 12) / 24.0);
    } else if response.double_clicked() {
        crate::editor::begin_edit(state, P::OctaveShift);
        crate::editor::begin_edit(state, P::Transpose);
        state.set_param(P::OctaveShift, 0.5);
        state.set_param(P::Transpose, 0.5);
        crate::editor::end_edit(state, P::Transpose);
        crate::editor::end_edit(state, P::OctaveShift);
    }
    if ui.data(|data| data.get_temp::<f32>(drag_id).is_some())
        && (response.drag_stopped() || crate::editor_controls::pointer_gesture_aborted(ui))
    {
        ui.data_mut(|data| data.remove::<f32>(drag_id));
        crate::editor::end_edit(state, P::Transpose);
        crate::editor::end_edit(state, P::OctaveShift);
    }
    paint_metric_readout_response(
        ui,
        rect,
        "OCT",
        &value,
        editor_theme::semantic().primary,
        &response,
    );
}

fn pitch_range_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let down = state.params().pitch_bend_down_range.value_i32();
    let up = state.params().pitch_bend_range.value_i32();
    let value = format!("-{down}  |  +{up}");
    let (id, rect) = ui.allocate_space(egui::vec2(width.max(1.0), height.max(1.0)));
    let interaction = metric_text_bounds(ui, rect, "RANGE", &value);
    let third = interaction.width() / 3.0;
    let left = egui::Rect::from_min_max(
        interaction.min,
        egui::pos2(interaction.left() + third, interaction.bottom()),
    );
    let middle = egui::Rect::from_min_max(
        egui::pos2(left.right(), interaction.top()),
        egui::pos2(interaction.right() - third, interaction.bottom()),
    );
    let right = egui::Rect::from_min_max(
        egui::pos2(middle.right(), interaction.top()),
        interaction.max,
    );
    let left_response = ui
        .interact(left, id.with("down"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Pitch wheel down range");
    let right_response = ui
        .interact(right, id.with("up"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Pitch wheel up range");
    crate::editor_controls::update_parameter_drag(
        ui,
        state,
        P::PitchBendDownRange,
        "PB RANGE",
        &left_response,
    );
    crate::editor_controls::update_parameter_drag(
        ui,
        state,
        P::PitchBendRange,
        "PB RANGE",
        &right_response,
    );
    let middle_response = ui
        .interact(middle, id.with("symmetric"), egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Drag the divider to change both ranges symmetrically");
    let drag_id = middle_response.id.with("drag");
    if middle_response.drag_started() {
        crate::editor::begin_edit(state, P::PitchBendDownRange);
        crate::editor::begin_edit(state, P::PitchBendRange);
        ui.data_mut(|data| data.insert_temp(drag_id, (down as f32, up as f32)));
    }
    if middle_response.dragged() {
        let mut values = ui
            .data(|data| data.get_temp::<(f32, f32)>(drag_id))
            .unwrap_or((down as f32, up as f32));
        let scale = if ui.input(|input| input.modifiers.shift) {
            60.0
        } else {
            6.0
        };
        let delta = -middle_response.drag_motion().y / scale;
        values.0 = (values.0 + delta).clamp(1.0, 96.0);
        values.1 = (values.1 + delta).clamp(1.0, 96.0);
        ui.data_mut(|data| data.insert_temp(drag_id, values));
        state.set_param(
            P::PitchBendDownRange,
            f64::from((values.0.round() - 1.0) / 95.0),
        );
        state.set_param(
            P::PitchBendRange,
            f64::from((values.1.round() - 1.0) / 95.0),
        );
    }
    if ui.data(|data| data.get_temp::<(f32, f32)>(drag_id).is_some())
        && (middle_response.drag_stopped() || crate::editor_controls::pointer_gesture_aborted(ui))
    {
        ui.data_mut(|data| data.remove::<(f32, f32)>(drag_id));
        crate::editor::end_edit(state, P::PitchBendRange);
        crate::editor::end_edit(state, P::PitchBendDownRange);
    }
    let response = if left_response.hovered() || left_response.is_pointer_button_down_on() {
        &left_response
    } else if middle_response.hovered() || middle_response.is_pointer_button_down_on() {
        &middle_response
    } else {
        &right_response
    };
    paint_metric_readout_response(
        ui,
        rect,
        "RANGE",
        &value,
        editor_theme::semantic().primary,
        response,
    );
}

fn global_tuning_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let edit_id = egui::Id::new("global-tuning-entry");
    let editing = ui.data(|data| data.get_temp::<bool>(edit_id).unwrap_or(false));
    let value = state.params().global_tuning_hz.value();
    let value_text = format!("{value:.2} Hz");
    let (allocation_id, rect) = ui.allocate_space(egui::vec2(width.max(1.0), height.max(1.0)));
    let layout = crate::editor_controls::layout_metric_text(
        ui,
        &ui.painter_at(rect),
        rect,
        "TUNING",
        &value_text,
    );
    if editing {
        ui.painter().galley(
            layout.label_position,
            layout.label,
            editor_theme::semantic().primary,
        );
        let mut draft = ui
            .data(|data| data.get_temp::<String>(edit_id.with("draft")))
            .unwrap_or_else(|| format!("{value:.2}"));
        let value_rect = egui::Rect::from_min_size(layout.value_position, layout.value.size())
            .expand2(egui::vec2(
                editor_theme::space::XS,
                editor_theme::space::XXS,
            ))
            .intersect(rect);
        let response = ui.put(
            value_rect,
            egui::TextEdit::singleline(&mut draft)
                .id_salt(edit_id.with("field"))
                .font(editor_theme::font::value())
                .horizontal_align(egui::Align::Center)
                .desired_width(value_rect.width())
                .frame(egui::Frame::NONE),
        );
        if !response.has_focus() {
            response.request_focus();
        }
        let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if cancel {
            ui.data_mut(|data| {
                data.remove::<bool>(edit_id);
                data.remove::<String>(edit_id.with("draft"));
            });
        } else if commit {
            if let Ok(parsed) = draft.trim().trim_end_matches("Hz").trim().parse::<f32>() {
                let value = parsed.clamp(1.0, 10_000.0);
                if let Some(info) = state
                    .params()
                    .param_infos()
                    .into_iter()
                    .find(|info| info.id == u32::from(P::GlobalTuningHz))
                {
                    crate::editor::begin_edit(state, P::GlobalTuningHz);
                    state.set_param(P::GlobalTuningHz, info.range.normalize(f64::from(value)));
                    crate::editor::end_edit(state, P::GlobalTuningHz);
                }
            }
            ui.data_mut(|data| {
                data.remove::<bool>(edit_id);
                data.remove::<String>(edit_id.with("draft"));
            });
        } else {
            ui.data_mut(|data| data.insert_temp(edit_id.with("draft"), draft));
        }
        return;
    }

    let interaction = metric_text_bounds(ui, rect, "TUNING", &value_text);
    let response = ui
        .interact(
            interaction,
            allocation_id.with("metric-value"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let alt_clicked = response.clicked() && ui.input(|input| input.modifiers.alt);
    if alt_clicked {
        ui.data_mut(|data| {
            data.insert_temp(edit_id, true);
            data.insert_temp(edit_id.with("draft"), format!("{value:.2}"));
        });
    } else if response.double_clicked() {
        crate::editor_controls::update_parameter_drag(
            ui,
            state,
            P::GlobalTuningHz,
            "TUNING",
            &response,
        );
    } else {
        crate::editor_controls::update_parameter_drag(
            ui,
            state,
            P::GlobalTuningHz,
            "TUNING",
            &response,
        );
    }
    paint_metric_readout_response(
        ui,
        rect,
        "TUNING",
        &value_text,
        editor_theme::semantic().primary,
        &response,
    );
    response.on_hover_text(
        "Global A4 reference: drag vertically, Alt-click to type, or double-click to reset.",
    );
}

fn performance_param_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    metric_param_readout(
        ui,
        state,
        id,
        label,
        &state.format_param(id),
        width,
        height,
        editor_theme::semantic().primary,
    )
}

fn voice_mode_selector(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let value_text = voice_mode_text(state.params().voice_mode.value_u8());
    let text_bounds = metric_text_bounds(ui, rect, "VOICES", &value_text);
    let combo_rect = egui::Rect::from_center_size(
        text_bounds.center(),
        egui::vec2(
            (text_bounds.width() + editor_theme::title_height(ui) * 0.62).min(rect.width()),
            rect.height(),
        ),
    )
    .intersect(rect);
    let mut combo_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("performance-voice-mode-field")
            .max_rect(combo_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    combo_ui.spacing_mut().interact_size.y = combo_rect.height();
    combo_ui.spacing_mut().button_padding = egui::Vec2::ZERO;
    combo_ui.spacing_mut().button_padding.x = editor_theme::space::XXS;
    let widgets = &mut combo_ui.visuals_mut().widgets;
    for visuals in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        visuals.bg_fill = egui::Color32::TRANSPARENT;
        visuals.weak_bg_fill = egui::Color32::TRANSPARENT;
        visuals.bg_stroke = egui::Stroke::NONE;
    }
    let combo_width = combo_ui.available_width();
    let response = voice_mode_combo(&mut combo_ui, state, combo_width, &value_text);
    crate::editor_shell::register_parameter_hover(ui, P::VoiceMode.into(), response.hovered());
    paint_metric_readout_response(
        ui,
        rect,
        "VOICES",
        &value_text,
        editor_theme::semantic().primary,
        &response,
    );
}

fn voice_mode_text(mode: u8) -> String {
    match mode {
        0 => "MONO".to_owned(),
        1 => "LEGATO".to_owned(),
        voices => voices.to_string(),
    }
}

fn voice_mode_combo(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    selected: &str,
) -> egui::Response {
    const MODES: [u8; 11] = [0, 1, 2, 4, 6, 8, 10, 12, 16, 24, 32];
    let current = state.params().voice_mode.value_u8();
    let response = egui::ComboBox::from_id_salt("performance-voice-mode")
        .selected_text(
            egui::RichText::new(selected)
                .font(editor_theme::font::value())
                .color(egui::Color32::TRANSPARENT),
        )
        .width(width.max(editor_theme::shape::STROKE))
        .show_ui(ui, |ui| {
            for mode in MODES {
                let label = voice_mode_text(mode);
                if ui.selectable_label(current == mode, label).clicked() {
                    crate::editor::begin_edit(state, P::VoiceMode);
                    state.set_param(P::VoiceMode, f64::from(mode) / 32.0);
                    crate::editor::end_edit(state, P::VoiceMode);
                }
            }
        })
        .response;
    response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, response.enabled(), "VOICES");
        info.current_text_value = Some(selected.to_owned());
        info
    });
    response.on_hover_text("Maximum voice count and mono/legato behavior")
}
