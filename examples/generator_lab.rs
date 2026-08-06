#![allow(dead_code)]

#[path = "../src/lfo.rs"]
mod lfo;
#[path = "../src/oscillator.rs"]
mod oscillator;
#[path = "../src/oversampling.rs"]
mod oversampling;
#[path = "../src/pan_curve.rs"]
mod pan_curve;
#[path = "../src/voice.rs"]
mod voice;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::fs::File;
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use oscillator::{Antialiasing, PhaseWarpMode};
use oversampling::StereoOversampler;
use voice::{
    BLOCK_INTERNAL_SAMPLES, EnvelopeSettings, FACTOR3_BLOCK_INTERNAL_SAMPLES, InternalRtPool,
    MAX_JOB_SAMPLES, OscillatorSettings, PolySynth, SwarmMode, UnisonSettings, VaVoice,
    VoiceSettings, WANDER_BLOCK_INTERNAL_SAMPLES,
};
use wave_curve::WaveCurveRt;

const HOST_RATE: f32 = 48_000.0;

fn main() {
    let _denormal_guard = truce_core::denormal::DenormalGuard::new();
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("bench") => bench(&args[1..], true, false),
        Some("bench-pair") => bench(&args[1..], false, false),
        Some("bench-pool") => bench(&args[1..], true, true),
        Some("bench-morph") => bench_morph(&args[1..]),
        Some("bench-release") => bench_release(&args[1..]),
        Some("bench-trigger") => bench_trigger(&args[1..]),
        Some("bench-lfo") => bench_lfo(&args[1..]),
        Some("compare-spectral-pool") => compare_spectral_pool(&args[1..]),
        Some("idle-pool") => idle_pool(&args[1..]),
        Some("compare-pair") => compare_pair(&args[1..]),
        Some("compare-glide") => compare_glide(&args[1..]),
        Some("sweep-live") => sweep_live(&args[1..]),
        Some("sweep-unison") => sweep_unison(&args[1..]),
        Some("render") => render(&args[1..]),
        _ => usage(),
    }
}

