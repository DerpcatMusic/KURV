use std::{collections::HashMap, sync::LazyLock};

use truce::params::{ParamInfo, ParamUnit, Params};
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
    coarse: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum ValueSemantic {
    #[default]
    Continuous,
    Percent,
    Pan,
    Transpose,
    Semitones,
    Cents,
    Decibels,
    LinearGain,
    Phase,
    Degrees,
    Time,
    Frequency,
    FrequencyOffset(f32),
    GrainLength,
    UnisonRate,
    Keytrack,
    Cutoff,
    Q,
    Slope,
    Ratio,
    PitchRatio,
    Voices,
    Shape,
    IntegerTens,
}

impl ValueSemantic {
    pub(crate) fn from_label(label: &str, range: &std::ops::RangeInclusive<f32>) -> Self {
        match label {
            "SEMI" | "EXP FM" => Self::Transpose,
            "RANGE" | "PITCH" | "FORMANT" => Self::Semitones,
            "CENT" | "FINE" => Self::Cents,
            "PAN" | "BAL" | "BALANCE" | "RM" => Self::Pan,
            "PHASE" | "PM" => Self::Phase,
            "A" | "D" | "R" | "ATTACK" | "DECAY" | "RELEASE" => Self::Time,
            "LENGTH" if *range.end() <= 1.0 => Self::GrainLength,
            "LENGTH" => Self::Time,
            "RATE" if *range.end() >= 100.0 => Self::Frequency,
            "RATE" if *range.end() <= 1.0 => Self::UnisonRate,
            "GAIN" => Self::LinearGain,
            "AIR" => Self::Decibels,
            "VOICES" => Self::Voices,
            "SHAPE" => Self::Shape,
            "LEVEL" | "AM" | "S" | "SUSTAIN" | "WIDTH" | "JITTR" | "MORPH" | "DRY" | "SEND"
            | "SIDE" | "SIDECHAIN" | "SWING" | "SPREAD" | "SPRAY" | "POSITION" | "DYNAMIC"
            | "DIFFUSE" | "REVERSE" | "NORMALIZE" | "NORM" | "STEREO" | "TIMING" | "TUNE"
            | "MAIN LEVEL" | "AUX SEND" | "EXT GATE" => Self::Percent,
            _ => Self::Continuous,
        }
    }

    fn from_parameter(info: &ParamInfo, id: P, label: &str) -> Self {
        if is_integer_semitone_parameter(id) {
            return Self::Transpose;
        }
        match info.unit {
            ParamUnit::Db => Self::Decibels,
            ParamUnit::Hz => Self::Frequency,
            ParamUnit::Milliseconds | ParamUnit::Seconds => Self::Time,
            ParamUnit::Percent => Self::Percent,
            ParamUnit::Semitones => Self::Semitones,
            ParamUnit::Pan => Self::Pan,
            ParamUnit::Degrees => Self::Degrees,
            ParamUnit::None => Self::from_label(label, &(0.0..=1.0)),
        }
    }
}

#[derive(Clone, Copy)]
struct CustomDrag {
    raw: f32,
    coarse: bool,
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
    crate::editor_shell::register_parameter_hover(ui, id.into(), response.hovered());
    let mut value = state.get_param(id);

    if response.double_clicked()
        && let Some(default) = parameter_default_normalized(state, id)
    {
        value = default;
        crate::editor::begin_edit(state, id);
        state.set_param(id, f64::from(value));
        crate::editor::end_edit(state, id);
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
                crate::editor::begin_edit(state, id);
                state.set_param(id, f64::from(value));
                crate::editor::end_edit(state, id);
            }
        }
    }
    if response.drag_started() {
        let coarse = ui.input(|input| input.modifiers.ctrl);
        crate::editor::begin_edit(state, id);
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
                    coarse,
                },
            );
        });
    }
    if response.dragged() {
        let info = parameter_info(id);
        let fine = ui.input(|input| input.modifiers.shift);
        let coarse = ui.input(|input| input.modifiers.ctrl);
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
                coarse,
            });
        if drag.coarse != coarse {
            drag.value = value;
            drag.coarse = coarse;
        }
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
        let next = if !fine && !coarse && id == P::Shape {
            magnetic_shape_snap(unrounded)
        } else if let Some(info) = info {
            let plain = info.range.denormalize(f64::from(unrounded)) as f32;
            let semantic = parameter_semantic(state, info, id, label);
            let snapped = semantic_snap(plain, semantic, coarse);
            let normalized = info.range.normalize(f64::from(snapped)) as f32;
            drag.step_count.map_or(normalized, |steps| {
                #[allow(clippy::cast_precision_loss, reason = "parameter step counts are tiny")]
                let count = steps as f32;
                (normalized * count).round() / count
            })
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
        crate::editor::end_edit(state, id);
        log_knob_gesture(label, drag, state.get_param(id));
    }
    value
}

