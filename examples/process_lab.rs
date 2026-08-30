use std::hint::black_box;
use std::time::Instant;

use pure_va_dispersion_core::generators::{
    FilterConfig, FilterMode, GeneratorModMode, OscillatorConfig,
};
use pure_va_dispersion_core::{
    FilterControl, Kurv, KurvDspState, KurvParams, ModulationRouteTarget, OscillatorControl,
};
use truce::prelude::*;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 && args.len() != 5 && args.len() != 6 && args.len() != 7 {
        usage();
    }
    let frames = parse_usize(&args[0]);
    let callbacks = parse_usize(&args[1]);
    let repeats = parse_usize(&args[2]);
    let scenario = args.get(3).map_or("idle", String::as_str);
    let voices = args.get(4).map_or(0, |value| parse_usize(value)).min(64);
    let sample_rate = args
        .get(5)
        .map_or(48_000.0, |value| parse_sample_rate(value));
    let oversampling = args
        .get(6)
        .map_or(2, |value| parse_usize(value))
        .clamp(1, 4);

    let params = KurvParams::default();
    configure_scenario(&params, scenario);
    params.oversampling.set_value(oversampling as i64);
    params.set_sample_rate(sample_rate);
    params.snap_smoothers();
    let mut state = KurvDspState::default();
    <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(sample_rate, frames));

    let input_events = EventList::with_capacity(0);
    let mut note_events = EventList::with_capacity(voices);
    for voice in 0..voices {
        note_events.push(Event::new(
            0,
            EventBody::NoteOn {
                group: 0,
                channel: 1,
                note: 36 + voice as u8,
                velocity: 127,
            },
        ));
    }
    let mut output_events = EventList::with_capacity(0);
    let transport = TransportInfo::default();
    let mut context = ProcessContext::new(&transport, sample_rate, frames, &mut output_events);
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let inputs: [&[f32]; 0] = [];
    let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
    let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);

    black_box(<Kurv as PluginLogic>::process(
        &mut state,
        &params,
        &mut buffer,
        &note_events,
        &mut context,
    ));

    let warmup_callbacks = if scenario.starts_with("stress4") {
        16
    } else {
        256
    };
    for _ in 0..warmup_callbacks {
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
    let finite = left.iter().chain(&right).all(|sample| sample.is_finite());
    let peak = left
        .iter()
        .chain(&right)
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let checksum = left
        .iter()
        .chain(&right)
        .enumerate()
        .fold(0.0_f64, |sum, (index, sample)| {
            (*sample as f64).mul_add(index as f64 + 1.0, sum)
        });
    println!(
        "scenario={scenario},voices={voices},frames={frames},sample_rate={sample_rate:.0},callbacks={callbacks},repeats={repeats},median_ns_per_callback={:.3},median_ns_per_frame={:.3},finite={finite},peak={peak:.6},checksum={checksum:.9}",
        median.as_nanos() as f64 / callbacks as f64,
        median.as_nanos() as f64 / (callbacks * frames) as f64,
    );
}

