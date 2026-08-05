//! Parameter-bound controls shared by the KURV editor panels.

use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::{KurvParams, P, editor_theme};

#[derive(Clone, Copy)]
struct KnobDrag {
    #[cfg(debug_assertions)]
    start: f32,
    value: f32,
    delta_y: f32,
    frames: u32,
}

#[derive(Clone, Copy)]
enum DragAxis {
    Horizontal,
    Vertical,
}

pub(crate) fn param_knob<PARAMS: Params + ?Sized>(
    ui: &mut egui::Ui,
    state: &PluginContext<PARAMS>,
    id: impl Into<u32>,
    label: &str,
) -> egui::Response {
    const START: f32 = std::f32::consts::FRAC_PI_4 * 3.0;
    const SWEEP: f32 = std::f32::consts::FRAC_PI_2 * 3.0;

    let id = id.into();
    let size = editor_theme::knob_size(ui);
    let height = size + editor_theme::space::MD;
    let radius = size * 0.32;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size, height), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let value = update_parameter_drag(ui, state, id, label, &response, DragAxis::Vertical);

    let painter = ui.painter_at(rect);
    let center = egui::pos2(
        rect.center().x,
        rect.top() + radius + editor_theme::space::XS,
    );
    let interactive = response.hovered() || response.dragged() || response.has_focus();
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id.with("outer_arc"), interactive, 0.18);
    let face = editor_theme::semantic().surface;
    let rim = editor_theme::semantic().grid;

    painter.circle_filled(
        center + egui::vec2(0.0, 2.0),
        radius + 1.0,
        egui::Color32::from_black_alpha(if interactive { 92 } else { 68 }),
    );
    painter.circle_filled(center, radius, rim);
    painter.circle_filled(center, radius - 2.0, face);
    painter.circle_stroke(
        center,
        radius - 1.5,
        egui::Stroke::new(
            if response.has_focus() {
                1.5_f32
            } else {
                1.0_f32
            },
            if interactive {
                editor_theme::palette().accent
            } else {
                rim
            },
        ),
    );

    let arc_radius = radius + editor_theme::space::XS;
    painter.add(egui::Shape::line(
        arc_points(center, arc_radius, START, SWEEP, 64),
        egui::Stroke::new(1.0 + hover * 0.35, egui::Color32::from_gray(74)),
    ));
    if value > 0.001 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the arc uses at most 64 segments"
        )]
        let segments = ((value * 64.0) as usize).max(2);
        let width = 1.35 + hover * 1.35;
        painter.add(egui::Shape::line(
            arc_points(
                center,
                arc_radius + (width - 1.35) * 0.5,
                START,
                SWEEP * value,
                segments,
            ),
            egui::Stroke::new(width, editor_theme::palette().accent),
        ));
    }
    let direction = egui::Vec2::angled(START + SWEEP * value);
    painter.line_segment(
        [
            center + direction * radius * 0.62,
            center + direction * radius * 0.86,
        ],
        egui::Stroke::new(1.75_f32, ui.visuals().text_color()),
    );
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        state.format_param(id),
        editor_theme::font::value(),
        ui.visuals().text_color(),
    );
    painter.text(
        egui::pos2(rect.center().x, center.y + radius + editor_theme::space::XS),
        egui::Align2::CENTER_TOP,
        label,
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    response
}

/// A spring-loaded bipolar wheel: drag vertically, release to return to center.
pub(crate) fn pitch_wheel(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) -> egui::Response {
    let width = editor_theme::metrics(ui).points(2.5).clamp(34.0, 40.0);
    let height = editor_theme::control_height(ui);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.center().x - 7.0, rect.top() + 3.0),
        egui::pos2(rect.center().x + 7.0, rect.bottom() - 12.0),
    );

    if response.drag_started() {
        state.begin_edit(P::PitchBend);
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let normalized = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
        state.set_param(P::PitchBend, f64::from(normalized));
    }
    if response.drag_stopped() {
        state.set_param(P::PitchBend, 0.5);
        state.end_edit(P::PitchBend);
    } else if response.double_clicked() {
        state.begin_edit(P::PitchBend);
        state.set_param(P::PitchBend, 0.5);
        state.end_edit(P::PitchBend);
    }

    let value = state.get_param(P::PitchBend);
    let handle_y = egui::lerp(track.bottom()..=track.top(), value);
    let center_y = track.center().y;
    let painter = ui.painter_at(rect);
    painter.rect_filled(track, 5.0, editor_theme::semantic().control);
    painter.rect_stroke(
        track,
        5.0,
        egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_x_y_ranges(
            (track.left() + 2.0)..=(track.right() - 2.0),
            handle_y.min(center_y)..=handle_y.max(center_y),
        ),
        2.0,
        editor_theme::palette().accent.linear_multiply(0.7),
    );
    painter.line_segment(
        [
            egui::pos2(track.left() + 2.0, center_y),
            egui::pos2(track.right() - 2.0, center_y),
        ],
        egui::Stroke::new(1.0_f32, editor_theme::semantic().text_muted),
    );
    painter.circle_filled(
        egui::pos2(track.center().x, handle_y),
        4.0,
        editor_theme::palette().accent,
    );
    painter.text(
        rect.center_bottom(),
        egui::Align2::CENTER_BOTTOM,
        "PITCH",
        editor_theme::font::caption(),
        editor_theme::semantic().text_muted,
    );
    response.on_hover_text("Spring-loaded pitch bend wheel")
}