fn bench_lfo(args: &[String]) {
    if args.len() != 4 {
        usage();
    }
    let active = parse_usize(&args[0]).clamp(1, lfo::LFO_COUNT);
    let rate = parse_bounded_f32(&args[1], 0.01, 20_000.0);
    let frames = parse_usize(&args[2]);
    let repeats = parse_usize(&args[3]);
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0_f32;
    for _ in 0..repeats {
        let mut bank = lfo::LfoBank::default();
        bank.reset(HOST_RATE * 2.0);
        bank.configure(
            [lfo::LfoConfig {
                rate_hz: rate,
                ..lfo::LfoConfig::default()
            }; lfo::LFO_COUNT],
            [Some(WaveCurveRt::default()); lfo::LFO_COUNT],
            if active == u8::BITS as usize {
                u8::MAX
            } else {
                (1_u8 << active) - 1
            },
            &truce_core::events::TransportInfo::default(),
        );
        for _ in 0..4_096 {
            checksum += black_box(bank.next()).iter().sum::<f32>();
        }
        let start = Instant::now();
        for _ in 0..frames {
            checksum += black_box(bank.next()).iter().sum::<f32>();
        }
        measurements.push(start.elapsed().as_nanos() as f64 / frames as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "active_lfos={active},rate={rate},frames={frames},repeats={repeats},median_ns_per_internal_sample={:.3},min_ns_per_internal_sample={:.3},max_ns_per_internal_sample={:.3},checksum={checksum:.9}",
        measurements[measurements.len() / 2],
        measurements[0],
        measurements[measurements.len() - 1],
    );
}

fn bench_release(args: &[String]) {
    if args.len() != 3 {
        usage();
    }
    let pooled = match args[0].as_str() {
        "serial" => false,
        "pool" => true,
        _ => usage(),
    };
    let frames = parse_usize(&args[1]);
    let repeats = parse_usize(&args[2]);
    let mut measurements = Vec::with_capacity(repeats);
    let mut participation = [0_u64; 4];
    let mut fallbacks = 0_u64;
    let mut checksum = 0.0_f32;
    for _ in 0..repeats {
        let mut engine = BenchEngine::new(
            Antialiasing::SplineOptimized,
            2,
            2.0,
            64,
            48,
            0.5,
            0.0,
            0.7,
            24,
            SwarmMode::Noise,
            3,
        );
        engine.envelope = EnvelopeSettings {
            release: 0.182,
            ..EnvelopeSettings::default()
        };
        if pooled {
            engine.pool = Some(InternalRtPool::new());
            engine.pool_chunks = MAX_JOB_SAMPLES / engine.block_samples;
            engine.block_frames = engine.block_samples * engine.pool_chunks / 2;
            engine.block_index = engine.block_frames;
        }
        for _ in 0..4_096 {
            checksum += black_box(engine.next());
        }
        for note in 48..72 {
            engine.synth.note_off(note, 0, None);
        }
        let start = Instant::now();
        for _ in 0..frames {
            checksum += black_box(engine.next());
        }
        measurements.push(start.elapsed());
        if let Some(pool) = &engine.pool {
            participation = pool.worker_participation_all();
            fallbacks = pool.deadline_fallbacks();
        }
    }
    measurements.sort_unstable();
    println!(
        "mode={},release_ms=182,frames={frames},repeats={repeats},median_ns_per_frame={:.3},participation={participation:?},deadline_fallbacks={fallbacks},checksum={checksum:.9}",
        args[0],
        nanos_per_frame(measurements[repeats / 2], frames),
    );
}

fn sweep_unison(args: &[String]) {
    if !args.is_empty() {
        usage();
    }
    let sample_rate = HOST_RATE * 2.0;
    let initial = UnisonSettings::new(64, 48.0, 1.0, 1.0, 0.35).with_stereo_square(1.0, 0.0);
    let settings = VoiceSettings::new(0.0, 110.0, 0.5, 0.0, 0.0, 0.0)
        .with_antialiasing(Antialiasing::SplineOptimized);
    let envelope = EnvelopeSettings::default();
    for (change, start, target) in [
        (
            "xy",
            initial,
            UnisonSettings::new(64, 48.0, 1.0, 1.0, 0.35).with_stereo_square(0.0, 1.0),
        ),
        (
            "voices-remove",
            initial,
            UnisonSettings::new(1, 48.0, 1.0, 1.0, 0.35).with_stereo_square(1.0, 0.0),
        ),
        (
            "voices-add",
            UnisonSettings::new(1, 48.0, 1.0, 1.0, 0.35).with_stereo_square(1.0, 0.0),
            initial,
        ),
    ] {
        let mut reference = PolySynth::default();
        let mut changed = PolySynth::default();
        for synth in [&mut reference, &mut changed] {
            synth.set_sample_rate(sample_rate);
            synth.configure_unison(start);
            synth.note_on(60, 1.0, 0, None);
        }
        let mut previous_changed = (0.0, 0.0);
        for _ in 0..4_096 {
            black_box(reference.render(settings, envelope));
            previous_changed = black_box(changed.render(settings, envelope));
        }
        changed.configure_unison(target);
        let mut previous_error = 0.0_f32;
        let mut maximum_error_step = 0.0_f32;
        let mut maximum_output_step = 0.0_f32;
        let mut error_energy = 0.0_f64;
        for _ in 0..960 {
            let reference_sample = reference.render(settings, envelope);
            let changed_sample = changed.render(settings, envelope);
            let error = changed_sample.0 - reference_sample.0;
            maximum_error_step = maximum_error_step.max((error - previous_error).abs());
            maximum_output_step = maximum_output_step.max(
                (changed_sample.0 - previous_changed.0)
                    .abs()
                    .max((changed_sample.1 - previous_changed.1).abs()),
            );
            error_energy += f64::from(error * error);
            previous_error = error;
            previous_changed = changed_sample;
        }
        println!(
            "change={change},max_residual_step={maximum_error_step:.9},max_output_step={maximum_output_step:.9},residual_rms={:.9}",
            (error_energy / 960.0).sqrt()
        );
    }

    let mut reference = PolySynth::default();
    let mut changed = PolySynth::default();
    for synth in [&mut reference, &mut changed] {
        synth.set_sample_rate(sample_rate);
        synth.configure_unison(initial);
        synth.note_on(60, 1.0, 0, None);
    }
    let mut previous_reference = (0.0, 0.0);
    let mut previous_changed = (0.0, 0.0);
    for _ in 0..4_096 {
        previous_reference = reference.render(settings, envelope);
        previous_changed = changed.render(settings, envelope);
    }
    let mut maximum_reference_step = 0.0_f32;
    let mut maximum_changed_step = 0.0_f32;
    for frame in 0..960 {
        if frame % 16 == 0 {
            let x = frame as f32 / 959.0;
            changed.configure_unison(initial.with_stereo_square(1.0 - x, x));
        }
        let reference_sample = reference.render(settings, envelope);
        let changed_sample = changed.render(settings, envelope);
        maximum_reference_step = maximum_reference_step.max(
            (reference_sample.0 - previous_reference.0)
                .abs()
                .max((reference_sample.1 - previous_reference.1).abs()),
        );
        maximum_changed_step = maximum_changed_step.max(
            (changed_sample.0 - previous_changed.0)
                .abs()
                .max((changed_sample.1 - previous_changed.1).abs()),
        );
        previous_reference = reference_sample;
        previous_changed = changed_sample;
    }
    println!(
        "change=xy-drag,max_reference_step={maximum_reference_step:.9},max_output_step={maximum_changed_step:.9}"
    );

    let mut warped_settings = settings;
    warped_settings.oscillators[0] =
        warped_settings.oscillators[0].with_phase_warp(PhaseWarpMode::PhaseBend, 0.98);
    let mut reference = PolySynth::default();
    let mut changed = PolySynth::default();
    for synth in [&mut reference, &mut changed] {
        synth.set_sample_rate(sample_rate);
        synth.configure_unison(initial);
        synth.note_on(60, 1.0, 0, None);
    }
    reference.configure_phase_warp_modes([
        PhaseWarpMode::PhaseBend,
        PhaseWarpMode::None,
        PhaseWarpMode::None,
    ]);
    let mut previous_reference = (0.0, 0.0);
    let mut previous_changed = (0.0, 0.0);
    for _ in 0..4_096 {
        previous_reference = reference.render(warped_settings, envelope);
        previous_changed = changed.render(warped_settings, envelope);
    }
    changed.configure_phase_warp_modes([
        PhaseWarpMode::PhaseBend,
        PhaseWarpMode::None,
        PhaseWarpMode::None,
    ]);
    let mut maximum_reference_step = 0.0_f32;
    let mut maximum_changed_step = 0.0_f32;
    for _ in 0..960 {
        let reference_sample = reference.render(warped_settings, envelope);
        let changed_sample = changed.render(warped_settings, envelope);
        maximum_reference_step = maximum_reference_step.max(
            (reference_sample.0 - previous_reference.0)
                .abs()
                .max((reference_sample.1 - previous_reference.1).abs()),
        );
        maximum_changed_step = maximum_changed_step.max(
            (changed_sample.0 - previous_changed.0)
                .abs()
                .max((changed_sample.1 - previous_changed.1).abs()),
        );
        previous_reference = reference_sample;
        previous_changed = changed_sample;
    }
    println!(
        "change=warp-mode,max_reference_step={maximum_reference_step:.9},max_output_step={maximum_changed_step:.9}"
    );
}

fn bench_trigger(args: &[String]) {
    if args.len() != 4 {
        usage();
    }
    let polyphony = parse_u8(&args[0], 1, 32);
    let oscillators = parse_u8(&args[1], 1, 3);
    let mode = args[2].as_str();
    let repeats = parse_usize(&args[3]);
    let (vertical, horizontal) = match mode {
        "shape" => (0.0, 1.0),
        "random" => (0.0, 0.0),
        _ => usage(),
    };
    let unison =
        UnisonSettings::new(64, 100.0, 1.0, 1.0, 0.0).with_stereo_square(vertical, horizontal);
    let mut measurements = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let mut synth = PolySynth::default();
        synth.set_sample_rate(192_000.0);
        synth.configure_unison(unison);
        for oscillator in 1..usize::from(oscillators) {
            synth.configure_secondary_unison(oscillator, unison);
        }
        synth.configure_oscillator_enabled(std::array::from_fn(|index| {
            index < usize::from(oscillators)
        }));
        let start = Instant::now();
        for note in 0..polyphony {
            synth.note_on(48 + note, 1.0, 0, None);
        }
        measurements.push(start.elapsed());
        black_box(synth);
    }
    measurements.sort_unstable();
    println!(
        "polyphony={polyphony},oscillators={oscillators},stereo={mode},repeats={repeats},median_trigger_ns={}",
        measurements[repeats / 2].as_nanos()
    );
}