fn parameter_default_normalized(_state: &PluginContext<KurvParams>, id: P) -> Option<f32> {
    parameter_info(id).map(|info| info.range.normalize(info.default_plain) as f32)
}

fn parameter_info(id: P) -> Option<&'static ParamInfo> {
    static INFOS: LazyLock<HashMap<u32, ParamInfo>> = LazyLock::new(|| {
        KurvParams::param_infos_static()
            .into_iter()
            .map(|info| (info.id, info))
            .collect()
    });
    INFOS.get(&u32::from(id))
}

fn parameter_semantic(
    state: &PluginContext<KurvParams>,
    info: &ParamInfo,
    id: P,
    label: &str,
) -> ValueSemantic {
    let Some(mode_id) = lfo_rate_mode(id) else {
        return ValueSemantic::from_parameter(info, id, label);
    };
    let mode = parameter_info(mode_id).map_or(0, |mode_info| {
        mode_info
            .range
            .denormalize(f64::from(state.get_param(mode_id)))
            .round() as u8
    });
    match mode {
        1 => ValueSemantic::Time,
        3 => ValueSemantic::Keytrack,
        _ => ValueSemantic::Frequency,
    }
}

fn lfo_rate_mode(id: P) -> Option<P> {
    Some(match id {
        P::Lfo1Rate => P::Lfo1RateMode,
        P::Lfo2Rate => P::Lfo2RateMode,
        P::Lfo3Rate => P::Lfo3RateMode,
        P::Lfo4Rate => P::Lfo4RateMode,
        P::Lfo5Rate => P::Lfo5RateMode,
        P::Lfo6Rate => P::Lfo6RateMode,
        P::Lfo7Rate => P::Lfo7RateMode,
        P::Lfo8Rate => P::Lfo8RateMode,
        _ => return None,
    })
}

