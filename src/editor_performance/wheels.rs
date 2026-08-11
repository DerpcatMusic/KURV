use std::f32::consts::{FRAC_PI_2, PI};

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{accumulate_drag, pointer_gesture_aborted};
use crate::editor_modulation;
use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, P, editor_theme};

const RIB_COUNT: usize = 7;
const RIB_TRAVEL: f32 = 0.8;
const RIB_EDGE_INSET: f32 = 0.18;
const WHEEL_EDGE_INSET: f32 = 0.08;
const WHEEL_ROUNDING: f32 = 0.28;

#[derive(Clone, Copy)]
enum WheelKind {
    Pitch,
    Mod,
}

impl WheelKind {
    const fn param(self) -> P {
        match self {
            Self::Pitch => P::PitchBend,
            Self::Mod => P::ModWheel,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Pitch => "PITCH",
            Self::Mod => "MOD",
        }
    }

    const fn reset_value(self) -> f64 {
        match self {
            Self::Pitch => 0.5,
            Self::Mod => 0.0,
        }
    }

    const fn springs_to_center(self) -> bool {
        matches!(self, Self::Pitch)
    }

    const fn tooltip(self) -> &'static str {
        match self {
            Self::Pitch => {
                "Pitch bend: drag vertically. Hold Shift for fine control; releases to center."
            }
            Self::Mod => {
                "Mod wheel: drag vertically. Hold Shift for fine control; double-click to reset."
            }
        }
    }
}

pub(super) fn pitch_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    wheel(ui, state, WheelKind::Pitch, width, height)
}

pub(super) fn mod_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    wheel(ui, state, WheelKind::Mod, width, height)
}

fn wheel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    kind: WheelKind,
    width: f32,
    height: f32,
) -> egui::Response {
    let minimum = editor_theme::shape::STROKE;
    let (rect, allocation) = ui.allocate_exact_size(
        egui::vec2(width.max(minimum), height.max(minimum)),
        egui::Sense::hover(),
    );
    let (surface, label_rect) = wheel_layout(rect);
    let response = ui
        .interact(
            surface,
            allocation.id.with("value"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(kind.tooltip());
    update_wheel_value(ui, state, kind, &response);

    let value = state.get_param(kind.param()).clamp(0.0, 1.0);
    let visuals = wheel_visuals(&response);
    paint_wheel(ui, surface, value, kind.springs_to_center(), visuals);
    paint_label(ui, state, allocation.id, kind, label_rect, visuals);
    response
}

fn update_wheel_value(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    kind: WheelKind,
    response: &egui::Response,
) {
    let param = kind.param();
    let edit_id = response.id.with("host-edit");
    if response.drag_started() {
        state.begin_edit(param);
        ui.data_mut(|data| data.insert_temp(edit_id, true));
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let value = accumulate_drag(state.get_param(param), motion);
        state.set_param(param, f64::from(value));
    }

    let edit_active = ui.data(|data| data.get_temp::<bool>(edit_id).unwrap_or(false));
    if edit_active && (response.drag_stopped() || pointer_gesture_aborted(ui)) {
        if kind.springs_to_center() {
            state.set_param(param, kind.reset_value());
        }
        state.end_edit(param);
        ui.data_mut(|data| data.remove::<bool>(edit_id));
    } else if response.double_clicked() {
        state.begin_edit(param);
        state.set_param(param, kind.reset_value());
        state.end_edit(param);
    }
}

fn wheel_visuals(response: &egui::Response) -> editor_theme::ControlVisuals {
    let active = response.is_pointer_button_down_on() || response.dragged();
    editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        active,
        response.has_focus(),
        editor_theme::semantic().primary,
    )
}

fn paint_wheel(
    ui: &egui::Ui,
    surface: egui::Rect,
    value: f32,
    bipolar: bool,
    visuals: editor_theme::ControlVisuals,
) {
    let painter = ui.painter_at(surface);
    let palette = editor_theme::semantic();
    let rounding = surface.width() * WHEEL_ROUNDING;
    painter.rect_filled(surface, rounding, visuals.fill.gamma_multiply(0.72));
    painter.rect_stroke(surface, rounding, visuals.stroke, egui::StrokeKind::Inside);

    let inner = surface.shrink2(egui::vec2(
        surface.width() * RIB_EDGE_INSET,
        surface.height() * WHEEL_EDGE_INSET,
    ));
    let rim = palette.text.gamma_multiply(0.12);
    for x in [inner.left(), inner.right()] {
        painter.line_segment(
            [egui::pos2(x, inner.top()), egui::pos2(x, inner.bottom())],
            egui::Stroke::new(editor_theme::shape::STROKE, rim),
        );
    }

    let phase = value * RIB_TRAVEL * RIB_COUNT as f32;
    for rib in 0..RIB_COUNT {
        let position =
            ((rib as f32 - phase).rem_euclid(RIB_COUNT as f32) / RIB_COUNT as f32).clamp(0.0, 1.0);
        let angle = position * PI - FRAC_PI_2;
        let depth = angle.cos().max(0.0);
        let y = inner.center().y + angle.sin() * inner.height() * 0.5;
        let half_width = inner.width() * (0.32 + depth * 0.18);
        let curve = (1.0 - depth) * editor_theme::shape::STROKE;
        let color = palette.text_muted.gamma_multiply(0.24 + depth * 0.46);
        painter.add(egui::Shape::line(
            vec![
                egui::pos2(inner.center().x - half_width, y + curve),
                egui::pos2(inner.center().x, y),
                egui::pos2(inner.center().x + half_width, y + curve),
            ],
            egui::Stroke::new(editor_theme::shape::STROKE, color),
        ));
    }

    let marker_y = egui::lerp(inner.bottom()..=inner.top(), value);
    let marker_half_width = inner.width() * 0.46;
    painter.line_segment(
        [
            egui::pos2(inner.center().x - marker_half_width, marker_y),
            egui::pos2(inner.center().x + marker_half_width, marker_y),
        ],
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, visuals.indicator),
    );
    if bipolar {
        painter.circle_filled(
            egui::pos2(inner.right(), inner.center().y),
            editor_theme::shape::FOCUS_STROKE,
            palette.text_muted.gamma_multiply(0.58),
        );
    }
}