fn sweep_live(args: &[String]) {
    if args.len() != 1 {
        usage();
    }
    let polyphony = parse_u8(&args[0], 1, 24);
    for (changing, spectral) in [(false, false), (true, false), (true, true)] {
        let sample_rate = HOST_RATE * 2.0;
        let mut synth = PolySynth::default();
        synth.set_sample_rate(sample_rate);
        synth.reset();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(2);
        oversampler.set_spline_correction_immediate(!spectral);
        for note in 0..polyphony {
            synth.note_on(48 + note, 0.65, 0, None);
        }
        let mut settings =
            VoiceSettings::new(2.0, 110.0, 0.5, 0.0, 0.0, 0.0).with_antialiasing(if spectral {
                Antialiasing::Spectral
            } else {
                Antialiasing::SplineOptimized
            });
        let mut spectral_compatibility = false;
        let envelope = EnvelopeSettings::default();
        let mut previous = 0.0_f32;
        let mut maximum_step = 0.0_f32;
        let mut step_energy = 0.0_f64;
        let frames = 48_000_usize;
        for frame in 0..frames + 4_096 {
            if changing && frame >= 4_096 && (frame - 4_096) % 1_024 == 0 {
                let change = (frame - 4_096) / 1_024;
                let voices = [1, 4, 17, 64][change % 4];
                let unison = UnisonSettings::new(
                    voices,
                    [0.0, 9.0, 31.0, 74.0][change % 4],
                    [0.0, 0.35, 0.7, 1.0][change % 4],
                    0.0,
                    [-1.0, -0.25, 0.4, 1.0][change % 4],
                )
                .with_swarm([0.0, 0.3, 0.7, 1.0][change % 4], 0.7)
                .with_swarm_mode(if change.is_multiple_of(2) {
                    SwarmMode::Noise
                } else {
                    SwarmMode::Sine
                });
                synth.configure_unison(unison);
                let mode = [
                    PhaseWarpMode::None,
                    PhaseWarpMode::Pwm,
                    PhaseWarpMode::PhaseBend,
                    PhaseWarpMode::Harmonic,
                ][change % 4];
                synth.configure_phase_warp_modes([mode, PhaseWarpMode::None, PhaseWarpMode::None]);
                settings.oscillators[0] = OscillatorSettings::new(true, 2.0, 0.5, 1.0, 1.0, 0.0)
                    .with_phase_warp(
                        mode,
                        if mode == PhaseWarpMode::None {
                            0.0
                        } else {
                            0.98
                        },
                    );
            }
            if spectral {
                let compatible = settings.spectral_warp_compatibility(spectral_compatibility)
                    || synth.spectral_low_fallback_eligible(settings, spectral_compatibility);
                if compatible != spectral_compatibility {
                    synth.mark_output_continuity();
                }
                spectral_compatibility = compatible;
                settings.antialiasing = if compatible {
                    Antialiasing::SplineOptimized
                } else {
                    Antialiasing::Spectral
                };
                oversampler.set_spline_correction(compatible);
            }
            for _ in 0..2 {
                let (left, right) = synth.render(settings, envelope);
                oversampler.push(left, right);
            }
            let output = oversampler.output().0;
            if frame >= 4_096 {
                let step = (output - previous).abs();
                maximum_step = maximum_step.max(step);
                step_energy += f64::from(step * step);
            }
            previous = output;
        }
        println!(
            "polyphony={polyphony},changing={changing},spectral={spectral},max_sample_step={maximum_step:.9},rms_sample_step={:.9}",
            (step_energy / frames as f64).sqrt()
        );
    }
}

fn idle_pool(args: &[String]) {
    if args.len() != 1 {
        usage();
    }
    let seconds = parse_usize(&args[0]);
    let pool = InternalRtPool::new();
    std::thread::sleep(Duration::from_secs(seconds as u64));
    println!("idle_seconds={seconds},fifo={:?}", pool.fifo_workers_all());
}

