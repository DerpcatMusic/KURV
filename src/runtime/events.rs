use crate::modulators::routing::HOST_AUTOMATION_SLOT_COUNT;
use crate::*;

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "the event boundary keeps every supported MIDI 1 and MIDI 2 expression mapping explicit"
)]
pub(crate) fn dispatch_events(
    state: &mut KurvDspState,
    params: &KurvParams,
    events: &EventList,
    next_event: &mut usize,
    sample_index: usize,
) {
    while let Some(event) = events.get(*next_event) {
        if event.sample_offset as usize > sample_index {
            break;
        }
        match &event.body {
            EventBody::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => {
                state.lfos.note_on(*note, *channel);
                state
                    .synth
                    .note_on(*note, norm_7bit(*velocity), *channel, None);
            }
            EventBody::NoteOn2 {
                channel,
                note,
                velocity,
                ..
            } => {
                state.lfos.note_on(*note, *channel);
                state
                    .synth
                    .note_on(*note, norm_u16(*velocity), *channel, None);
            }
            EventBody::NoteOff { channel, note, .. } => {
                state.lfos.note_off(*note, *channel);
                state.synth.note_off(*note, *channel, None);
            }
            EventBody::NoteOff2 { channel, note, .. } => {
                state.lfos.note_off(*note, *channel);
                state.synth.note_off(*note, *channel, None);
            }
            EventBody::Aftertouch {
                channel,
                note,
                pressure,
                ..
            } => state
                .synth
                .pressure(*note, *channel, None, norm_7bit(*pressure)),
            EventBody::PolyPressure2 {
                channel,
                note,
                pressure,
                ..
            } => state
                .synth
                .pressure(*note, *channel, None, norm_u32(*pressure)),
            EventBody::ChannelPressure {
                channel, pressure, ..
            } => state.synth.channel_pressure(*channel, norm_7bit(*pressure)),
            EventBody::ChannelPressure2 {
                channel, pressure, ..
            } => state.synth.channel_pressure(*channel, norm_u32(*pressure)),
            EventBody::PitchBend { channel, value, .. } => {
                state.synth.pitch_bend_asymmetric(
                    *channel,
                    norm_pitch_bend(*value),
                    state.pitch_bend_down_range,
                    state.pitch_bend_range,
                );
            }
            EventBody::PitchBend2 { channel, value, .. } => {
                state.synth.pitch_bend_asymmetric(
                    *channel,
                    norm_pitch_bend_32(*value),
                    state.pitch_bend_down_range,
                    state.pitch_bend_range,
                );
            }
            EventBody::PerNotePitchBend {
                channel,
                note,
                value,
                ..
            } => state.synth.per_note_pitch_bend(
                *note,
                *channel,
                (per_note_bend_semitones(*value) / 48.0 * f64::from(state.mpe_bend_range)) as f32,
            ),
            EventBody::PerNoteCC {
                channel,
                note,
                cc: 74,
                value,
                ..
            } => state
                .synth
                .per_note_timbre(*note, *channel, norm_u32(*value)),
            EventBody::PerNoteManagement {
                channel,
                note,
                flags,
                ..
            } if flags & 0b10 != 0 => {
                state.synth.reset_per_note_controllers(*note, *channel);
            }
            EventBody::ParamChange { id, value } if *id == u32::from(P::PitchBend) => {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "the pitch wheel parameter is bounded to -1..1 before entering f32 DSP"
                )]
                apply_pitch_bend_param(state, *value as f32);
            }
            EventBody::ParamChange { id, value } if *id == u32::from(P::ModWheel) => {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "the mod wheel parameter is bounded to 0..1 before entering f32 DSP"
                )]
                apply_mod_wheel_param(state, *value as f32);
            }
            EventBody::ParamMod { id, value, .. } => {
                apply_param_mod(state, params, *id, *value);
            }
            EventBody::ControlChange {
                channel, cc, value, ..
            } => match cc {
                1 => {
                    state
                        .mod_wheel_ramp
                        .retarget(norm_7bit(*value), state.dsp_sample_rate);
                }
                64 => {
                    let held = *value >= 64;
                    state.lfos.sustain(*channel, held);
                    state.synth.sustain(*channel, held);
                }
                74 => state.synth.timbre(*channel, norm_7bit(*value)),
                120 => {
                    state.lfos.all_sound_off(*channel);
                    state.synth.all_sound_off(*channel);
                }
                121 => {
                    state.mod_wheel_ramp.retarget(0.0, state.dsp_sample_rate);
                    state.lfos.reset_controllers(*channel);
                    state.synth.reset_controllers(*channel);
                }
                123..=127 => {
                    state.lfos.all_notes_off(*channel);
                    state.synth.all_notes_off(*channel);
                }
                _ => {}
            },
            EventBody::ControlChange2 {
                channel, cc, value, ..
            } => match cc {
                1 => {
                    state
                        .mod_wheel_ramp
                        .retarget(norm_u32(*value), state.dsp_sample_rate);
                }
                64 => {
                    let held = *value >= 0x8000_0000;
                    state.lfos.sustain(*channel, held);
                    state.synth.sustain(*channel, held);
                }
                74 => state.synth.timbre(*channel, norm_u32(*value)),
                120 => {
                    state.lfos.all_sound_off(*channel);
                    state.synth.all_sound_off(*channel);
                }
                121 => {
                    state.mod_wheel_ramp.retarget(0.0, state.dsp_sample_rate);
                    state.lfos.reset_controllers(*channel);
                    state.synth.reset_controllers(*channel);
                }
                123..=127 => {
                    state.lfos.all_notes_off(*channel);
                    state.synth.all_notes_off(*channel);
                }
                _ => {}
            },
            _ => {}
        }
        *next_event += 1;
    }
}

