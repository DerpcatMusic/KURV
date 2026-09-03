//! Modulation-source handles and their click-to-assign state transitions.

use super::route_bank::UiRoute;
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

pub(super) fn modulation_handle_hit_radius(unit: f32) -> f32 {
    unit * 0.38
}

pub(super) fn modulation_handle_lane_spacing(unit: f32, reveal: f32) -> f32 {
    modulation_route_marker_radius(unit, reveal) * 2.0 + editor_theme::space::XS
}

pub(super) fn modulation_route_marker_radius(unit: f32, reveal: f32) -> f32 {
    egui::lerp(unit * 0.11..=unit * 0.27, reveal)
}

pub(super) fn paint_modulation_plus(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    color: egui::Color32,
    hovered: bool,
    active: bool,
) {
    painter.circle_filled(center, radius, editor_theme::semantic().well);
    if hovered || active {
        painter.circle_filled(
            center,
            radius,
            egui::Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                if active { 52 } else { 24 },
            ),
        );
    }
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            color.gamma_multiply(if hovered || active { 1.0 } else { 0.78 }),
        ),
    );
    let half = radius * 0.45;
    let stroke = egui::Stroke::new(editor_theme::shape::STROKE, color);
    painter.line_segment(
        [
            center - egui::vec2(half, 0.0),
            center + egui::vec2(half, 0.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center - egui::vec2(0.0, half),
            center + egui::vec2(0.0, half),
        ],
        stroke,
    );
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
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    let color = modulation_source_color(source);
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    let keyboard_activate = response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        });
    if response.clicked() || keyboard_activate {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            direct.armed_source = (direct.armed_source != Some(source)).then_some(source);
            direct.drag_assignment = None;
            direct.hovered_source = None;
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
            direct.inspector_rect = egui::Rect::NOTHING;
        });
    }
    let armed = ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        direct.armed_source == Some(source)
    });
    let chip = response.rect;
    let radius = (modulation_unit(ui) * 0.30)
        .min(chip.width().min(chip.height()) * 0.44)
        .max(editor_theme::shape::FOCUS_STROKE);
    let center = chip.center();
    paint_modulation_plus(
        ui.painter(),
        center,
        radius,
        color,
        response.hovered(),
        armed,
    );
    paint_incoming_depth(ui, state, source, center, radius);

    let pointer = ui.input(|input| input.pointer.latest_pos());
    let hover_rect = response.rect.expand(editor_theme::space::XS);
    let pointer_near_source = pointer.is_some_and(|pointer| hover_rect.contains(pointer));
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
        if direct.dragging_source == Some(source) || direct.armed_source == Some(source) {
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none()
            && direct.armed_source.is_none()
            && (response.hovered() || pointer_near_source)
        {
            direct.hovered_source = Some(source);
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none()
            && direct.armed_source.is_none()
            && direct.hovered_source == Some(source)
        {
            direct.source_rect_frame = frame;
            if direct.amount_drag.is_none()
                && !pointer.is_some_and(|pointer| {
                    response
                        .rect
                        .expand(editor_theme::space::XS)
                        .contains(pointer)
                        || direct
                            .inspector_rect
                            .expand(editor_theme::space::XS)
                            .contains(pointer)
                })
            {
                direct.hovered_source = None;
                direct.source_rect = egui::Rect::NOTHING;
                direct.source_rect_frame = u64::MAX;
            }
        }
    });
    response
        .clone()
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Click to assign {label} to multiple parameters"))
}

fn paint_incoming_depth(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    center: egui::Pos2,
    radius: f32,
) {
    let mut incoming = None;
    for (route, ..) in routes_for_source(ui, state, source).as_slice() {
        let depth_routes =
            routes_for_modular_target(ui, state, ModulationRouteTarget::route_depth(*route));
        for candidate in depth_routes.as_slice() {
            if incoming.is_none_or(|current: UiRoute| candidate.2.abs() > current.2.abs()) {
                incoming = Some(*candidate);
            }
        }
    }
    let Some((_, incoming_source, amount, _)) = incoming else {
        return;
    };
    let ring_radius = radius + editor_theme::space::XXS;
    let color = modulation_source_color(incoming_source);
    ui.painter().circle_stroke(
        center,
        ring_radius,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            color.gamma_multiply(0.20 + amount.abs().clamp(0.0, 1.0) * 0.22),
        ),
    );
    let live = match incoming_source {
        ResolvedRouteSource::Rack(index) => {
            let index = usize::from(index);
            if crate::editor_lfo::source_is_running(state, index) {
                editor_theme::request_display_repaint(ui);
            }
            crate::editor_lfo::source_value_meter(state, index).abs()
        }
        _ => 1.0,
    };
    let sweep = std::f32::consts::TAU * (amount.abs() * live).clamp(0.0, 1.0);
    if sweep <= f32::EPSILON {
        return;
    }
    let points = (0..=24).map(|step| {
        let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / 24.0;
        center + egui::Vec2::angled(angle) * ring_radius
    });
    ui.painter().add(egui::Shape::line(
        points.collect(),
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
}

pub(crate) fn source_drag_active(ui: &egui::Ui) -> bool {
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .is_some_and(|direct| direct.dragging_source.is_some())
    })
}

pub(super) fn assignment_source(ui: &egui::Ui) -> Option<ResolvedRouteSource> {
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .and_then(|direct| direct.dragging_source.or(direct.armed_source))
    })
}

pub(super) fn armed_source(ui: &egui::Ui) -> Option<ResolvedRouteSource> {
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .and_then(|direct| direct.armed_source)
    })
}

pub(crate) fn source_assignment_active(ui: &egui::Ui, source: ResolvedRouteSource) -> bool {
    assignment_source(ui) == Some(source)
}

pub(crate) fn generator_source_drag_active(ui: &egui::Ui) -> bool {
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .is_some_and(|direct| {
                matches!(
                    direct.dragging_source.or(direct.armed_source),
                    Some(ResolvedRouteSource::Generator(_))
                )
            })
    })
}

pub(super) fn clear_source_interaction(direct: &mut DirectModulationState) {
    direct.dragging_source = None;
    direct.armed_source = None;
    direct.drag_assignment = None;
    direct.hovered_source = None;
    direct.source_rect = egui::Rect::NOTHING;
    direct.source_rect_frame = u64::MAX;
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    direct.inspector_rect = egui::Rect::NOTHING;
}