fn compare_glide(args: &[String]) {
    if !(4..=5).contains(&args.len()) {
        usage();
    }
    let shape = parse_shape(&args[0]);
    let start = args[1]
        .parse::<f32>()
        .ok()
        .filter(|value| *value > 0.0)
        .unwrap_or_else(|| usage());
    let end = args[2]
        .parse::<f32>()
        .ok()
        .filter(|value| *value > 0.0)
        .unwrap_or_else(|| usage());
    let frames = parse_usize(&args[3]);
    let pulse_width = args
        .get(4)
        .map_or(0.5, |value| parse_bounded_f32(value, 0.03, 0.97));
    let mut scalar = BenchEngine::new(
        Antialiasing::Spectral,
        1,
        shape,
        1,
        69,
        pulse_width,
        0.0,
        0.5,
        1,
        SwarmMode::Wander,
        1,
    );
    let mut vector = BenchEngine::new(
        Antialiasing::Spectral,
        1,
        shape,
        8,
        69,
        pulse_width,
        0.0,
        0.5,
        1,
        SwarmMode::Wander,
        1,
    );
    scalar
        .synth
        .configure_unison(UnisonSettings::new(1, 0.0, 0.0, 0.0, 0.0));
    vector
        .synth
        .configure_unison(UnisonSettings::new(8, 0.0, 0.0, 0.0, 0.0));
    let start_bend = 12.0 * (start / 440.0).log2() / 96.0;
    scalar.synth.pitch_bend(0, start_bend, 96.0);
    vector.synth.pitch_bend(0, start_bend, 96.0);
    for _ in 0..16_384 {
        black_box(scalar.next());
        black_box(vector.next());
    }
    let mut reference = Vec::with_capacity(frames);
    let mut candidate = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mix = frame as f32 / (frames - 1).max(1) as f32;
        let frequency = mix.mul_add(end - start, start);
        let bend = 12.0 * (frequency / 440.0).log2() / 96.0;
        scalar.synth.pitch_bend(0, bend, 96.0);
        vector.synth.pitch_bend(0, bend, 96.0);
        reference.push(f64::from(scalar.next()));
        candidate.push(f64::from(vector.next()));
    }
    let reference_energy = reference.iter().map(|sample| sample * sample).sum::<f64>();
    let scale = reference
        .iter()
        .zip(&candidate)
        .map(|(a, b)| a * b)
        .sum::<f64>()
        / reference_energy;
    let mut error_energy = 0.0;
    let mut maximum = 0.0_f64;
    let mut maximum_step = 0.0_f64;
    let mut previous = 0.0;
    for (index, (a, b)) in reference.iter().zip(&candidate).enumerate() {
        let error = b / scale - a;
        error_energy = error.mul_add(error, error_energy);
        maximum = maximum.max(error.abs());
        if index != 0 {
            maximum_step = maximum_step.max((error - previous).abs());
        }
        previous = error;
    }
    let rms_error = (error_energy / frames as f64).sqrt();
    let rms_signal = (reference_energy / frames as f64).sqrt();
    println!(
        "shape={shape},start_hz={start},end_hz={end},frames={frames},gain_scale={scale:.9},max_abs_error={maximum:.9e},max_error_step={maximum_step:.9e},rms_error={rms_error:.9e},error_db={:.3}",
        20.0 * (rms_error / rms_signal).log10()
    );
}

fn compare_pair(args: &[String]) {
    if !(2..=5).contains(&args.len()) {
        usage();
    }
    let swarm_amount = parse_bounded_f32(&args[0], 0.0, 1.0);
    let frames = parse_usize(&args[1]);
    let shape = args.get(2).map_or(2.0, |value| parse_shape(value));
    let voices = args.get(3).map_or(64, |value| parse_u8(value, 1, 64));
    let swarm_mode = args.get(4).map_or(SwarmMode::Wander, |value| {
        if value == "jitter" {
            SwarmMode::Jitter
        } else {
            SwarmMode::Wander
        }
    });
    let mut sequential = BenchEngine::new(
        Antialiasing::Spline,
        2,
        shape,
        voices,
        69,
        0.5,
        swarm_amount,
        0.7,
        16,
        swarm_mode,
        1,
    );
    let mut fused = BenchEngine::new(
        Antialiasing::Spline,
        2,
        shape,
        voices,
        69,
        0.5,
        swarm_amount,
        0.7,
        16,
        swarm_mode,
        1,
    );
    let mut maximum = 0.0_f64;
    let mut error_energy = 0.0_f64;
    let mut signal_energy = 0.0_f64;
    for _ in 0..frames {
        let reference = f64::from(sequential.next_sequential());
        let candidate = f64::from(fused.next());
        let error = candidate - reference;
        maximum = maximum.max(error.abs());
        error_energy = error.mul_add(error, error_energy);
        signal_energy = reference.mul_add(reference, signal_energy);
    }
    let rms_error = (error_energy / frames as f64).sqrt();
    let rms_signal = (signal_energy / frames as f64).sqrt();
    println!(
        "swarm_amount={swarm_amount},swarm_mode={swarm_mode:?},shape={shape},voices={voices},frames={frames},max_abs_error={maximum:.12e},rms_error={rms_error:.12e},rms_signal={rms_signal:.12e},error_db={:.3}",
        20.0 * (rms_error / rms_signal).log10()
    );
}

