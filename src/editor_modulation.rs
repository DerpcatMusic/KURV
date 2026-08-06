//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::{KurvParams, P};

const UI_STATE_ID: &str = "kurv-direct-modulation";

const ROUTES: [(P, P, P); 8] = [
    (P::Mod1Source, P::Mod1Target, P::Mod1Amount),
    (P::Mod2Source, P::Mod2Target, P::Mod2Amount),
    (P::Mod3Source, P::Mod3Target, P::Mod3Amount),
    (P::Mod4Source, P::Mod4Target, P::Mod4Amount),
    (P::Mod5Source, P::Mod5Target, P::Mod5Amount),
    (P::Mod6Source, P::Mod6Target, P::Mod6Amount),
    (P::Mod7Source, P::Mod7Target, P::Mod7Amount),
    (P::Mod8Source, P::Mod8Target, P::Mod8Amount),
];

#[derive(Clone, Copy)]
struct DirectModulationState {
    dragging_source: u8,
    hovered_target: u8,
    hovered_rect: egui::Rect,
    amount_drag: Option<AmountDrag>,
}

impl Default for DirectModulationState {
    fn default() -> Self {
        Self {
            dragging_source: 0,
            hovered_target: 0,
            hovered_rect: egui::Rect::NOTHING,
            amount_drag: None,
        }
    }
}

#[derive(Clone, Copy)]
struct AmountDrag {
    route: usize,
    normalized: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum TrackAxis {
    Horizontal,
    Vertical,
}

pub(crate) const fn source_color(index: usize) -> egui::Color32 {
    match index {
        0 => egui::Color32::from_rgb(67, 214, 151),
        1 => egui::Color32::from_rgb(62, 169, 255),
        2 => egui::Color32::from_rgb(198, 112, 255),
        _ => egui::Color32::from_rgb(255, 188, 65),
    }
}

pub(crate) fn source_handle(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) -> egui::Response {
    let source = (index + 1) as u8;
    let color = source_color(index);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(32.0), height.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let active = response.dragged() || response.drag_started();
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        3.0,
        color.gamma_multiply(if active { 0.34 } else { 0.20 }),
    );
    painter.circle_filled(
        egui::pos2(rect.left() + 11.0, rect.center().y),
        if active { 5.0 } else { 4.0 },
        color,
    );
    painter.text(
        egui::pos2(rect.left() + 21.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("DRAG LFO {}", index + 1),
        crate::editor_theme::font::caption(),
        crate::editor_theme::readable_text(color.gamma_multiply(0.72)),
    );

    let id = egui::Id::new(UI_STATE_ID);
    if response.drag_started() {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.dragging_source = source;
            data.insert_temp(id, direct);
        });
    }
    if response.drag_stopped() {
        let direct = ui
            .data(|data| data.get_temp::<DirectModulationState>(id))
            .unwrap_or_default();
        let pointer = ui.input(|input| input.pointer.latest_pos());
        if direct.dragging_source == source
            && direct.hovered_target != 0
            && pointer.is_some_and(|pointer| direct.hovered_rect.contains(pointer))
        {
            assign_route(state, source, direct.hovered_target);
        }
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.dragging_source = 0;
            direct.hovered_target = 0;
            data.insert_temp(id, direct);
        });
    }
    if active {
        if let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) {
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("kurv-modulation-cable"),
                ))
                .line_segment(
                    [rect.center(), pointer],
                    egui::Stroke::new(1.5_f32, color.gamma_multiply(0.88)),
                );
        }
        ui.ctx().request_repaint();
    }
    response.on_hover_text("Drag this colored source onto an oscillator parameter")
}

/// Registers a supported destination, edits route depth with Alt-drag, and
/// paints each route as a thin source-colored range around the base value.
/// Returns true while the gesture owns the control so its base value is not
/// changed at the same time.
pub(crate) fn destination(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
    base: f32,
    track: egui::Rect,
    axis: TrackAxis,
) -> bool {
    let Some(target) = target_for_param(param) else {
        return false;
    };
    let id = egui::Id::new(UI_STATE_ID);
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    if response.contains_pointer() {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.hovered_target = target;
            direct.hovered_rect = response.rect;
            data.insert_temp(id, direct);
        });
    }

    let routes = routes_for_target(state, target);
    paint_routes(ui, track, axis, base, &routes);
    if direct.dragging_source != 0 && response.contains_pointer() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            2.0,
            egui::Stroke::new(
                1.5_f32,
                source_color(usize::from(direct.dragging_source.saturating_sub(1))),
            ),
            egui::StrokeKind::Inside,
        );
    }

    let alt = ui.input(|input| input.modifiers.alt);
    if alt && response.double_clicked() {
        if let Some((route, _, _)) = routes.first().copied() {
            clear_route(state, route);
        }
        return true;
    }
    if alt && response.drag_started() {
        if let Some((route, _, _)) = routes.first().copied() {
            let normalized = state.get_param(ROUTES[route].2);
            state.begin_edit(ROUTES[route].2);
            ui.data_mut(|data| {
                let mut direct = data
                    .get_temp::<DirectModulationState>(id)
                    .unwrap_or_default();
                direct.amount_drag = Some(AmountDrag { route, normalized });
                data.insert_temp(id, direct);
            });
            return true;
        }
    }
    if let Some(mut drag) = direct.amount_drag {
        if response.dragged() {
            drag.normalized = (drag.normalized - response.drag_motion().y / 150.0).clamp(0.0, 1.0);
            state.set_param(ROUTES[drag.route].2, f64::from(drag.normalized));
            ui.data_mut(|data| {
                let mut direct = data
                    .get_temp::<DirectModulationState>(id)
                    .unwrap_or_default();
                direct.amount_drag = Some(drag);
                data.insert_temp(id, direct);
            });
            return true;
        }
        if response.drag_stopped() {
            state.end_edit(ROUTES[drag.route].2);
            ui.data_mut(|data| {
                let mut direct = data
                    .get_temp::<DirectModulationState>(id)
                    .unwrap_or_default();
                direct.amount_drag = None;
                data.insert_temp(id, direct);
            });
            return true;
        }
    }

    if response.hovered() && !routes.is_empty() {
        let chip_size = 11.0;
        for (chip, (_, source, _)) in routes.iter().enumerate() {
            let center = egui::pos2(
                response.rect.right() - chip_size * (chip as f32 + 0.65),
                response.rect.top() + chip_size * 0.65,
            );
            let color = source_color(usize::from(source.saturating_sub(1)));
            ui.painter().circle_filled(center, chip_size * 0.48, color);
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                source,
                egui::FontId::monospace(7.0),
                crate::editor_theme::readable_text(color),
            );
        }
        let mut description = String::new();
        for (_, source, amount) in routes {
            if !description.is_empty() {
                description.push('\n');
            }
            description.push_str(&format!("LFO {source}: {:+.0}%", amount * 100.0));
        }
        response.clone().on_hover_text(format!(
            "{description}\nAlt-drag: depth · Alt-double-click: remove"
        ));
    }
    false
}