fn parameter_step_count(_state: &PluginContext<KurvParams>, id: P) -> Option<u32> {
    parameter_info(id)
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

pub(crate) fn update_custom_value_drag(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    semantic: ValueSemantic,
) -> bool {
    let before = *value;
    let drag_id = response.id.with("semantic-drag");
    if response.drag_started() {
        let coarse = ui.input(|input| input.modifiers.ctrl);
        ui.data_mut(|data| {
            data.insert_temp(
                drag_id,
                CustomDrag {
                    raw: *value,
                    coarse,
                },
            );
        });
    }
    if response.dragged() {
        let (fine, coarse) = ui.input(|input| (input.modifiers.shift, input.modifiers.ctrl));
        let mut drag = ui
            .data_mut(|data| data.get_temp::<CustomDrag>(drag_id))
            .unwrap_or(CustomDrag {
                raw: *value,
                coarse,
            });
        if drag.coarse != coarse {
            drag.raw = *value;
            drag.coarse = coarse;
        }
        drag.raw = (drag.raw - response.drag_motion().y * speed * if fine { 0.1 } else { 1.0 })
            .clamp(*range.start(), *range.end());
        *value = if fine {
            drag.raw
        } else {
            semantic_snap(drag.raw, semantic, coarse)
        }
        .clamp(*range.start(), *range.end());
        ui.data_mut(|data| data.insert_temp(drag_id, drag));
    } else if response.double_clicked() {
        *value = default.clamp(*range.start(), *range.end());
    }
    if response.drag_stopped() || pointer_gesture_aborted(ui) {
        ui.data_mut(|data| data.remove::<CustomDrag>(drag_id));
    }
    value.to_bits() != before.to_bits()
}

pub(crate) fn update_custom_pitch_ratio_drag(
    ui: &egui::Ui,
    response: &egui::Response,
    value: &mut f32,
) -> bool {
    let before = *value;
    let drag_id = response.id.with("pitch-ratio-drag");
    if response.drag_started() {
        let coarse = ui.input(|input| input.modifiers.ctrl);
        ui.data_mut(|data| {
            data.insert_temp(
                drag_id,
                CustomDrag {
                    raw: value.max(1.0 / 64.0).log2(),
                    coarse,
                },
            );
        });
    }
    if response.dragged() {
        let (fine, coarse) = ui.input(|input| (input.modifiers.shift, input.modifiers.ctrl));
        let mut drag = ui
            .data_mut(|data| data.get_temp::<CustomDrag>(drag_id))
            .unwrap_or(CustomDrag {
                raw: value.max(1.0 / 64.0).log2(),
                coarse,
            });
        if drag.coarse != coarse {
            drag.raw = value.max(1.0 / 64.0).log2();
            drag.coarse = coarse;
        }
        drag.raw = (drag.raw - response.drag_motion().y / if fine { 480.0 } else { 48.0 })
            .clamp(-6.0, 6.0);
        let raw_ratio = drag.raw.exp2();
        *value = if fine {
            raw_ratio
        } else {
            semantic_snap(raw_ratio, ValueSemantic::PitchRatio, coarse)
        }
        .clamp(1.0 / 64.0, 64.0);
        ui.data_mut(|data| data.insert_temp(drag_id, drag));
    } else if response.double_clicked() {
        *value = 1.0;
    }
    if response.drag_stopped() || pointer_gesture_aborted(ui) {
        ui.data_mut(|data| data.remove::<CustomDrag>(drag_id));
    }
    value.to_bits() != before.to_bits()
}

pub(crate) fn semantic_snap(value: f32, semantic: ValueSemantic, coarse: bool) -> f32 {
    match (semantic, coarse) {
        (ValueSemantic::Transpose, false) => value.round(),
        (ValueSemantic::Ratio | ValueSemantic::Voices, false) => value.round(),
        (ValueSemantic::FrequencyOffset(_), false) => value,
        (ValueSemantic::PitchRatio, false) => value,
        (_, false) => value,
        (ValueSemantic::Continuous, true) => value,
        (ValueSemantic::Percent | ValueSemantic::Pan, true) => snap_step(value, 0.1),
        (ValueSemantic::Transpose, true) => snap_pitch_landmark(value),
        (ValueSemantic::Semitones, true) => value.round(),
        (ValueSemantic::Cents, true) => snap_step(value, 10.0),
        (ValueSemantic::Decibels, true) => snap_step(value, 3.0),
        (ValueSemantic::LinearGain, true) if value > 0.0 => {
            10.0_f32.powf(snap_step(20.0 * value.log10(), 3.0) / 20.0)
        }
        (ValueSemantic::LinearGain, true) => 0.0,
        (ValueSemantic::Phase, true) => snap_step(value, 1.0 / 24.0),
        (ValueSemantic::Degrees, true) => snap_step(value, 15.0),
        (ValueSemantic::Time | ValueSemantic::Frequency, true) => snap_125(value),
        (ValueSemantic::FrequencyOffset(reference_hz), true) => {
            let reference_hz = reference_hz.clamp(1.0, 10_000.0);
            let frequency_hz = (reference_hz + value).max(1.0);
            let semitones = 12.0 * (frequency_hz / reference_hz).log2();
            let melodic = reference_hz * (semitones.round() / 12.0).exp2() - reference_hz;
            nearest(value, &[snap_step(value, 10.0), melodic])
        }
        (ValueSemantic::GrainLength, true) => {
            let milliseconds = snap_125(5.0 * 200.0_f32.powf(value));
            (milliseconds / 5.0).log(200.0)
        }
        (ValueSemantic::UnisonRate, true) => {
            let hz = snap_125(0.02 * 5_000.0_f32.powf(value));
            (hz / 0.02).log(5_000.0)
        }
        (ValueSemantic::Keytrack, true) => {
            let multiplier = crate::modulators::lfo::keytrack_multiplier(value);
            let snapped = nearest(
                multiplier,
                &[
                    0.03125, 0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0,
                ],
            );
            if snapped <= 1.0 {
                10.0_f32.powf(2.0 * snapped.log2() / 5.0)
            } else {
                10.0_f32.powf(snapped.log2() * 20_000.0_f32.log10() / 5.0)
            }
        }
        (ValueSemantic::Cutoff, true) if value > 0.0 => {
            1_000.0 * 2.0_f32.powf((3.0 * (value / 1_000.0).log2()).round() / 3.0)
        }
        (ValueSemantic::Cutoff, true) => value,
        (ValueSemantic::Q, true) => nearest(value, &[0.5, 0.707, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0]),
        (ValueSemantic::Slope, true) => snap_step(value, 6.0),
        (ValueSemantic::Ratio, true) => nearest(
            value,
            &[
                0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0, 1_024.0,
            ],
        ),
        (ValueSemantic::PitchRatio, true) if value > 0.0 => {
            (snap_pitch_landmark(12.0 * value.log2()) / 12.0).exp2()
        }
        (ValueSemantic::PitchRatio, true) => value,
        (ValueSemantic::Voices, true) => nearest(
            value,
            &[
                1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0,
            ],
        ),
        (ValueSemantic::Shape, true) => value.round(),
        (ValueSemantic::IntegerTens, true) => snap_step(value, 10.0),
    }
}

fn snap_step(value: f32, step: f32) -> f32 {
    (value / step).round() * step
}

fn snap_pitch_landmark(value: f32) -> f32 {
    let octave = (value / 12.0).round() as i32;
    let mut points = [0.0; 9];
    let mut index = 0;
    for octave in (octave - 1)..=(octave + 1) {
        for offset in [0, 5, 7] {
            points[index] = (octave * 12 + offset) as f32;
            index += 1;
        }
    }
    nearest(value, &points)
}

fn snap_125(value: f32) -> f32 {
    if value <= 0.0 {
        return value;
    }
    let exponent = value.log10().floor();
    let scale = 10.0_f32.powf(exponent);
    nearest(value, &[scale, scale * 2.0, scale * 5.0, scale * 10.0])
}

fn nearest(value: f32, points: &[f32]) -> f32 {
    let Some((&first, rest)) = points.split_first() else {
        return value;
    };
    rest.iter().copied().fold(first, |best, point| {
        if (value - point).abs() < (value - best).abs() {
            point
        } else {
            best
        }
    })
}

pub(crate) fn magnetic_shape_snap(value: f32) -> f32 {
    [0.0_f32, 1.0 / 3.0, 2.0 / 3.0, 1.0]
        .into_iter()
        .find(|point| (value - point).abs() <= 0.018)
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use truce::params::Params;
    use truce_core::editor::{ClosureBridge, PluginContext};

    use super::update_parameter_drag;
    use crate::{KurvParams, P};

    #[test]
    fn pointer_drag_emits_one_complete_host_gesture() {
        let params = Arc::new(KurvParams::default());
        params.set_normalized(P::Osc1Level.into(), 0.5);
        let events = Arc::new(Mutex::new(Vec::new()));
        let begin_events = Arc::clone(&events);
        let set_events = Arc::clone(&events);
        let end_events = Arc::clone(&events);
        let params_for_set = Arc::clone(&params);
        let params_for_get = Arc::clone(&params);
        let params_for_plain = Arc::clone(&params);
        let params_for_format = Arc::clone(&params);
        let state = PluginContext::new(
            Arc::new(ClosureBridge {
                begin_edit: Box::new(move |id| {
                    begin_events.lock().expect("events").push(("begin", id));
                }),
                set_param: Box::new(move |id, value| {
                    params_for_set.set_normalized(id, value);
                    set_events.lock().expect("events").push(("set", id));
                }),
                end_edit: Box::new(move |id| {
                    end_events.lock().expect("events").push(("end", id));
                }),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| {
                    params_for_get.get_normalized(id).unwrap_or_default()
                }),
                get_param_plain: Box::new(move |id| {
                    params_for_plain.get_plain(id).unwrap_or_default()
                }),
                format_param: Box::new(move |id| {
                    let plain = params_for_format.get_plain(id).unwrap_or_default();
                    params_for_format
                        .format_value(id, plain)
                        .unwrap_or_default()
                }),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| {}),
                transport: Box::new(|| None),
            }),
            Arc::clone(&params),
        );
        let context = egui::Context::default();
        let frame = |events| {
            let mut rect = egui::Rect::NOTHING;
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(200.0, 100.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = ui
                        .allocate_response(egui::vec2(100.0, 40.0), egui::Sense::click_and_drag());
                    rect = response.rect;
                    update_parameter_drag(ui, &state, P::Osc1Level, "LEVEL", &response);
                },
            );
            rect
        };

        let start = frame(Vec::new()).center();
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(vec![egui::Event::PointerMoved(
            start - egui::vec2(0.0, 30.0),
        )]);
        frame(vec![egui::Event::PointerButton {
            pos: start - egui::vec2(0.0, 30.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);

        let events = events.lock().expect("events");
        assert_eq!(events.first(), Some(&("begin", u32::from(P::Osc1Level))));
        assert!(events.iter().any(|event| event.0 == "set"));
        assert_eq!(events.last(), Some(&("end", u32::from(P::Osc1Level))));
        assert!(
            params
                .get_normalized(P::Osc1Level.into())
                .unwrap_or_default()
                > 0.5
        );
    }
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