pub(crate) fn apply_incoming_param_mods(
    state: &mut KurvDspState,
    params: &KurvParams,
    events: &EventList,
) {
    for event in events.iter() {
        let EventBody::ParamMod { id, value, .. } = event.body else {
            continue;
        };
        apply_param_mod(state, params, id, value);
    }
}

fn apply_param_mod(state: &mut KurvDspState, params: &KurvParams, id: u32, value: f64) {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "CLAP modulation offsets are bounded host-normalized amounts"
    )]
    let offset = value as f32;
    if let Some(slot) = host_automation_slot_index(id) {
        state.host_param_mod[slot] = offset;
        return;
    }
    if id == u32::from(P::PitchBend) {
        state.pitch_bend_mod = offset;
        apply_pitch_bend_param(state, params.pitch_bend.value());
        return;
    }
    if id == u32::from(P::ModWheel) {
        state.mod_wheel_mod = offset;
        apply_mod_wheel_param(state, params.mod_wheel.value());
    }
}

fn apply_pitch_bend_param(state: &mut KurvDspState, value: f32) {
    state.synth.parameter_pitch_bend_asymmetric(
        (value + state.pitch_bend_mod).clamp(-1.0, 1.0),
        state.pitch_bend_down_range,
        state.pitch_bend_range,
    );
}

fn apply_mod_wheel_param(state: &mut KurvDspState, value: f32) {
    state.mod_wheel_ramp.retarget(
        (value + state.mod_wheel_mod).clamp(0.0, 1.0),
        state.dsp_sample_rate,
    );
}

fn host_automation_slot_index(id: u32) -> Option<usize> {
    let first = u32::from(P::Host01);
    let offset = id.checked_sub(first)?;
    usize::try_from(offset)
        .ok()
        .filter(|slot| *slot < HOST_AUTOMATION_SLOT_COUNT)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MIDI 2.0 normalized controls intentionally enter the f32 DSP domain"
)]
pub(crate) fn norm_u16(value: u16) -> f32 {
    f32::from(value) / f32::from(u16::MAX)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MIDI 2.0 normalized controls intentionally enter the f32 DSP domain"
)]
pub(crate) fn norm_u32(value: u32) -> f32 {
    value as f32 / u32::MAX as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the normalized bend is bounded to -1..1 before entering the f32 DSP domain"
)]
pub(crate) fn norm_pitch_bend_32(value: u32) -> f32 {
    ((f64::from(value) - 2_147_483_648.0) / 2_147_483_648.0) as f32
}
