//! Modulation-source handles and their hover/drag state transitions.

use super::*;

pub(crate) fn source_color(index: usize) -> egui::Color32 {
    editor_theme::modulation_source_accent(index)
}

pub(super) fn modulation_source_color(source: ResolvedRouteSource) -> egui::Color32 {
    match source {
        ResolvedRouteSource::Rack(index) => source_color(usize::from(index)),
        ResolvedRouteSource::Generator(index) => {
            source_color(crate::modulators::state::MAX_MODULATION_SOURCES + usize::from(index))
        }
        ResolvedRouteSource::ModWheel => editor_theme::semantic().primary,
        ResolvedRouteSource::XyX => source_color(crate::modulators::state::MAX_MODULATION_SOURCES),
        ResolvedRouteSource::XyY => {
            source_color(crate::modulators::state::MAX_MODULATION_SOURCES + 1)
        }
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

pub(crate) fn source_handle_for(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    source_handle_impl(ui, state, source, label, response)
}

fn source_handle_impl(
    ui: &egui::Ui,
    _state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    let color = modulation_source_color(source);
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    // A press selects and visually arms the source, but routing should not own
    // every visible parameter until the pointer has crossed egui's drag
    // threshold. This keeps ordinary source clicks cheap and predictable.
    if response.drag_started() || response.dragged() {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            if direct.dragging_source.is_none() && !direct.source_drag_cancelled_until_release {
                direct.dragging_source = Some(source);
                direct.drag_assignment = None;
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
    let chip = response.rect;
    let radius = (chip.height() * 0.20).max(editor_theme::shape::FOCUS_STROKE);
    let center = chip.center();
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

    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        let source_index = match source {
            ResolvedRouteSource::Rack(index) => usize::from(index),
            ResolvedRouteSource::Generator(index) => {
                crate::modulators::state::MAX_MODULATION_SOURCES + usize::from(index)
            }
            ResolvedRouteSource::XyX => SOURCE_GEOMETRY_COUNT - 3,
            ResolvedRouteSource::XyY => SOURCE_GEOMETRY_COUNT - 2,
            ResolvedRouteSource::ModWheel => SOURCE_GEOMETRY_COUNT - 1,
        };
        direct.source_rects[source_index] = response.rect;
        direct.source_rect_frames[source_index] = frame;
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
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .is_some_and(|direct| direct.dragging_source.is_some())
    })
}

pub(super) fn clear_source_interaction(direct: &mut DirectModulationState) {
    direct.dragging_source = None;
    direct.drag_assignment = None;
    direct.hovered_source = None;
    direct.source_rect = egui::Rect::NOTHING;
    direct.source_rect_frame = u64::MAX;
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    direct.inspector_rect = egui::Rect::NOTHING;
}