pub(crate) fn param_field_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    let portrait = height > width * 1.15;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(16.0), height.max(16.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(if portrait {
        egui::CursorIcon::ResizeVertical
    } else {
        egui::CursorIcon::ResizeHorizontal
    });
    let value = update_parameter_drag(
        ui,
        state,
        id,
        label,
        &response,
        if portrait {
            DragAxis::Vertical
        } else {
            DragAxis::Horizontal
        },
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        2.0,
        if response.hovered() || response.dragged() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().control
        },
    );
    let mut portrait_fill = None;
    if portrait {
        let value_y = egui::lerp(rect.bottom()..=rect.top(), value);
        let center = bipolar_center(state, id);
        let anchor_y = center.map_or(rect.bottom(), |center| {
            egui::lerp(rect.bottom()..=rect.top(), center)
        });
        let fill = egui::Rect::from_x_y_ranges(
            rect.x_range(),
            value_y.min(anchor_y)..=value_y.max(anchor_y),
        );
        painter.rect_filled(fill, 0.0, editor_theme::semantic().primary);
        portrait_fill = Some(fill);
        if center.is_some() {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), anchor_y),
                    egui::pos2(rect.right(), anchor_y),
                ],
                egui::Stroke::new(1.0_f32, editor_theme::palette().accent.gamma_multiply(0.55)),
            );
        }
    } else {
        let progress = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 2.0),
            egui::pos2(egui::lerp(rect.left()..=rect.right(), value), rect.bottom()),
        );
        painter.rect_filled(progress, 1.0, editor_theme::palette().accent);
    }
    if rect.height() >= 22.0 {
        let text_inset = ((rect.height() - 19.5) / 3.0).clamp(1.0, 4.0);
        let label_position = rect.center_top() + egui::vec2(0.0, text_inset);
        let value_position = rect.center_bottom() - egui::vec2(0.0, text_inset);
        let text_on_fill = |position| {
            portrait_fill
                .filter(|fill| fill.contains(position))
                .map_or(editor_theme::semantic().text_muted, |_| {
                    editor_theme::readable_text(editor_theme::semantic().primary)
                })
        };
        painter.text(
            label_position,
            egui::Align2::CENTER_TOP,
            label,
            editor_theme::font::caption(),
            text_on_fill(label_position),
        );
        painter.text(
            value_position,
            egui::Align2::CENTER_BOTTOM,
            compact_param_value(state, id),
            editor_theme::font::value(),
            portrait_fill
                .filter(|fill| fill.contains(value_position))
                .map_or_else(
                    || ui.visuals().text_color(),
                    |_| editor_theme::readable_text(editor_theme::semantic().primary),
                ),
        );
    } else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            compact_param_value(state, id),
            editor_theme::font::value(),
            ui.visuals().text_color(),
        );
    }
    response
}

