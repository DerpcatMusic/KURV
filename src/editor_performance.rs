use truce_core::editor::PluginContext;

use crate::editor_controls::{
    fit_font_to_width, metric_param_readout, mod_wheel_sized, pitch_wheel_sized,
};
use crate::{KurvParams, P, editor_theme};

pub(crate) fn performance_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    ui.set_min_size(egui::vec2(width, height));
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, editor_theme::semantic().surface);
    let gap = editor_theme::compact_gap(ui);
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    performance_heading(ui, "PERFORMANCE");

    let body_size = egui::vec2(
        ui.available_width().max(1.0),
        (ui.available_height() - gap).max(1.0),
    );
    ui.allocate_ui_with_layout(
        body_size,
        egui::Layout::left_to_right(egui::Align::Center),
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
            let rail_gap = editor_theme::space::SM;
            let section_gap = gap.max(editor_theme::shape::STROKE);
            let rail_min_width = editor_theme::space::LG + editor_theme::space::MD;
            let desired_rail_width = label_width("PITCH")
                .max(label_width("MOD") + editor_theme::space::SM)
                .max(rail_min_width);
            let column_share = ((body_size.x - section_gap - rail_gap).max(1.0) / 5.0)
                .max(editor_theme::shape::STROKE);
            let rail_width = desired_rail_width.min(column_share);
            let strip_width = rail_width * 2.0 + rail_gap;
            ui.spacing_mut().item_spacing.x = section_gap;
            ui.allocate_ui_with_layout(
                egui::vec2(strip_width, body_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| wheel_strip(ui, state, body_size.y, rail_width),
            );
            let fields = egui::vec2(
                (body_size.x - strip_width - section_gap).max(1.0),
                body_size.y,
            );
            ui.allocate_ui_with_layout(fields, egui::Layout::top_down(egui::Align::Min), |ui| {
                performance_field_grid(ui, state, fields.y)
            });
        },
    );
}

fn wheel_strip(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32, width: f32) {
    let gap = editor_theme::space::SM;
    ui.spacing_mut().item_spacing.x = gap;
    pitch_wheel_sized(ui, state, width, height);
    mod_wheel_sized(ui, state, width, height);
}

fn performance_heading(ui: &mut egui::Ui, label: &str) {
    let heading_height = editor_theme::font::LABEL_SIZE + editor_theme::compact_gap(ui) * 2.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), heading_height),
        egui::Sense::hover(),
    );
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
}

fn performance_field_grid(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32) {
    let gap = editor_theme::compact_gap(ui);
    let width = ui.available_width();
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    let row_height = ((height - gap * 2.0).max(0.0) / 3.0).max(editor_theme::shape::STROKE);
    let field_width = ((width - gap * 2.0).max(0.0) / 3.0).max(editor_theme::shape::STROKE);
    let row_size = egui::vec2(width, row_height);
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            voice_mode_selector(ui, state, field_width, row_height);
            performance_param_field(ui, state, P::Transpose, "SEMI", field_width, row_height);
            performance_param_field(ui, state, P::OctaveShift, "OCT", field_width, row_height);
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            performance_param_field(ui, state, P::GlideTime, "GLIDE", field_width, row_height)
                .on_hover_text("Used by LEGATO mode");
            performance_param_field(
                ui,
                state,
                P::PitchBendRange,
                "PB RANGE",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::MpeBendRange,
                "MPE RANGE",
                field_width,
                row_height,
            );
        },
    );
    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            performance_param_field(
                ui,
                state,
                P::VelocityAmount,
                "VEL AMT",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::PressureAmount,
                "PRESS AMT",
                field_width,
                row_height,
            );
            performance_param_field(
                ui,
                state,
                P::TimbreAmount,
                "TIMBRE AMT",
                field_width,
                row_height,
            );
        },
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
    let split_height = editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui)
        + editor_theme::shape::STROKE;
    let split_label = rect.height() >= split_height;
    let palette = editor_theme::semantic();
    let mut combo_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("performance-voice-mode-field")
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    combo_ui.spacing_mut().interact_size.y = rect.height();
    combo_ui.spacing_mut().button_padding = egui::vec2(editor_theme::space::XXS, 0.0);
    combo_ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.hovered.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.active.weak_bg_fill = egui::Color32::TRANSPARENT;
    combo_ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
    combo_ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
    combo_ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
    let value_text = voice_mode_text(state.params().voice_mode.value_u8());
    let combo_width = combo_ui.available_width();
    let response = voice_mode_combo(&mut combo_ui, state, combo_width, &value_text);
    let visuals = editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        response.is_pointer_button_down_on(),
        response.has_focus(),
        palette.primary,
    );
    let painter = ui.painter_at(rect);
    let text_width = (rect.width() - editor_theme::space::SM * 2.0).max(1.0);
    if split_label {
        let label_font = fit_font_to_width(
            &painter,
            "VOICES",
            editor_theme::font::caption(),
            text_width,
        );
        let value_font = fit_font_to_width(
            &painter,
            &value_text,
            editor_theme::font::value(),
            text_width,
        );
        let label = painter.layout_no_wrap("VOICES".to_owned(), label_font, visuals.label);
        let value = painter.layout_no_wrap(value_text, value_font, visuals.value);
        let gap = editor_theme::compact_gap(ui);
        let content_height = label.size().y + gap + value.size().y;
        let top = rect.center().y - content_height * 0.5;
        painter.galley(
            egui::pos2(rect.center().x - label.size().x * 0.5, top),
            label,
            visuals.label,
        );
        painter.galley(
            egui::pos2(
                rect.center().x - value.size().x * 0.5,
                top + editor_theme::font::CAPTION_SIZE + gap,
            ),
            value,
            visuals.value,
        );
    } else {
        let text = format!("VOICES {value_text}");
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            &text,
            fit_font_to_width(&painter, &text, editor_theme::font::value(), text_width),
            visuals.value,
        );
    }
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
                    state.begin_edit(P::VoiceMode);
                    state.set_param(P::VoiceMode, f64::from(mode) / 32.0);
                    state.end_edit(P::VoiceMode);
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
