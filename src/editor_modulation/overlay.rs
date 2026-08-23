//! Final-pass modulation drag feedback, drop targeting, and route inspection.

mod route_inspector;
mod source_drag;

use super::*;
use source_drag::{paint_drop_targets, paint_source_drag_feedback, update_drop_targets};

fn modulation_source_label(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> String {
    let source = match source {
        ResolvedRouteSource::Rack(source) => source,
        ResolvedRouteSource::ModWheel => return "MOD WHEEL".to_owned(),
        ResolvedRouteSource::XyX => return "XY X".to_owned(),
        ResolvedRouteSource::XyY => return "XY Y".to_owned(),
    };
    let index = usize::from(source);
    let envelope = if index < 8 {
        let kind = match index {
            0 => P::Source1Envelope,
            1 => P::Source2Envelope,
            2 => P::Source3Envelope,
            3 => P::Source4Envelope,
            4 => P::Source5Envelope,
            5 => P::Source6Envelope,
            6 => P::Source7Envelope,
            _ => P::Source8Envelope,
        };
        state.get_param(kind) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    };
    format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1)
}

fn clamp_overlay_rect(rect: egui::Rect, bounds: egui::Rect) -> egui::Rect {
    let max_x = (bounds.right() - rect.width()).max(bounds.left());
    let max_y = (bounds.bottom() - rect.height()).max(bounds.top());
    egui::Rect::from_min_size(
        egui::pos2(
            rect.left().clamp(bounds.left(), max_x),
            rect.top().clamp(bounds.top(), max_y),
        ),
        rect.size(),
    )
}

fn cubic_bezier_points(
    start: egui::Pos2,
    control_a: egui::Pos2,
    control_b: egui::Pos2,
    end: egui::Pos2,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let t = index as f32 / segments.max(1) as f32;
            let inverse = 1.0 - t;
            let point = start.to_vec2() * inverse.powi(3)
                + control_a.to_vec2() * (3.0 * inverse.powi(2) * t)
                + control_b.to_vec2() * (3.0 * inverse * t.powi(2))
                + end.to_vec2() * t.powi(3);
            egui::pos2(point.x, point.y)
        })
        .collect()
}

/// Paints the source-hover route editor after every destination has registered
/// its current frame geometry. Destination controls keep their own base-value
/// hit testing; this final pass owns the modulation handles and popup.
pub(crate) fn cancel_interaction(ui: &egui::Ui, state: &PluginContext<KurvParams>) {
    let primary_down = ui.input(|input| input.pointer.primary_down());
    let id = egui::Id::new(UI_STATE_ID);
    finish_amount_drag(ui, state, id, true);
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        clear_source_interaction(direct);
        direct.source_drag_cancelled_until_release = primary_down;
    });
}

pub(crate) fn draw_overlay(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    let (focused, escape_pressed, primary_down, released, pointer) = ui.input(|input| {
        (
            input.focused,
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_down(),
            input.pointer.button_released(egui::PointerButton::Primary),
            input.pointer.latest_pos(),
        )
    });
    let should_finish_amount_drag = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .is_some()
            && (escape_pressed || !focused || !primary_down)
    });
    if should_finish_amount_drag {
        finish_amount_drag(
            ui,
            state,
            id,
            escape_pressed || !focused || (!primary_down && !released),
        );
    }
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        if !primary_down {
            direct.source_drag_cancelled_until_release = false;
        }
        if direct.dragging_source.is_some() && (escape_pressed || !focused) {
            clear_source_interaction(direct);
            direct.source_drag_cancelled_until_release = primary_down;
        }
        // Once armed, a source drag owns its origin until release. Scrolling or
        // an insertion row can legitimately cull/move the source card mid-drag;
        // treating that as a stale hover cancelled otherwise valid drops.
        if direct.source_rect_frame != frame
            && direct.amount_drag.is_none()
            && direct.dragging_source.is_none()
        {
            clear_source_interaction(direct);
        }
    });
    let mut direct = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .snapshot()
    });
    let mut drag_destinations = None;
    if direct.dragging_source.is_some() {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("kurv-modulation-targets"),
        ));
        let Some(source) = direct.dragging_source else {
            return;
        };
        let availability = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .drag_assignment
        });
        let availability = availability.unwrap_or_else(|| {
            let availability = RouteAssignmentSnapshot::capture(ui, state, source);
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .drag_assignment = Some(availability);
            });
            availability
        });
        let bank_full = availability.bank_full();
        drag_destinations = Some(*availability.destinations());
        let (hovered_valid, drop_targets, feedback) = ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            let hovered_valid = update_drop_targets(&availability, direct, frame, pointer);
            (
                hovered_valid,
                direct.drop_target_snapshot(),
                direct.snapshot(),
            )
        });
        paint_drop_targets(&availability, &drop_targets, &painter);
        if let Some(valid) = hovered_valid {
            ui.ctx().set_cursor_icon(if valid {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::NotAllowed
            });
        }
        direct = feedback;
        paint_source_drag_feedback(ui, state, direct, bank_full);
        if escape_pressed || released || !primary_down {
            let assignment = ui.data_mut(|data| {
                let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
                let assignment = if released
                    && !escape_pressed
                    && direct.hovered_target_valid
                    && pointer.is_some_and(|pointer| direct.hovered_rect.contains(pointer))
                {
                    direct.dragging_source.zip(direct.hovered_target)
                } else {
                    None
                };
                clear_source_interaction(direct);
                assignment
            });
            if let Some((source, target)) = assignment {
                match target {
                    UiDestination::Host(target) => {
                        assign_route(state, source, target);
                    }
                    UiDestination::Modular(target) => {
                        assign_modular_route(state, source, target);
                    }
                }
            }
            direct = ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .snapshot()
            });
        }
    }
    if direct.dragging_source.is_none() {
        route_inspector::paint_persistent_cables(ui, state, id);
    }
    route_inspector::draw(ui, state, id, direct, drag_destinations.as_ref());
}