pub(crate) fn shape_morph_strip(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    width: f32,
    height: f32,
) -> egui::Response {
    const STOPS: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
    const LABELS: [&str; 4] = ["SIN", "TRI", "SAW", "PUL"];

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(72.0), height.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let set_from_pointer = |pointer: egui::Pos2| {
        let raw = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let value = if ui.input(|input| input.modifiers.shift) {
            raw
        } else {
            magnetic_shape_snap(raw)
        };
        state.set_param(id, f64::from(value));
    };

    if response.double_clicked() {
        if let Some(info) = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(id))
        {
            state.automate(id, info.range.normalize(info.default_plain));
        }
    } else if response.drag_started() {
        state.begin_edit(id);
        if let Some(pointer) = response.interact_pointer_pos() {
            set_from_pointer(pointer);
        }
    } else if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            set_from_pointer(pointer);
        }
    } else if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        state.begin_edit(id);
        set_from_pointer(pointer);
        state.end_edit(id);
    }
    if response.drag_stopped() {
        state.end_edit(id);
    }

    let value = state.get_param(id).clamp(0.0, 1.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        2.0,
        if response.hovered() || response.dragged() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().control
        },
    );
    let track_y = rect.bottom() - 3.0;
    painter.line_segment(
        [
            egui::pos2(rect.left() + 4.0, track_y),
            egui::pos2(rect.right() - 4.0, track_y),
        ],
        egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
    );
    for (index, stop) in STOPS.into_iter().enumerate() {
        let x = egui::lerp((rect.left() + 4.0)..=(rect.right() - 4.0), stop);
        let active = (value - stop).abs() <= 0.02;
        painter.line_segment(
            [egui::pos2(x, track_y - 3.0), egui::pos2(x, track_y + 1.0)],
            egui::Stroke::new(
                if active { 2.0_f32 } else { 1.0_f32 },
                if active {
                    editor_theme::palette().accent
                } else {
                    editor_theme::semantic().text_muted
                },
            ),
        );
        painter.text(
            egui::pos2(x, rect.top() + 3.0),
            egui::Align2::CENTER_TOP,
            LABELS[index],
            editor_theme::font::caption(),
            if active {
                editor_theme::palette().accent
            } else {
                editor_theme::semantic().text_muted
            },
        );
    }
    let x = egui::lerp((rect.left() + 4.0)..=(rect.right() - 4.0), value);
    painter.circle_filled(
        egui::pos2(x, track_y),
        3.0_f32,
        editor_theme::palette().accent,
    );
    response.on_hover_text("Drag to morph; Shift bypasses waveform snapping")
}

pub(crate) fn enum_cycle_field(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    values: &[&str],
    width: f32,
    height: f32,
) -> egui::Response {
    debug_assert!(!values.is_empty());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(28.0), height.max(18.0)),
        egui::Sense::click(),
    );
    #[allow(
        clippy::cast_precision_loss,
        reason = "UI mode menus contain at most four values"
    )]
    let last = values.len().saturating_sub(1) as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let current = (state.get_param(id).clamp(0.0, 1.0) * last).round() as usize;
    if response.clicked() {
        let next = (current + 1) % values.len();
        #[allow(
            clippy::cast_precision_loss,
            reason = "UI mode menus contain at most four values"
        )]
        state.automate(
            id,
            if last > 0.0 {
                next as f64 / f64::from(last)
            } else {
                0.0
            },
        );
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        2.0,
        if response.hovered() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().control
        },
    );
    if rect.height() >= 24.0 {
        painter.text(
            rect.center_top() + egui::vec2(0.0, 2.0),
            egui::Align2::CENTER_TOP,
            label,
            editor_theme::font::caption(),
            editor_theme::semantic().text_muted,
        );
        painter.text(
            rect.center_bottom() - egui::vec2(0.0, 2.0),
            egui::Align2::CENTER_BOTTOM,
            values[current.min(values.len() - 1)],
            editor_theme::font::value(),
            ui.visuals().text_color(),
        );
    } else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            values[current.min(values.len() - 1)],
            editor_theme::font::value(),
            ui.visuals().text_color(),
        );
    }
    response.on_hover_text(format!("{label}: click to cycle"))
}

fn bipolar_center(state: &PluginContext<KurvParams>, id: P) -> Option<f32> {
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == u32::from(id))
        .filter(|info| info.range.min() < 0.0 && info.range.max() > 0.0)
        .map(|info| info.range.normalize(0.0) as f32)
}

