use truce_core::editor::PluginContext;

use crate::editor_controls::{
    metric_param_readout, metric_text_bounds, paint_metric_readout_response,
};
use crate::{KurvParams, P, editor_theme};

mod wheels;
mod xy_source;

use wheels::{mod_wheel_sized, pitch_wheel_sized};
use xy_source::xy_source_pad;

const FIELD_ROW_COUNT: f32 = 2.0;

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
            let advanced_width = editor_theme::title_height(ui) * 1.34;
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
                (body_size.x - strip_width - xy_width - advanced_width - section_gap * 3.0)
                    .max(editor_theme::shape::STROKE),
                body_size.y,
            );
            ui.allocate_ui_with_layout(fields, egui::Layout::top_down(egui::Align::Min), |ui| {
                performance_field_grid(ui, state, fields.y)
            });
            performance_advanced_button(ui, state, advanced_width, body_size.y);
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
    pitch_wheel_sized(ui, state, width, height);
    mod_wheel_sized(ui, state, width, height);
}

fn performance_field_grid(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32) {
    let width = ui.available_width();
    let row_gap = editor_theme::compact_gap(ui);
    let field_gap = editor_theme::space::XXS;
    ui.spacing_mut().item_spacing = egui::vec2(field_gap, row_gap);
    let gap_count = FIELD_ROW_COUNT - 1.0;
    let row_height =
        ((height - row_gap * gap_count) / FIELD_ROW_COUNT).max(editor_theme::shape::STROKE);
    let field_width =
        ((width - field_gap * gap_count) / FIELD_ROW_COUNT).max(editor_theme::shape::STROKE);
    let row_size = egui::vec2(width, row_height);
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            voice_mode_selector(ui, state, field_width, row_height);
            performance_param_field(ui, state, P::Transpose, "SEMI", field_width, row_height);
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            performance_param_field(ui, state, P::GlideTime, "GLIDE", field_width, row_height)
                .on_hover_text("Used by LEGATO mode");
            performance_param_field(ui, state, P::OctaveShift, "OCT", field_width, row_height);
        },
    );
}

fn performance_advanced_button(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(editor_theme::shape::STROKE), height),
        egui::Sense::click(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Pitch ranges and expression response");
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            rect.shrink(editor_theme::space::XXS),
            editor_theme::shape::CONTROL_RADIUS,
            editor_theme::semantic().primary.gamma_multiply(0.18),
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::FADERS_HORIZONTAL,
        editor_theme::font::title(),
        editor_theme::semantic().primary,
    );
    egui::Popup::from_toggle_button_response(&response)
        .kind(egui::PopupKind::Popup)
        .layout(egui::Layout::top_down(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width((width * 13.0).clamp(240.0, 340.0))
        .show(|ui| {
            ui.label(
                egui::RichText::new("PERFORMANCE")
                    .font(editor_theme::font::title())
                    .color(editor_theme::semantic().text),
            );
            ui.add_space(editor_theme::compact_gap(ui));
            let gap = editor_theme::space::XS;
            let field_width = (ui.available_width() - gap) * 0.5;
            let field_height = editor_theme::title_height(ui) * 2.2;
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            ui.horizontal(|ui| {
                performance_param_field(
                    ui,
                    state,
                    P::PitchBendRange,
                    "PB RANGE",
                    field_width,
                    field_height,
                );
                performance_param_field(
                    ui,
                    state,
                    P::MpeBendRange,
                    "MPE RANGE",
                    field_width,
                    field_height,
                );
            });
            ui.horizontal(|ui| {
                performance_param_field(
                    ui,
                    state,
                    P::VelocityAmount,
                    "VELOCITY",
                    field_width,
                    field_height,
                );
                performance_param_field(
                    ui,
                    state,
                    P::PressureAmount,
                    "PRESSURE",
                    field_width,
                    field_height,
                );
            });
            performance_param_field(
                ui,
                state,
                P::TimbreAmount,
                "TIMBRE",
                field_width,
                field_height,
            );
        });
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