fn usage() -> ! {
    eprintln!(concat!(
        "usage:\n",
        "  generator_lab <bench|bench-pair|bench-pool> <legacy|spline|splineopt|lagrange|spectral> <1..4x> <triangle|saw|pulse|0..3> <1..64 voices> <frames> <repeats> [midi-note] [pulse-width] [swarm-amount] [swarm-rate] [polyphony] [noise|sine] [oscillators]\n",
        "  generator_lab bench-morph <serial|pool> <host-frames> <repeats> [off|noise|sine]\n",
        "  generator_lab bench-release <serial|pool> <host-frames> <repeats>\n",
        "  generator_lab bench-trigger <polyphony> <oscillators> <shape|random> <repeats>\n",
        "  generator_lab bench-lfo <1..8 active> <rate-hz> <internal-samples> <repeats>\n",
        "  generator_lab compare-spectral-pool <midi-note> <frames>\n",
        "  generator_lab idle-pool <seconds>\n",
        "  generator_lab compare-glide <triangle|saw|pulse|0..3> <start-hz> <end-hz> <frames> [pulse-width]\n",
        "  generator_lab sweep-live <polyphony>\n",
        "  generator_lab sweep-unison\n",
        "  generator_lab render <legacy|spline|splineopt|lagrange|spectral> <1..4x> <triangle|saw|pulse|0..3> <fft-bin> <samples> <output.f32> [pulse-width] [unison-voices] [none|pwm|bend|harm] [warp-amount] [oscillator]",
    ));
    std::process::exit(2);
}