pub(crate) fn param_toggle_dot(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    diameter: f32,
) -> egui::Response {
    let size = diameter.max(16.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    if response.clicked() {
        let enabled = state.get_param(id) >= 0.5;
        state.begin_edit(id);
        state.set_param(id, if enabled { 0.0 } else { 1.0 });
        state.end_edit(id);
    }
    let enabled = state.get_param(id) >= 0.5;
    let painter = ui.painter_at(rect);
    painter.circle_filled(
        rect.center(),
        (size * 0.24).max(3.0),
        if enabled {
            editor_theme::palette().accent
        } else {
            editor_theme::semantic().text_muted.linear_multiply(0.42)
        },
    );
    response
}

fn compact_param_value(state: &PluginContext<KurvParams>, id: P) -> String {
    if id == P::Shape {
        let shape = state.params().shape.value();
        let rounded = shape.round();
        if (shape - rounded).abs() < 0.01 {
            match rounded {
                value if value < 0.5 => "SINE",
                value if value < 1.5 => "TRI",
                value if value < 2.5 => "SAW",
                _ => "PULSE",
            }
            .to_owned()
        } else {
            format!("{shape:.2}")
        }
    } else {
        state.format_param(id)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Truce normalized parameters are bounded to 0..1 before entering egui's f32 controls"
)]
fn update_parameter_drag<PARAMS: Params + ?Sized>(
    ui: &egui::Ui,
    state: &PluginContext<PARAMS>,
    id: impl Into<u32> + Copy,
    label: &str,
    response: &egui::Response,
    axis: DragAxis,
) -> f32 {
    let id = id.into();
    let origin_id = response.id.with("drag_origin");
    let mut value = state.get_param(id);
    let info = state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == id);

    if response.double_clicked()
        && let Some(info) = info
    {
        value = info.range.normalize(info.default_plain) as f32;
        state.begin_edit(id);
        state.set_param(id, f64::from(value));
        state.end_edit(id);
        return value;
    }
    if response.drag_started() {
        state.begin_edit(id);
        ui.data_mut(|data| {
            data.insert_temp(
                origin_id,
                KnobDrag {
                    #[cfg(debug_assertions)]
                    start: value,
                    value,
                    delta_y: 0.0,
                    frames: 0,
                },
            );
        });
    }
    if response.dragged() {
        let motion = match axis {
            DragAxis::Horizontal => response.drag_motion().x,
            DragAxis::Vertical => response.drag_motion().y,
        };
        let mut drag = ui
            .data_mut(|data| data.get_temp::<KnobDrag>(origin_id))
            .unwrap_or(KnobDrag {
                #[cfg(debug_assertions)]
                start: value,
                value,
                delta_y: 0.0,
                frames: 0,
            });
        drag.value = match axis {
            DragAxis::Horizontal => (drag.value + motion / 150.0).clamp(0.0, 1.0),
            DragAxis::Vertical => accumulate_drag(drag.value, motion),
        };
        drag.delta_y += motion;
        drag.frames += 1;
        ui.data_mut(|data| data.insert_temp(origin_id, drag));
        let unrounded = drag.value;
        let next = if id == u32::from(P::Shape) {
            magnetic_shape_snap(unrounded)
        } else {
            info.and_then(|info| info.range.step_count())
                .map_or(unrounded, |steps| {
                    #[allow(clippy::cast_precision_loss, reason = "parameter step counts are tiny")]
                    let count = steps.get() as f32;
                    (unrounded * count).round() / count
                })
        };
        if (next - value).abs() > f32::EPSILON {
            value = next;
            state.set_param(id, f64::from(value));
        }
    }
    if response.drag_stopped() {
        state.end_edit(id);
        let drag = ui.data_mut(|data| {
            let drag = data.get_temp::<KnobDrag>(origin_id);
            data.remove::<KnobDrag>(origin_id);
            drag
        });
        log_knob_gesture(label, drag, state.get_param(id));
    }
    value
}

pub(crate) fn accumulate_drag(value: f32, delta_y: f32) -> f32 {
    (value - delta_y / 150.0).clamp(0.0, 1.0)
}

pub(crate) fn magnetic_shape_snap(value: f32) -> f32 {
    [0.0_f32, 1.0 / 3.0, 2.0 / 3.0, 1.0]
        .into_iter()
        .find(|point| (value - point).abs() <= 0.018)
        .unwrap_or(value)
}

#[cfg(debug_assertions)]
#[allow(
    clippy::print_stderr,
    reason = "debug builds log UI gestures to diagnose host pointer behavior"
)]
fn log_knob_gesture(label: &str, drag: Option<KnobDrag>, end: f32) {
    if let Some(drag) = drag {
        eprintln!(
            "[KURV UI] knob: {label} start={:.4} end={end:.4} delta_y={:.2} frames={}",
            drag.start, drag.delta_y, drag.frames
        );
    }
}

#[cfg(not(debug_assertions))]
fn log_knob_gesture(_label: &str, _drag: Option<KnobDrag>, _end: f32) {}

#[allow(
    clippy::cast_precision_loss,
    reason = "knob arcs use at most 64 segments"
)]
fn arc_points(
    center: egui::Pos2,
    radius: f32,
    start: f32,
    sweep: f32,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let angle = (index as f32 / segments as f32).mul_add(sweep, start);
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}
