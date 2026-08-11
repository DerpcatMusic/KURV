//! Modulation-source handles and their hover/drag state transitions.

use super::*;

pub(crate) fn source_color(index: usize) -> egui::Color32 {
    editor_theme::modulation_source_accent(index)
}

pub(super) fn modulation_source_color(source: ResolvedRouteSource) -> egui::Color32 {
    match source {
        ResolvedRouteSource::Rack(index) => source_color(usize::from(index)),
        ResolvedRouteSource::ModWheel => editor_theme::semantic().primary,
    }
}

pub(super) fn modulation_unit(ui: &egui::Ui) -> f32 {
    editor_theme::title_height(ui)
}

pub(super) fn modulation_knob_radius(unit: f32) -> f32 {
    unit * 0.29
}

pub(super) fn modulation_handle_hit_radius(unit: f32) -> f32 {
    unit * 0.38
}

pub(super) fn modulation_handle_lane_spacing(unit: f32) -> f32 {
    unit * 0.5
}

pub(crate) fn source_handle(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    source_handle_impl(
        ui,
        state,
        ResolvedRouteSource::Rack(index as u8),
        label,
        response,
        true,
    )
}

pub(crate) fn source_handle_for(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    source_handle_impl(ui, state, source, label, response, false)
}

fn source_handle_impl(
    ui: &egui::Ui,
    _state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
    paint_label: bool,
) -> egui::Response {
    let color = modulation_source_color(source);
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    // Arm the dedicated source affordance on press instead of waiting for
    // egui's drag threshold. That keeps scroll areas and quick pointer moves
    // from swallowing the first frame of the gesture.
    if response.is_pointer_button_down_on() || response.drag_started() || response.dragged() {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            if direct.dragging_source.is_none() && !direct.source_drag_cancelled_until_release {
                direct.dragging_source = Some(source);
                direct.hovered_source = Some(source);
                direct.source_rect = response.rect;
                direct.source_rect_frame = frame;
                direct.hovered_target = None;
                direct.hovered_target_valid = false;
                direct.hovered_rect = egui::Rect::NOTHING;
                direct.inspector_rect = egui::Rect::NOTHING;
            }
        });
    }
    let active = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .dragging_source
            == Some(source)
    });
    let palette = editor_theme::semantic();
    let focused = response.has_focus();
    let chip = response.rect.shrink2(if paint_label {
        egui::vec2(editor_theme::shape::STROKE, editor_theme::space::XXS)
    } else {
        egui::Vec2::ZERO
    });
    let radius = if paint_label {
        (chip.height() * 0.16).max(editor_theme::shape::FOCUS_STROKE)
    } else {
        (chip.height() * 0.20).max(editor_theme::shape::FOCUS_STROKE)
    };
    let center = if paint_label {
        egui::pos2(
            chip.left() + editor_theme::space::XS + radius,
            chip.center().y,
        )
    } else {
        chip.center()
    };
    ui.painter().circle_filled(
        center,
        radius,
        color.gamma_multiply(if active || response.hovered() {
            1.0
        } else {
            0.76
        }),
    );
    ui.painter().circle_stroke(
        center,
        radius + editor_theme::shape::FOCUS_STROKE,
        egui::Stroke::new(
            if active {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if active || response.hovered() {
                palette.text
            } else {
                color.gamma_multiply(0.42)
            },
        ),
    );
    if paint_label {
        ui.painter().with_clip_rect(chip).text(
            egui::pos2(center.x + radius + editor_theme::space::XS, chip.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            editor_theme::font::label(),
            if active {
                palette.text
            } else if response.hovered() || focused {
                color
            } else {
                color.gamma_multiply(0.82)
            },
        );
    }

    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        if direct.dragging_source == Some(source) {
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none() && response.hovered() {
            direct.hovered_source = Some(source);
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none() && direct.hovered_source == Some(source) {
            direct.source_rect_frame = frame;
            if direct.amount_drag.is_none()
                && !pointer.is_some_and(|pointer| {
                    response.rect.contains(pointer) || direct.inspector_rect.contains(pointer)
                })
            {
                direct.hovered_source = None;
                direct.source_rect = egui::Rect::NOTHING;
                direct.source_rect_frame = u64::MAX;
            }
        }
    });
    if active {
        editor_theme::request_display_repaint(ui);
    }
    response
        .clone()
        .on_hover_cursor(if active {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        })
        .on_hover_text(format!("Drag {label} onto a highlighted parameter"))
}

pub(crate) fn source_drag_active(ui: &egui::Ui) -> bool {
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .dragging_source
            .is_some()
    })
}

pub(super) fn clear_source_interaction(direct: &mut DirectModulationState) {
    direct.dragging_source = None;
    direct.hovered_source = None;
    direct.source_rect = egui::Rect::NOTHING;
    direct.source_rect_frame = u64::MAX;
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    direct.inspector_rect = egui::Rect::NOTHING;
}