fn configure_scenario(params: &KurvParams, scenario: &str) {
    if let Some(modulation) = scenario.strip_prefix("stress4") {
        configure_stress(params, modulation);
        return;
    }
    let (scenario, filter_modulation) = [
        ("-morph-mod", FilterControl::Morph),
        ("-slope-mod", FilterControl::Slope),
        ("-q-mod", FilterControl::Resonance),
        ("-mod", FilterControl::Cutoff),
    ]
    .into_iter()
    .find_map(|(suffix, control)| scenario.strip_suffix(suffix).map(|name| (name, control)))
    .map_or((scenario, None), |(name, control)| (name, Some(control)));
    if matches!(
        scenario,
        "osc" | "custom" | "noise" | "dual" | "pm" | "am" | "rm" | "pan"
    ) {
        if matches!(scenario, "dual" | "pm" | "am" | "rm" | "pan") {
            let group = params.generator_stack.snapshot().groups()[0].id();
            params
                .generator_stack
                .edit(|patch| patch.insert_oscillator(group, 1))
                .unwrap_or_else(|error| fail(&format!("failed to add PM carrier: {error:?}")));
            let snapshot = params.generator_stack.snapshot();
            let oscillators = snapshot.groups()[0]
                .modules()
                .iter()
                .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
                .collect::<Vec<_>>();
            if scenario != "dual" {
                let mut carrier = OscillatorConfig::default();
                carrier.phase_mod_source = oscillators[0].1.index() as u8 + 1;
                carrier.phase_mod_amount = 0.75;
                carrier.modulation_mode = match scenario {
                    "am" => GeneratorModMode::Amplitude,
                    "rm" => GeneratorModMode::Ring,
                    "pan" => GeneratorModMode::Pan,
                    _ => GeneratorModMode::Phase,
                };
                params
                    .generator_stack
                    .set_oscillator_config(oscillators[1].1, carrier);
                if filter_modulation.is_some() {
                    params.lfo1_active.set_value(true);
                    params.lfo1_rate.set_value(1_000.0);
                    params.lfo1_bipolar.set_value(true);
                    params.mod1_source.set_value(1);
                    params.mod1_amount.set_value(1.0);
                    params.modulation_route_targets.set(
                        0,
                        ModulationRouteTarget::oscillator(
                            oscillators[1].0,
                            oscillators[1].1,
                            OscillatorControl::PhaseModAmount,
                        ),
                    );
                }
            }
            return;
        }
        let module = params.generator_stack.snapshot().groups()[0].modules()[0].clone();
        let slot = module
            .oscillator_slot()
            .unwrap_or_else(|| fail("profiling oscillator slot was not published"));
        if scenario == "noise" {
            let config = OscillatorConfig::for_engine(
                pure_va_dispersion_core::generators::OscillatorEngineKind::Noise,
            );
            params.generator_stack.set_oscillator_config(slot, config);
        } else if scenario == "custom" {
            params.generator_stack.set_oscillator_config(
                slot,
                OscillatorConfig {
                    custom_shape: 1.0,
                    phase_random: 0.0,
                    ..OscillatorConfig::default()
                },
            );
        }
        if filter_modulation.is_some() {
            params.lfo1_active.set_value(true);
            params.lfo1_rate.set_value(17.0);
            params.lfo1_bipolar.set_value(true);
            params.mod1_source.set_value(1);
            params.mod1_amount.set_value(1.0);
            params.modulation_route_targets.set(
                0,
                ModulationRouteTarget::oscillator(module.id(), slot, OscillatorControl::Level),
            );
        }
        return;
    }
    let maximum = scenario.ends_with("-max");
    let mode = match scenario {
        "idle" | "osc" => return,
        "svf" | "svf-max" => FilterMode::Svf,
        "phaser" | "phaser-max" => FilterMode::Phaser,
        "scream" => FilterMode::Scream,
        _ => usage(),
    };
    let group = params.generator_stack.snapshot().groups()[0].id();
    let module = params
        .generator_stack
        .edit(|patch| patch.insert_filter(group, 1))
        .unwrap_or_else(|error| fail(&format!("failed to configure profiling filter: {error:?}")));
    let slot = params.generator_stack.snapshot().groups()[0]
        .modules()
        .iter()
        .find(|candidate| candidate.id() == module)
        .and_then(|module| module.filter_slot())
        .unwrap_or_else(|| fail("profiling filter slot was not published"));
    params.generator_stack.set_filter_config(
        slot,
        FilterConfig {
            mode,
            cutoff_hz: 1_000.0,
            q: 1.0,
            slope_db_oct: if maximum { 768.0 } else { 24.0 },
            morph: if maximum { 1.0 } else { 0.5 },
            shape: 0.0,
        },
    );
    if let Some(control) = filter_modulation {
        params.lfo1_active.set_value(true);
        params.lfo1_rate.set_value(17.0);
        params.lfo1_bipolar.set_value(true);
        params.mod1_source.set_value(1);
        params.mod1_amount.set_value(1.0);
        params
            .modulation_route_targets
            .set(0, ModulationRouteTarget::filter(module, slot, control));
    }
}