pub(crate) fn owns_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
) -> bool {
    let Some(target) = target_for_param(param) else {
        return false;
    };
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID)))
        .unwrap_or_default();
    let route_exists = !routes_for_target(state, target).is_empty();
    (ui.input(|input| input.modifiers.alt)
        && route_exists
        && (response.drag_started()
            || response.dragged()
            || response.drag_stopped()
            || response.double_clicked()))
        || direct.amount_drag.is_some() && (response.dragged() || response.drag_stopped())
}

fn routes_for_target(state: &PluginContext<KurvParams>, target: u8) -> Vec<(usize, u8, f32)> {
    ROUTES
        .iter()
        .enumerate()
        .filter_map(|(index, (source, destination, amount))| {
            let source = discrete_value(state.get_param(*source), 4);
            let destination = discrete_value(state.get_param(*destination), 18);
            (source != 0 && destination == target).then(|| {
                let amount = state.get_param(*amount).mul_add(2.0, -1.0);
                (index, source, amount)
            })
        })
        .collect()
}

fn paint_routes(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    base: f32,
    routes: &[(usize, u8, f32)],
) {
    for (lane, (_, source, amount)) in routes.iter().enumerate() {
        let end = (base + amount).clamp(0.0, 1.0);
        let offset = lane as f32 * 2.0;
        let (start, finish) = match axis {
            TrackAxis::Horizontal => (
                egui::pos2(
                    egui::lerp(track.left()..=track.right(), base),
                    track.top() + offset,
                ),
                egui::pos2(
                    egui::lerp(track.left()..=track.right(), end),
                    track.top() + offset,
                ),
            ),
            TrackAxis::Vertical => (
                egui::pos2(
                    track.right() - offset,
                    egui::lerp(track.bottom()..=track.top(), base),
                ),
                egui::pos2(
                    track.right() - offset,
                    egui::lerp(track.bottom()..=track.top(), end),
                ),
            ),
        };
        let color = source_color(usize::from(source.saturating_sub(1)));
        ui.painter()
            .line_segment([start, finish], egui::Stroke::new(1.5_f32, color));
        ui.painter().circle_filled(finish, 2.0, color);
    }
}

fn assign_route(state: &PluginContext<KurvParams>, source: u8, target: u8) {
    let exact = ROUTES.iter().position(|(src, dst, _)| {
        discrete_value(state.get_param(*src), 4) == source
            && discrete_value(state.get_param(*dst), 18) == target
    });
    let vacant = ROUTES.iter().position(|(src, dst, _)| {
        discrete_value(state.get_param(*src), 4) == 0
            || discrete_value(state.get_param(*dst), 18) == 0
    });
    let Some(route) = exact.or(vacant) else {
        return;
    };
    let (source_param, target_param, amount_param) = ROUTES[route];
    if exact.is_none() {
        state.automate(amount_param, 0.5);
    }
    state.automate(source_param, f64::from(source) / 4.0);
    state.automate(target_param, f64::from(target) / 18.0);
    if exact.is_none() {
        state.automate(amount_param, 0.625);
    }
}

fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
    let (source, target, amount) = ROUTES[route];
    state.automate(amount, 0.5);
    state.automate(target, 0.0);
    state.automate(source, 0.0);
}

fn discrete_value(normalized: f32, maximum: u8) -> u8 {
    (normalized.clamp(0.0, 1.0) * f32::from(maximum)).round() as u8
}

const fn target_for_param(param: P) -> Option<u8> {
    match param {
        P::Osc1Transpose | P::Osc1Cents => Some(1),
        P::Shape => Some(2),
        P::PulseWidth => Some(3),
        P::Osc1WarpAmount => Some(4),
        P::Osc1Level => Some(5),
        P::Osc1Pan => Some(6),
        P::Osc2Transpose | P::Osc2Cents => Some(7),
        P::Osc2Shape => Some(8),
        P::Osc2PulseWidth => Some(9),
        P::Osc2WarpAmount => Some(10),
        P::Osc2Level => Some(11),
        P::Osc2Pan => Some(12),
        P::Osc3Transpose | P::Osc3Cents => Some(13),
        P::Osc3Shape => Some(14),
        P::Osc3PulseWidth => Some(15),
        P::Osc3WarpAmount => Some(16),
        P::Osc3Level => Some(17),
        P::Osc3Pan => Some(18),
        _ => None,
    }
}
