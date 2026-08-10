use std::hint::black_box;
use std::time::Instant;

use pure_va_dispersion_core::{Kurv, KurvDspState, KurvParams};
use truce::prelude::*;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        usage();
    }
    let frames = parse_usize(&args[0]);
    let callbacks = parse_usize(&args[1]);
    let repeats = parse_usize(&args[2]);

    let params = KurvParams::default();
    params.set_sample_rate(48_000.0);
    params.snap_smoothers();
    let mut state = KurvDspState::default();
    <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, frames));

    let input_events = EventList::with_capacity(0);
    let mut output_events = EventList::with_capacity(0);
    let transport = TransportInfo::default();
    let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let inputs: [&[f32]; 0] = [];
    let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
    let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);

    for _ in 0..4_096 {
        black_box(<Kurv as PluginLogic>::process(
            &mut state,
            &params,
            &mut buffer,
            &input_events,
            &mut context,
        ));
    }

    let mut measurements = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        for _ in 0..callbacks {
            black_box(<Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            ));
        }
        measurements.push(start.elapsed());
    }
    measurements.sort_unstable();
    let median = measurements[measurements.len() / 2];
    println!(
        "frames={frames},callbacks={callbacks},repeats={repeats},median_ns_per_callback={:.3},median_ns_per_frame={:.3}",
        median.as_nanos() as f64 / callbacks as f64,
        median.as_nanos() as f64 / (callbacks * frames) as f64,
    );
}

fn parse_usize(value: &str) -> usize {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value != 0)
        .unwrap_or_else(|| usage())
}

fn usage() -> ! {
    eprintln!("usage: process_lab <frames> <callbacks> <repeats>");
    std::process::exit(2);
}