fn configure_stress(params: &KurvParams, modulation: &str) {
    let group = params.generator_stack.snapshot().groups()[0].id();
    for _ in 1..4 {
        let index = params.generator_stack.snapshot().groups()[0]
            .modules()
            .len();
        params
            .generator_stack
            .edit(|patch| patch.insert_oscillator(group, index))
            .unwrap_or_else(|error| {
                fail(&format!("failed to configure stress oscillator: {error:?}"))
            });
    }
    let snapshot = params.generator_stack.snapshot();
    let oscillators = snapshot.groups()[0]
        .modules()
        .iter()
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .take(4)
        .collect::<Vec<_>>();
    for (index, (_, slot)) in oscillators.iter().copied().enumerate() {
        params.generator_stack.set_oscillator_config(
            slot,
            OscillatorConfig {
                shape: [0.45, 1.35, 2.45, 1.75][index],
                pulse_width: 0.37,
                transpose: [0.0, 7.0, -12.0, 19.0][index],
                level: 0.25,
                unison_voices: 64,
                unison_range: 24.0,
                unison_amount: 0.8,
                unison_jitter: 0.75,
                unison_rate: 0.85,
                phase_position: 0.37,
                phase_warp_mode: 1,
                phase_warp_amount: 0.65,
                ..OscillatorConfig::default()
            },
        );
    }
    let control = match modulation {
        "" => return,
        "-filter" => {
            configure_stress_filter(params, false);
            return;
        }
        "-filter-mod" => {
            configure_stress_filter(params, true);
            return;
        }
        "-phase-mod" => OscillatorControl::PhasePosition,
        "-shape-mod" => OscillatorControl::Shape,
        "-warp-mod" => OscillatorControl::PhaseWarpAmount,
        _ => usage(),
    };
    params.lfo1_active.set_value(true);
    params.lfo1_rate.set_value(5_000.0);
    params.lfo1_bipolar.set_value(true);
    let sources = [
        &params.mod1_source,
        &params.mod2_source,
        &params.mod3_source,
        &params.mod4_source,
    ];
    let amounts = [
        &params.mod1_amount,
        &params.mod2_amount,
        &params.mod3_amount,
        &params.mod4_amount,
    ];
    for (route, ((module, slot), (source, amount))) in oscillators
        .into_iter()
        .zip(sources.into_iter().zip(amounts))
        .enumerate()
    {
        source.set_value(1);
        amount.set_value(1.0);
        params.modulation_route_targets.set(
            route,
            ModulationRouteTarget::oscillator(module, slot, control),
        );
    }
}

fn configure_stress_filter(params: &KurvParams, modulated: bool) {
    let group = params.generator_stack.snapshot().groups()[0].id();
    let index = params.generator_stack.snapshot().groups()[0]
        .modules()
        .len();
    let module = params
        .generator_stack
        .edit(|patch| patch.insert_filter(group, index))
        .unwrap_or_else(|error| fail(&format!("failed to configure stress filter: {error:?}")));
    let slot = params.generator_stack.snapshot().groups()[0]
        .modules()
        .iter()
        .find(|candidate| candidate.id() == module)
        .and_then(|module| module.filter_slot())
        .unwrap_or_else(|| fail("stress filter slot was not published"));
    params.generator_stack.set_filter_config(
        slot,
        FilterConfig {
            mode: FilterMode::Svf,
            cutoff_hz: 1_000.0,
            q: 1.0,
            slope_db_oct: 24.0,
            morph: 0.5,
            shape: 0.0,
        },
    );
    if modulated {
        params.lfo1_active.set_value(true);
        params.lfo1_rate.set_value(5_000.0);
        params.lfo1_bipolar.set_value(true);
        params.mod1_source.set_value(1);
        params.mod1_amount.set_value(1.0);
        params.modulation_route_targets.set(
            0,
            ModulationRouteTarget::filter(module, slot, FilterControl::Cutoff),
        );
    }
}

fn parse_usize(value: &str) -> usize {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value != 0)
        .unwrap_or_else(|| usage())
}

fn parse_sample_rate(value: &str) -> f64 {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 1.0)
        .unwrap_or_else(|| usage())
}

fn usage() -> ! {
    eprintln!(
        "usage: process_lab <frames> <callbacks> <repeats> [idle|osc|custom|noise|dual|pm|am|rm|pan|svf|svf-max|phaser|phaser-max|scream][-mod|-q-mod|-slope-mod|-morph-mod]|stress4[-phase-mod|-shape-mod|-warp-mod|-filter|-filter-mod] [voices] [sample-rate] [oversampling]"
    );
    std::process::exit(2);
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