fn bench_morph(args: &[String]) {
    if !(3..=4).contains(&args.len()) {
        usage();
    }
    let pooled = match args[0].as_str() {
        "serial" => false,
        "pool" => true,
        _ => usage(),
    };
    let host_frames = parse_usize(&args[1]);
    let repeats = parse_usize(&args[2]);
    let (swarm_amount, swarm_mode) = match args.get(3).map(String::as_str).unwrap_or("off") {
        "off" => (0.0, SwarmMode::Noise),
        "noise" => (1.0, SwarmMode::Noise),
        "sine" => (1.0, SwarmMode::Sine),
        _ => usage(),
    };
    let swarm_rate = std::env::var("KURV_LAB_SWARM_RATE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.7);
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0_f32;
    let mut participation = [0_u64; 4];
    let mut fallbacks = 0_u64;
    for _ in 0..repeats {
        let mut engine = BenchEngine::new(
            Antialiasing::SplineOptimized,
            2,
            0.0,
            64,
            69,
            0.5,
            swarm_amount,
            swarm_rate,
            24,
            swarm_mode,
            3,
        );
        let mut pool = InternalRtPool::new();
        let mut rendered = 0_usize;
        let mut position = 0.0_f32;
        let start = Instant::now();
        while rendered < host_frames {
            let shapes = std::array::from_fn(|_| {
                std::array::from_fn(|frame| {
                    let shape = (position + frame as f32 * 3.0 / (HOST_RATE * 2.0)).rem_euclid(3.0);
                    shape
                })
            });
            let block = if pooled {
                pool.render_morph_job::<BLOCK_INTERNAL_SAMPLES>(
                    &mut engine.synth,
                    engine.settings,
                    engine.envelope,
                    MAX_JOB_SAMPLES / BLOCK_INTERNAL_SAMPLES,
                    &shapes,
                )
                .expect("dense morph benchmark must stay pool eligible")
                .samples
            } else {
                let mut block = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
                for chunk in 0..MAX_JOB_SAMPLES / BLOCK_INTERNAL_SAMPLES {
                    let offset = chunk * BLOCK_INTERNAL_SAMPLES;
                    let chunk_shapes = std::array::from_fn(|oscillator| {
                        std::array::from_fn(|frame| shapes[oscillator][offset + frame])
                    });
                    let samples = engine.synth.render_morph_block::<BLOCK_INTERNAL_SAMPLES>(
                        engine.settings,
                        engine.envelope,
                        &chunk_shapes,
                    );
                    block[offset..offset + BLOCK_INTERNAL_SAMPLES].copy_from_slice(&samples);
                }
                block
            };
            checksum += block.iter().map(|sample| sample.0).sum::<f32>();
            position =
                (position + MAX_JOB_SAMPLES as f32 * 3.0 / (HOST_RATE * 2.0)).rem_euclid(3.0);
            rendered += MAX_JOB_SAMPLES / 2;
        }
        measurements.push(start.elapsed());
        participation = pool.worker_participation_all();
        fallbacks = pool.deadline_fallbacks();
    }
    measurements.sort_unstable();
    println!(
        "mode={},swarm={:?},swarm_rate={swarm_rate},host_frames={},repeats={},median_ns_per_frame={:.3},participation={participation:?},deadline_fallbacks={fallbacks},checksum={checksum:.9}",
        args[0],
        args.get(3).map(String::as_str).unwrap_or("off"),
        host_frames,
        repeats,
        nanos_per_frame(measurements[repeats / 2], host_frames),
    );
}

fn bench(args: &[String], block_major: bool, internal_pool: bool) {
    if !(6..=13).contains(&args.len()) {
        usage();
    }
    let algorithm = parse_algorithm(&args[0]);
    let factor = parse_u8(&args[1], 1, 4);
    let shape = parse_shape(&args[2]);
    let voices = parse_u8(&args[3], 1, 64);
    let frames = parse_usize(&args[4]);
    let repeats = parse_usize(&args[5]);
    let note = args.get(6).map_or(69, |value| parse_u8(value, 0, 127));
    let pulse_width = args
        .get(7)
        .map_or(0.5, |value| parse_bounded_f32(value, 0.03, 0.97));
    let swarm_amount = args
        .get(8)
        .map_or(0.0, |value| parse_bounded_f32(value, 0.0, 1.0));
    let swarm_rate = args
        .get(9)
        .map_or(0.7, |value| parse_bounded_f32(value, 0.02, 100.0));
    let polyphony = args.get(10).map_or(1, |value| parse_u8(value, 1, 32));
    let swarm_mode = args
        .get(11)
        .map_or(SwarmMode::Noise, |value| match value.as_str() {
            "noise" | "wander" | "jitter" => SwarmMode::Noise,
            "sine" => SwarmMode::Sine,
            _ => usage(),
        });
    let oscillators = args.get(12).map_or(1, |value| parse_u8(value, 1, 3));

    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0_f32;
    let mut participation = [0_u64; 4];
    let mut fifo = [false; 4];
    let mut deadline_fallbacks = 0_u64;
    for _ in 0..repeats {
        let mut engine = BenchEngine::new(
            algorithm,
            factor,
            shape,
            voices,
            note,
            pulse_width,
            swarm_amount,
            swarm_rate,
            polyphony,
            swarm_mode,
            oscillators,
        );
        engine.block_major &= block_major;
        if internal_pool && engine.block_major {
            engine.pool = Some(InternalRtPool::new());
            engine.pool_chunks = std::env::var("KURV_LAB_POOL_CHUNKS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(MAX_JOB_SAMPLES / engine.block_samples)
                .clamp(1, MAX_JOB_SAMPLES / engine.block_samples);
            engine.block_frames = engine.block_samples * engine.pool_chunks / usize::from(factor);
            engine.block_index = engine.block_frames;
        }
        for _ in 0..16_384 {
            checksum += black_box(engine.next());
        }
        let start = Instant::now();
        for _ in 0..frames {
            checksum += black_box(engine.next());
        }
        measurements.push(start.elapsed());
        if let Some(pool) = &engine.pool {
            participation = pool.worker_participation_all();
            fifo = pool.fifo_workers_all();
            deadline_fallbacks = pool.deadline_fallbacks();
        }
    }
    measurements.sort_unstable();
    let median = measurements[measurements.len() / 2];
    let minimum = measurements[0];
    let maximum = measurements[measurements.len() - 1];
    println!(
        "algorithm={},factor={},waveform={},oscillators={},voices={},polyphony={},note={},swarm_amount={},swarm_rate={},swarm_mode={:?},frames={},repeats={},median_ns_per_frame={:.3},min_ns_per_frame={:.3},max_ns_per_frame={:.3},voice_bytes={},participation={:?},fifo={:?},deadline_fallbacks={},checksum={:.9}",
        args[0],
        factor,
        args[2],
        oscillators,
        voices,
        polyphony,
        note,
        swarm_amount,
        swarm_rate,
        swarm_mode,
        frames,
        repeats,
        nanos_per_frame(median, frames),
        nanos_per_frame(minimum, frames),
        nanos_per_frame(maximum, frames),
        std::mem::size_of::<VaVoice>(),
        participation,
        fifo,
        deadline_fallbacks,
        black_box(checksum)
    );
}

fn compare_spectral_pool(args: &[String]) {
    if args.len() != 2 {
        usage();
    }
    let note = parse_u8(&args[0], 0, 127);
    let frames = parse_usize(&args[1]);
    let mut serial = BenchEngine::new(
        Antialiasing::Spectral,
        2,
        2.0,
        64,
        note,
        0.5,
        0.0,
        0.7,
        24,
        SwarmMode::Noise,
        3,
    );
    let mut pooled = BenchEngine::new(
        Antialiasing::Spectral,
        2,
        2.0,
        64,
        note,
        0.5,
        0.0,
        0.7,
        24,
        SwarmMode::Noise,
        3,
    );
    pooled.pool = Some(InternalRtPool::new());
    pooled.pool_chunks = MAX_JOB_SAMPLES / pooled.block_samples;
    pooled.block_frames = pooled.block_samples * pooled.pool_chunks / usize::from(pooled.factor);
    pooled.block_index = pooled.block_frames;

    let mut maximum = 0.0_f32;
    let mut mismatches = 0_usize;
    for _ in 0..frames {
        let expected = serial.next();
        let actual = pooled.next();
        maximum = maximum.max((actual - expected).abs());
        mismatches += usize::from(actual.to_bits() != expected.to_bits());
    }
    let pool = pooled.pool.as_ref().expect("pool is configured");
    println!(
        "note={note},frames={frames},bit_mismatches={mismatches},max_abs_error={maximum:.12e},participation={:?},deadline_fallbacks={}",
        pool.worker_participation(),
        pool.deadline_fallbacks(),
    );
}

fn render(args: &[String]) {
    if !(6..=11).contains(&args.len()) {
        usage();
    }
    let algorithm = parse_algorithm(&args[0]);
    let factor = parse_u8(&args[1], 1, 4);
    let shape = parse_shape(&args[2]);
    let fft_bin = parse_usize(&args[3]);
    let samples = parse_usize(&args[4]);
    let pulse_width = args
        .get(6)
        .map_or(0.5, |value| parse_bounded_f32(value, 0.03, 0.97));
    let voices = args.get(7).map_or(1, |value| parse_u8(value, 1, 64));
    let warp_mode = args
        .get(8)
        .map_or(PhaseWarpMode::None, |value| match value.as_str() {
            "none" => PhaseWarpMode::None,
            "pwm" => PhaseWarpMode::Pwm,
            "bend" => PhaseWarpMode::PhaseBend,
            "harm" => PhaseWarpMode::Harmonic,
            _ => usage(),
        });
    let warp_amount = args
        .get(9)
        .map_or(0.0, |value| parse_bounded_f32(value, 0.0, 1.0));
    let oscillator = args.get(10).map_or(1, |value| parse_u8(value, 1, 3));
    if fft_bin == 0 || fft_bin >= samples / 2 {
        eprintln!("fft-bin must be in 1..samples/2");
        std::process::exit(2);
    }
    let frequency = HOST_RATE * fft_bin as f32 / samples as f32;
    let mut engine = RenderEngine::new(
        algorithm,
        factor,
        shape,
        frequency,
        pulse_width,
        voices,
        warp_mode,
        warp_amount,
        oscillator,
    );
    for _ in 0..16_384 {
        black_box(engine.next());
    }
    let file = File::create(&args[5]).unwrap_or_else(|error| {
        eprintln!("could not create {}: {error}", args[5]);
        std::process::exit(1);
    });
    let mut output = BufWriter::new(file);
    for _ in 0..samples {
        output
            .write_all(&engine.next().to_le_bytes())
            .unwrap_or_else(|error| {
                eprintln!("could not write {}: {error}", args[5]);
                std::process::exit(1);
            });
    }
    output.flush().unwrap_or_else(|error| {
        eprintln!("could not finish {}: {error}", args[5]);
        std::process::exit(1);
    });
    println!(
        "algorithm={},factor={},waveform={},fft_bin={},samples={},sample_rate={},frequency_hz={:.9},output={}",
        args[0], args[1], args[2], fft_bin, samples, HOST_RATE, frequency, args[5]
    );
}

struct BenchEngine {
    synth: PolySynth,
    oversampler: StereoOversampler,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    factor: u8,
    block_major: bool,
    block_output: [f32; MAX_JOB_SAMPLES],
    block_samples: usize,
    block_frames: usize,
    block_index: usize,
    pool: Option<InternalRtPool>,
    pool_chunks: usize,
}

impl BenchEngine {
    fn new(
        algorithm: Antialiasing,
        factor: u8,
        shape: f32,
        voices: u8,
        note: u8,
        pulse_width: f32,
        swarm_amount: f32,
        swarm_rate: f32,
        polyphony: u8,
        swarm_mode: SwarmMode,
        oscillator_count: u8,
    ) -> Self {
        let algorithm = algorithm.for_factor(factor);
        let sample_rate = HOST_RATE * f32::from(factor);
        let mut synth = PolySynth::default();
        synth.set_sample_rate(sample_rate);
        synth.reset();
        let detune_cents = std::env::var("KURV_LAB_DETUNE_CENTS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(17.0);
        let detune_amount = std::env::var("KURV_LAB_DETUNE_AMOUNT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1.0);
        let unison = UnisonSettings::new(voices, detune_cents, 0.0, 0.0, 0.0)
            .with_detune_amount(detune_amount)
            .with_swarm(swarm_amount, swarm_rate)
            .with_swarm_mode(swarm_mode);
        synth.configure_unison(unison);
        for oscillator in 1..3 {
            synth.configure_secondary_unison(oscillator, unison);
        }
        synth.configure_oscillator_enabled(std::array::from_fn(|index| {
            index < usize::from(oscillator_count)
        }));
        for index in 0..polyphony {
            synth.note_on(note.saturating_add(index), 1.0, 0, None);
        }
        let block_samples = if factor == 3 {
            FACTOR3_BLOCK_INTERNAL_SAMPLES
        } else if swarm_amount > f32::EPSILON && swarm_mode == SwarmMode::Wander {
            WANDER_BLOCK_INTERNAL_SAMPLES
        } else {
            BLOCK_INTERNAL_SAMPLES
        };
        let block_frames = block_samples / usize::from(factor);
        let custom = std::env::var_os("KURV_LAB_CUSTOM").is_some();
        let warp_mode = match std::env::var("KURV_LAB_WARP").as_deref() {
            Ok("pwm") => PhaseWarpMode::Pwm,
            Ok("harm") => PhaseWarpMode::Harmonic,
            Ok(_) => PhaseWarpMode::PhaseBend,
            Err(_) => PhaseWarpMode::None,
        };
        synth.configure_phase_warp_modes([warp_mode; 3]);
        let mut settings = VoiceSettings::new(shape, 440.0, pulse_width, 0.0, 0.0, 0.0)
            .with_antialiasing(algorithm)
            .with_oscillators(std::array::from_fn(|index| {
                let oscillator = OscillatorSettings::new(
                    index < usize::from(oscillator_count),
                    shape,
                    pulse_width,
                    1.0,
                    1.0,
                    0.0,
                )
                .with_phase_warp(warp_mode, 0.98);
                if custom {
                    oscillator.with_custom_curve(WaveCurveRt::default(), 1.0)
                } else {
                    oscillator
                }
            }));
        let spectral_compatibility = algorithm == Antialiasing::Spectral
            && (settings.spectral_warp_compatibility(false)
                || synth.spectral_low_fallback_eligible(settings, false));
        if spectral_compatibility {
            settings.antialiasing = Antialiasing::SplineOptimized;
        }
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(matches!(
            settings.antialiasing,
            Antialiasing::SplineOptimized
        ));
        let block_major = synth.block_internal_samples(settings, factor).is_some();
        Self {
            synth,
            oversampler,
            settings,
            envelope: EnvelopeSettings::default(),
            factor,
            block_major,
            block_output: [0.0; MAX_JOB_SAMPLES],
            block_samples,
            block_frames,
            block_index: block_frames,
            pool: None,
            pool_chunks: 1,
        }
    }

    fn next(&mut self) -> f32 {
        if self.block_major {
            if self.block_index == self.block_frames {
                if self.block_samples == WANDER_BLOCK_INTERNAL_SAMPLES {
                    self.refill_block::<WANDER_BLOCK_INTERNAL_SAMPLES>();
                } else if self.factor == 3 {
                    self.refill_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>();
                } else {
                    self.refill_block::<BLOCK_INTERNAL_SAMPLES>();
                }
                self.block_index = 0;
            }
            let sample = self.block_output[self.block_index];
            self.block_index += 1;
            return sample;
        }
        if self.factor == 2 && self.settings.antialiasing != Antialiasing::Spectral {
            for (left, right) in self.synth.render_pair(self.settings, self.envelope) {
                self.oversampler.push(left, right);
            }
            return self.oversampler.output().0;
        }
        for _ in 0..usize::from(self.factor) {
            let (left, right) = self.synth.render(self.settings, self.envelope);
            self.oversampler.push(left, right);
        }
        self.oversampler.output().0
    }

    fn refill_block<const SAMPLES: usize>(&mut self) {
        let chunks = self.pool_chunks;
        let len = SAMPLES * chunks;
        let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
        let pooled = self.pool.as_mut().and_then(|pool| {
            pool.render_block_job::<SAMPLES>(&mut self.synth, self.settings, self.envelope, chunks)
        });
        if let Some(block) = pooled {
            debug_assert_eq!(block.len, len);
            samples = block.samples;
        } else {
            for chunk in 0..chunks {
                let rendered = self
                    .synth
                    .render_block::<SAMPLES>(self.settings, self.envelope);
                samples[chunk * SAMPLES..(chunk + 1) * SAMPLES].copy_from_slice(&rendered);
            }
        }
        let factor = usize::from(self.factor);
        for frame in 0..len / factor {
            if factor == 1 {
                self.block_output[frame] = self
                    .oversampler
                    .process_direct(samples[frame].0, samples[frame].1)
                    .0;
            } else {
                for (left, right) in samples[frame * factor..(frame + 1) * factor]
                    .iter()
                    .copied()
                {
                    self.oversampler.push(left, right);
                }
                self.block_output[frame] = self.oversampler.output().0;
            }
        }
    }

    fn next_sequential(&mut self) -> f32 {
        if self.factor == 1 {
            let (left, right) = self.synth.render(self.settings, self.envelope);
            return self.oversampler.process_direct(left, right).0;
        }
        for _ in 0..usize::from(self.factor) {
            let (left, right) = self.synth.render(self.settings, self.envelope);
            self.oversampler.push(left, right);
        }
        self.oversampler.output().0
    }
}

struct RenderEngine {
    voice: VaVoice,
    oversampler: StereoOversampler,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    sample_rate: f32,
    factor: u8,
}

impl RenderEngine {
    fn new(
        algorithm: Antialiasing,
        factor: u8,
        shape: f32,
        frequency: f32,
        pulse_width: f32,
        voices: u8,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        oscillator: u8,
    ) -> Self {
        let algorithm = algorithm.for_factor(factor);
        let sample_rate = HOST_RATE * f32::from(factor);
        let mut voice = VaVoice::default();
        voice.set_sample_rate(sample_rate);
        let unison = UnisonSettings::new(voices, 0.0, 0.0, 0.0, 0.0);
        voice.configure_unison(unison);
        for secondary in 1..3 {
            voice.configure_secondary_unison(secondary, unison);
        }
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler
            .set_spline_correction_immediate(matches!(algorithm, Antialiasing::SplineOptimized));
        let custom = std::env::var_os("KURV_LAB_CUSTOM").is_some();
        let oscillators = std::array::from_fn(|index| {
            let oscillator = OscillatorSettings::new(
                index + 1 == usize::from(oscillator),
                shape,
                pulse_width,
                1.0,
                1.0,
                0.0,
            )
            .with_phase_warp(warp_mode, warp_amount);
            if custom {
                oscillator.with_custom_curve(WaveCurveRt::default(), 1.0)
            } else {
                oscillator
            }
        });
        Self {
            voice,
            oversampler,
            settings: VoiceSettings::new(shape, frequency, pulse_width, 0.0, 0.0, 0.0)
                .with_antialiasing(algorithm)
                .with_oscillators(oscillators),
            envelope: EnvelopeSettings::default(),
            sample_rate,
            factor,
        }
    }

    fn next(&mut self) -> f32 {
        if self.factor == 1 {
            let (left, right) = self.voice.render(self.settings, self.sample_rate, true);
            return self.oversampler.process_direct(left, right).0;
        }
        for _ in 0..usize::from(self.factor) {
            let (left, right) = self.voice.render(self.settings, self.sample_rate, true);
            self.oversampler.push(left, right);
        }
        self.oversampler.output().0
    }
}

fn parse_algorithm(value: &str) -> Antialiasing {
    match value {
        "legacy" => Antialiasing::Legacy,
        "spline" => Antialiasing::Spline,
        "splineopt" => Antialiasing::SplineOptimized,
        "lagrange" => Antialiasing::Lagrange,
        "spectral" => Antialiasing::Spectral,
        _ => usage(),
    }
}

fn parse_shape(value: &str) -> f32 {
    match value {
        "triangle" => 1.0,
        "saw" => 2.0,
        "pulse" => 3.0,
        _ => value
            .parse::<f32>()
            .ok()
            .filter(|shape| (0.0..=3.0).contains(shape))
            .unwrap_or_else(|| usage()),
    }
}

fn parse_u8(value: &str, minimum: u8, maximum: u8) -> u8 {
    value
        .parse::<u8>()
        .ok()
        .filter(|value| (*value >= minimum) && (*value <= maximum))
        .unwrap_or_else(|| usage())
}

fn parse_usize(value: &str) -> usize {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(|| usage())
}

fn parse_bounded_f32(value: &str, minimum: f32, maximum: f32) -> f32 {
    value
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && (*value >= minimum) && (*value <= maximum))
        .unwrap_or_else(|| usage())
}

fn nanos_per_frame(duration: Duration, frames: usize) -> f64 {
    duration.as_secs_f64() * 1.0e9 / frames as f64
}