fn paint_label(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    allocation_id: egui::Id,
    kind: WheelKind,
    label_rect: Option<egui::Rect>,
    visuals: editor_theme::ControlVisuals,
) {
    let Some(label_rect) = label_rect else {
        return;
    };
    let painter = ui.painter_at(label_rect);
    if matches!(kind, WheelKind::Mod) {
        let jack_size = (label_rect.height() * 0.58).max(editor_theme::shape::FOCUS_STROKE * 2.0);
        let jack_rect = egui::Rect::from_center_size(
            egui::pos2(label_rect.left() + jack_size * 0.5, label_rect.center().y),
            egui::vec2(jack_size, jack_size),
        );
        let mut jack_response = ui.interact(
            label_rect,
            allocation_id.with("source"),
            egui::Sense::drag(),
        );
        jack_response.rect = jack_rect;
        let _ = editor_modulation::source_handle_for(
            ui,
            state,
            ResolvedRouteSource::ModWheel,
            "MOD WHEEL",
            &jack_response,
        );
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(
                jack_rect.right() + editor_theme::space::XXS,
                label_rect.top(),
            ),
            label_rect.right_bottom(),
        );
        painter.text(
            text_rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            editor_theme::font::caption(),
            visuals.label,
        );
    } else {
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            editor_theme::font::caption(),
            visuals.label,
        );
    }
}

fn wheel_layout(rect: egui::Rect) -> (egui::Rect, Option<egui::Rect>) {
    let padding = editor_theme::space::XXS;
    let label_height = editor_theme::font::CAPTION_SIZE + padding;
    let show_label =
        rect.height() >= editor_theme::space::LG + label_height + editor_theme::space::XS;
    let label_rect = show_label.then(|| {
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - label_height),
            rect.right_bottom(),
        )
    });
    let wheel_bottom = label_rect
        .map_or(rect.bottom(), |label| label.top() - padding)
        .max(rect.top() + editor_theme::shape::STROKE);
    let wheel_area = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), wheel_bottom));
    let width = (editor_theme::space::LG + editor_theme::space::XS)
        .min(wheel_area.width())
        .max(editor_theme::shape::STROKE);
    let surface = egui::Rect::from_center_size(
        wheel_area.center(),
        egui::vec2(width, wheel_area.height().max(editor_theme::shape::STROKE)),
    );
    (surface, label_rect)
}
