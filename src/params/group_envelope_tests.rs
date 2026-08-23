use truce::params::Params;
use truce::prelude::{AudioBuffer, AudioConfig, PluginLogic, ProcessContext, TransportInfo};
use truce_core::events::{Event, EventBody, EventList};

use super::KurvParams;
use crate::generators::GroupOutput;

const SAMPLE_RATE: f64 = 48_000.0;
const BLOCK: usize = 64;

fn process_held_note(params: &KurvParams, blocks: usize) -> Vec<f32> {
    params.set_sample_rate(SAMPLE_RATE);
    params.snap_smoothers();
    let mut state = crate::KurvDspState::default();
    <crate::Kurv as PluginLogic>::reset(&mut state, params, &AudioConfig::new(SAMPLE_RATE, BLOCK));

    let mut samples = Vec::with_capacity(blocks * BLOCK);
    for block in 0..blocks {
        let mut input_events = EventList::with_capacity(1);
        if block == 0 {
            input_events.push(Event::new(
                0,
                EventBody::NoteOn {
                    group: 0,
                    channel: 1,
                    note: 60,
                    velocity: 127,
                },
            ));
        }
        let mut output_events = EventList::with_capacity(0);
        let transport = TransportInfo::default();
        let mut context = ProcessContext::new(&transport, SAMPLE_RATE, BLOCK, &mut output_events);
        let mut left = vec![0.0; BLOCK];
        let mut right = vec![0.0; BLOCK];
        let inputs: [&[f32]; 0] = [];
        let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
        let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, BLOCK);
        let _ = <crate::Kurv as PluginLogic>::process(
            &mut state,
            params,
            &mut buffer,
            &input_events,
            &mut context,
        );
        samples.extend(left);
    }
    samples
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
}

fn set_group_envelope(params: &KurvParams, sustain: f32, decay: f32) {
    let group_id = params.generator_stack.snapshot().groups()[0].id();
    let mut output = GroupOutput::default();
    output.attack = 0.0;
    output.decay = decay;
    output.sustain = sustain;
    output.release = 0.0;
    output.envelope_enabled = true;
    assert!(params.generator_stack.set_group_output(group_id, output));
}

#[test]
fn group_sustain_zero_silences_a_held_note_after_decay() {
    let silent = KurvParams::default();
    set_group_envelope(&silent, 0.0, 0.005);
    let silent_samples = process_held_note(&silent, 24);

    let held = KurvParams::default();
    set_group_envelope(&held, 1.0, 0.005);
    let held_samples = process_held_note(&held, 24);

    let tail_start = silent_samples.len() - BLOCK * 4;
    let silent_tail = peak(&silent_samples[tail_start..]);
    let held_tail = peak(&held_samples[tail_start..]);

    assert!(
        held_tail > 0.05,
        "sustain 1 must keep the group sounding, peak={held_tail}"
    );
    assert!(
        silent_tail < held_tail * 0.02,
        "sustain 0 must act as the group envelope, silent_tail={silent_tail} held_tail={held_tail}"
    );
}
