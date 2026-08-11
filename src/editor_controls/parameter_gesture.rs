use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::{KurvParams, P};

#[derive(Clone, Copy)]
struct KnobDrag {
    #[cfg(debug_assertions)]
    start: f32,
    value: f32,
    delta_y: f32,
    frames: u32,
    step_count: Option<u32>,
}

pub(crate) fn pointer_gesture_aborted(ui: &egui::Ui) -> bool {
    ui.input(|input| !input.focused || !input.pointer.primary_down())
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Truce normalized parameters are bounded to 0..1 before entering egui's f32 controls"
)]
pub(crate) fn update_parameter_drag(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    response: &egui::Response,
) -> f32 {
    let origin_id = response.id.with("drag_origin");
    let mut value = state.get_param(id);

    if response.double_clicked()
        && let Some(default) = parameter_default_normalized(state, id)
    {
        value = default;
        state.begin_edit(id);
        state.set_param(id, f64::from(value));
        state.end_edit(id);
        return value;
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    if response.has_focus() && !response.dragged() && !response.is_pointer_button_down_on() {
        let direction = ui.input(|input| {
            i8::from(
                input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::ArrowRight),
            ) - i8::from(
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowLeft),
            )
        });
        if direction != 0 {
            let fine = ui.input(|input| input.modifiers.shift);
            let step =
                parameter_step_count(state, id).map_or(if fine { 0.001 } else { 0.01 }, |steps| {
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "parameter step counts fit exactly in the compact control ranges"
                    )]
                    let count = steps as f32;
                    count.recip()
                });
            let next = (value + f32::from(direction) * step).clamp(0.0, 1.0);
            if (next - value).abs() > f32::EPSILON {
                value = next;
                state.begin_edit(id);
                state.set_param(id, f64::from(value));
                state.end_edit(id);
            }
        }
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
                    step_count: parameter_step_count(state, id),
                },
            );
        });
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let mut drag = ui
            .data_mut(|data| data.get_temp::<KnobDrag>(origin_id))
            .unwrap_or(KnobDrag {
                #[cfg(debug_assertions)]
                start: value,
                value,
                delta_y: 0.0,
                frames: 0,
                step_count: parameter_step_count(state, id),
            });
        let discrete_semitone_drag = if is_integer_semitone_parameter(id) {
            drag.step_count.map_or(0.0, |steps| steps as f32 * 8.0)
        } else {
            0.0
        };
        drag.value = if discrete_semitone_drag > 0.0 {
            (drag.value - motion / discrete_semitone_drag).clamp(0.0, 1.0)
        } else {
            accumulate_drag(drag.value, motion)
        };
        drag.delta_y += motion;
        drag.frames += 1;
        ui.data_mut(|data| data.insert_temp(origin_id, drag));
        let unrounded = drag.value;
        let next = if !fine && id == P::Shape {
            magnetic_shape_snap(unrounded)
        } else {
            drag.step_count.map_or(unrounded, |steps| {
                #[allow(clippy::cast_precision_loss, reason = "parameter step counts are tiny")]
                let count = steps as f32;
                (unrounded * count).round() / count
            })
        };
        if (next - value).abs() > f32::EPSILON {
            value = next;
            state.set_param(id, f64::from(value));
        }
    }
    let drag_active = ui.data(|data| data.get_temp::<KnobDrag>(origin_id).is_some());
    if drag_active && (response.drag_stopped() || pointer_gesture_aborted(ui)) {
        let drag = ui.data_mut(|data| {
            let drag = data.get_temp::<KnobDrag>(origin_id);
            data.remove::<KnobDrag>(origin_id);
            drag
        });
        state.end_edit(id);
        log_knob_gesture(label, drag, state.get_param(id));
    }
    value
}

fn parameter_default_normalized(state: &PluginContext<KurvParams>, id: P) -> Option<f32> {
    let raw_id = u32::from(id);
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == raw_id)
        .map(|info| info.range.normalize(info.default_plain) as f32)
}

fn parameter_step_count(state: &PluginContext<KurvParams>, id: P) -> Option<u32> {
    let raw_id = u32::from(id);
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == raw_id)
        .and_then(|info| info.range.step_count())
        .map(std::num::NonZeroU32::get)
}

fn is_integer_semitone_parameter(id: P) -> bool {
    matches!(
        id,
        P::Transpose | P::Osc1Transpose | P::Osc2Transpose | P::Osc3Transpose
    )
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
