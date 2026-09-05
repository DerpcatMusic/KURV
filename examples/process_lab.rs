use std::hint::black_box;
use std::time::{Duration, Instant};

use pure_va_dispersion_core::generators::{
    FilterConfig, FilterMode, GeneratorModMode, ModuleId, OscillatorConfig, OscillatorEngineKind,
    OscillatorSlot, OscillatorTuningMode,
};
use pure_va_dispersion_core::{
    FilterControl, GroupControl, Kurv, KurvParams, ModulationRouteTarget, OscillatorControl,
    ResynthAlgorithm, ResynthControls, analyze_wav_with_root_override,
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
    let voices = args
        .get(4)
        .map_or(0, |value| {
            value.parse::<usize>().unwrap_or_else(|_| usage())
        })
        .min(64);
    let sample_rate = args
        .get(5)
        .map_or(48_000.0, |value| parse_sample_rate(value));
    let oversampling = args
        .get(6)
        .map_or(2, |value| parse_usize(value))
        .clamp(1, 4);

    let params = KurvParams::default();
    configure_scenario(&params, scenario);
    let drag_filter = scenario.ends_with("-drag").then(|| {
        let slot = params
            .generator_stack
            .snapshot()
            .groups()
            .iter()
            .flat_map(|group| group.modules())
            .find_map(|module| module.filter_slot())
            .unwrap_or_else(|| fail("drag scenario needs a filter"));
        (slot, params.generator_stack.filter_config(slot))
    });
    params.oversampling.set_value(oversampling as i64);
    params.set_sample_rate(sample_rate);
    params.snap_smoothers();
    let mut state = <Kurv as PluginLogic>::init(&params, &InitContext::new(None));
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
    let input_left = (0..frames)
        .map(|frame| {
            (std::f32::consts::TAU * 440.0 * frame as f32 / sample_rate as f32).sin() * 0.25
        })
        .collect::<Vec<_>>();
    let input_right = input_left.iter().map(|sample| -*sample).collect::<Vec<_>>();
    let inputs: [&[f32]; 2] = [&input_left, &input_right];
    let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
    let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);

    black_box(<Kurv as PluginLogic>::process(
        &mut state,
        &params,
        &mut buffer,
        &note_events,
        &mut context,
    ));

    let warmup_callbacks = if scenario.starts_with("rig-") {
        (2_032 / frames).saturating_sub(1)
    } else if scenario.starts_with("stress4") || scenario.starts_with("gfilter-") {
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
    let mut callback_measurements = Vec::with_capacity(repeats * callbacks);
    let mut audible_callbacks = 0;
    let mut finite = true;
    let mut peak = 0.0_f32;
    let mut stream_sum = 0.0_f64;
    let mut stream_energy = 0.0_f64;
    let mut drag_frame = 0;
    for _ in 0..repeats {
        let mut elapsed = Duration::ZERO;
        for _ in 0..callbacks {
            if let Some((slot, mut config)) = drag_filter {
                config.cutoff_hz = if drag_frame & 1 == 0 { 300.0 } else { 8_000.0 };
                params.generator_stack.set_filter_config(slot, config);
                drag_frame += 1;
            }
            let callback_start = Instant::now();
            black_box(<Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            ));
            let callback_elapsed = callback_start.elapsed();
            elapsed += callback_elapsed;
            callback_measurements.push(callback_elapsed);
            let audible = buffer.output(0).iter().any(|sample| *sample != 0.0)
                || buffer.output(1).iter().any(|sample| *sample != 0.0);
            audible_callbacks += usize::from(audible);
            for channel in 0..2 {
                for sample in buffer.output(channel) {
                    finite &= sample.is_finite();
                    peak = peak.max(sample.abs());
                    stream_sum += f64::from(*sample);
                    stream_energy = f64::from(*sample).mul_add(f64::from(*sample), stream_energy);
                }
            }
        }
        measurements.push(elapsed);
    }
    measurements.sort_unstable();
    callback_measurements.sort_unstable();
    let median = measurements[measurements.len() / 2];
    // Upper empirical quantiles: a 20-callback p95 is the maximum.
    let p50 = callback_measurements[callback_measurements.len() / 2];
    let p95 = callback_measurements[callback_measurements.len() * 95 / 100];
    let maximum = callback_measurements[callback_measurements.len() - 1];
    if voices != 0 && audible_callbacks == 0 {
        fail(&format!(
            "timed workload went silent: {audible_callbacks}/{} callbacks produced audio",
            repeats * callbacks
        ));
    }
    if !finite {
        fail("timed workload produced non-finite audio");
    }
    let checksum = left
        .iter()
        .chain(&right)
        .enumerate()
        .fold(0.0_f64, |sum, (index, sample)| {
            (*sample as f64).mul_add(index as f64 + 1.0, sum)
        });
    let tail_checksum = left.iter().zip(&right).rev().take(16).enumerate().fold(
        0.0_f64,
        |sum, (index, (left, right))| {
            (*right as f64).mul_add(
                (index * 2 + 2) as f64,
                (*left as f64).mul_add((index * 2 + 1) as f64, sum),
            )
        },
    );
    println!(
        "scenario={scenario},voices={voices},frames={frames},sample_rate={sample_rate:.0},callbacks={callbacks},repeats={repeats},median_ns_per_callback={:.3},median_ns_per_frame={:.3},p50_ns={:.0},p95_ns={:.0},max_ns={:.0},deadline_pct_p95={:.2},audible_callbacks={audible_callbacks},finite={finite},peak={peak:.6},checksum={checksum:.9},tail_checksum={tail_checksum:.9},stream_sum={stream_sum:.9},stream_energy={stream_energy:.9}",
        median.as_nanos() as f64 / callbacks as f64,
        median.as_nanos() as f64 / (callbacks * frames) as f64,
        p50.as_nanos(),
        p95.as_nanos(),
        maximum.as_nanos(),
        p95.as_secs_f64() * sample_rate * 100.0 / frames as f64,
    );
}

fn configure_scenario(params: &KurvParams, scenario: &str) {
    if let Some(algorithm) = scenario.strip_prefix("resynth-") {
        configure_resynth(params, algorithm);
        return;
    }
    if matches!(
        scenario,
        "group-one"
            | "group-two-same"
            | "group-two-split"
            | "group-empty"
            | "group-noise-same"
            | "group-noise"
            | "group-resynth-empty-same"
            | "group-resynth-empty"
    ) {
        configure_group_layout(params, scenario);
        return;
    }
    if matches!(scenario, "aux-input" | "aux-same" | "aux-group") {
        let snapshot = params.generator_stack.snapshot();
        let source_group = snapshot.groups()[0].id();
        let source_slot = snapshot.groups()[0].modules()[0]
            .oscillator_slot()
            .unwrap_or_else(|| fail("profiling AUX source oscillator was not published"));
        let destination = if scenario == "aux-group" {
            params
                .generator_stack
                .edit(|patch| patch.insert_group(patch.groups().len()))
                .unwrap_or_else(|error| fail(&format!("failed to add AUX group: {error:?}")))
        } else {
            source_group
        };
        let index = params
            .generator_stack
            .snapshot()
            .groups()
            .iter()
            .find(|group| group.id() == destination)
            .map_or(0, |group| group.modules().len());
        let module = params
            .generator_stack
            .edit(|patch| patch.insert_aux(destination, index))
            .unwrap_or_else(|error| fail(&format!("failed to add AUX: {error:?}")));
        let slot = params
            .generator_stack
            .snapshot()
            .groups()
            .iter()
            .flat_map(|group| group.modules())
            .find(|candidate| candidate.id() == module)
            .and_then(|module| module.aux_slot())
            .unwrap_or_else(|| fail("profiling AUX slot was not published"));
        let route = 16;
        let source = 65 + source_slot.index() as u8;
        assert!(params.modulation_route_overflow.set(route, source, 1.0));
        assert!(
            params
                .modulation_route_targets
                .set(route, ModulationRouteTarget::aux(module, slot))
        );
        return;
    }
    if let Some(modulation) = scenario.strip_prefix("stress4") {
        configure_stress(params, modulation);
        return;
    }
    if let Some(spec) = scenario.strip_prefix("rig-") {
        configure_rig(params, spec);
        return;
    }
    if let Some(spec) = scenario.strip_prefix("gfilter-") {
        configure_generator_filter_route(params, spec);
        return;
    }
    if let Some((control, lanes)) = [
        (Some(OscillatorControl::Transpose), "xfm-"),
        (Some(OscillatorControl::PhasePosition), "xpm-"),
        (Some(OscillatorControl::Level), "xam-"),
        (Some(OscillatorControl::RingModAmount), "xrm-"),
        (Some(OscillatorControl::Pan), "xpan-"),
        (None, "xoff-"),
    ]
    .into_iter()
    .find_map(|(control, prefix)| scenario.strip_prefix(prefix).map(|lanes| (control, lanes)))
    {
        configure_generator_route(params, control, lanes);
        return;
    }
    if let Some((kind, lanes)) = [
        ("self", "xselfpm-"),
        ("cycle", "xcyclepm-"),
        ("mixed", "xmixed-"),
        ("depth", "xdepthpm-"),
        ("self-depth", "xselfdepthpm-"),
        ("lfo-depth-fast", "xlfodepthfastpm-"),
        ("lfo-depth", "xlfodepthpm-"),
    ]
    .into_iter()
    .find_map(|(kind, prefix)| scenario.strip_prefix(prefix).map(|lanes| (kind, lanes)))
    {
        configure_generator_graph(params, kind, lanes);
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
        "osc"
            | "custom"
            | "noise"
            | "dual"
            | "dual-svf-middle"
            | "dual-svf-terminal"
            | "pm"
            | "am"
            | "rm"
            | "pan"
    ) {
        if matches!(
            scenario,
            "dual" | "dual-svf-middle" | "dual-svf-terminal" | "pm" | "am" | "rm" | "pan"
        ) {
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
            if scenario.starts_with("dual-svf") {
                let index = usize::from(scenario == "dual-svf-terminal") + 1;
                let module = params
                    .generator_stack
                    .edit(|patch| patch.insert_filter(group, index))
                    .unwrap_or_else(|error| {
                        fail(&format!("failed to add profiling filter: {error:?}"))
                    });
                let slot = params.generator_stack.snapshot().groups()[0]
                    .modules()
                    .iter()
                    .find(|candidate| candidate.id() == module)
                    .and_then(|module| module.filter_slot())
                    .unwrap_or_else(|| fail("profiling filter slot was not published"));
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
                return;
            }
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

fn configure_resynth(params: &KurvParams, algorithm: &str) {
    let (algorithm, variant) = algorithm.split_once('-').unwrap_or((algorithm, ""));
    let algorithm = match algorithm {
        "sample" => ResynthAlgorithm::Sample,
        "grain" => ResynthAlgorithm::Grain,
        "rich" => ResynthAlgorithm::Rich,
        _ => usage(),
    };
    let controls = ResynthControls::default();
    let model = analyze_wav_with_root_override(
        "process-lab.wav",
        profiling_tone_wav(),
        controls,
        Some(220.0),
    )
    .unwrap_or_else(|error| fail(&format!("failed to analyze profiling source: {error:?}")));
    params
        .resynth_assets
        .slot(0)
        .unwrap_or_else(|| fail("profiling RESYNTH slot is missing"))
        .replace(model, algorithm, controls)
        .unwrap_or_else(|error| fail(&format!("failed to build profiling source: {error:?}")));
    let slot = params.generator_stack.snapshot().groups()[0].modules()[0]
        .oscillator_slot()
        .unwrap_or_else(|| fail("default profiling oscillator is missing"));
    params.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig {
            level: 0.5,
            ..OscillatorConfig::for_engine(OscillatorEngineKind::Resynth)
        },
    );
    match variant {
        "" => {}
        "2g" => configure_group_layout(params, "group-two-split"),
        "svf" => configure_stress_filter(params, false, FilterMode::Svf),
        "svf-mod" => configure_stress_filter(params, true, FilterMode::Svf),
        _ => usage(),
    }
}

fn profiling_tone_wav() -> Vec<u8> {
    const FRAMES: u32 = 12_000;
    let mut wav = Vec::with_capacity(44 + FRAMES as usize * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + FRAMES * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
    wav.extend_from_slice(&(FRAMES * 2).to_le_bytes());
    for frame in 0..FRAMES {
        let phase = std::f32::consts::TAU * 220.0 * frame as f32 / 48_000.0;
        let sample = (phase.sin() * 0.7 + (phase * 2.0).sin() * 0.2) * 24_000.0;
        wav.extend_from_slice(&(sample as i16).to_le_bytes());
    }
    wav
}

fn configure_group_layout(params: &KurvParams, scenario: &str) {
    let first_group = params.generator_stack.snapshot().groups()[0].id();
    let first_slot = params.generator_stack.snapshot().groups()[0].modules()[0]
        .oscillator_slot()
        .unwrap_or_else(|| fail("default profiling oscillator was not published"));
    params.generator_stack.set_oscillator_config(
        first_slot,
        OscillatorConfig {
            phase_random: 0.0,
            ..OscillatorConfig::default()
        },
    );
    if scenario == "group-one" {
        return;
    }
    let second_group = if matches!(
        scenario,
        "group-two-same" | "group-noise-same" | "group-resynth-empty-same"
    ) {
        first_group
    } else {
        params
            .generator_stack
            .edit(|patch| patch.insert_group(1))
            .unwrap_or_else(|error| fail(&format!("failed to add profiling group: {error:?}")))
    };
    if scenario == "group-empty" {
        return;
    }
    let module = params
        .generator_stack
        .edit(|patch| {
            let index = patch
                .groups()
                .iter()
                .find(|group| group.id() == second_group)
                .map_or(0, |group| group.modules().len());
            patch.insert_oscillator(second_group, index)
        })
        .unwrap_or_else(|error| fail(&format!("failed to add profiling oscillator: {error:?}")));
    let slot = params
        .generator_stack
        .snapshot()
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .find(|candidate| candidate.id() == module)
        .and_then(|module| module.oscillator_slot())
        .unwrap_or_else(|| fail("profiling oscillator slot was not published"));
    let engine = match scenario {
        "group-noise" | "group-noise-same" => OscillatorEngineKind::Noise,
        "group-resynth-empty" | "group-resynth-empty-same" => OscillatorEngineKind::Resynth,
        "group-two-same" | "group-two-split" => OscillatorEngineKind::Va,
        _ => unreachable!(),
    };
    let mut config = OscillatorConfig::for_engine(engine);
    config.phase_random = 0.0;
    params.generator_stack.set_oscillator_config(slot, config);
}

fn configure_generator_route(params: &KurvParams, control: Option<OscillatorControl>, lanes: &str) {
    let (source_voices, target_voices) = lanes
        .split_once('x')
        .and_then(|(source, target)| Some((source.parse::<u8>().ok()?, target.parse::<u8>().ok()?)))
        .filter(|&(source, target)| (1..=64).contains(&source) && (1..=64).contains(&target))
        .unwrap_or_else(|| usage());
    let group = params.generator_stack.snapshot().groups()[0].id();
    params
        .generator_stack
        .edit(|patch| patch.insert_oscillator(group, 1))
        .unwrap_or_else(|error| fail(&format!("failed to add modulation carrier: {error:?}")));
    let snapshot = params.generator_stack.snapshot();
    let oscillators = snapshot.groups()[0]
        .modules()
        .iter()
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .collect::<Vec<_>>();
    for ((_, slot), voices) in oscillators
        .iter()
        .copied()
        .zip([source_voices, target_voices])
    {
        let mut config = params.generator_stack.oscillator_config(slot);
        config.phase_random = 0.0;
        config.unison_voices = voices;
        params.generator_stack.set_oscillator_config(slot, config);
    }
    if let Some(control) = control {
        let route = 16;
        let source = 65 + oscillators[0].1.index() as u8;
        assert!(params.modulation_route_overflow.set(route, source, 1.0));
        assert!(params.modulation_route_targets.set(
            route,
            ModulationRouteTarget::oscillator(oscillators[1].0, oscillators[1].1, control),
        ));
    }
}

fn configure_generator_filter_route(params: &KurvParams, spec: &str) {
    let mut parts = spec.split('-');
    let mode = match parts.next().unwrap_or_default() {
        "svf" => FilterMode::Svf,
        "phaser" => FilterMode::Phaser,
        "scream" => FilterMode::Scream,
        "ratio" => FilterMode::RatioBrickwall,
        _ => usage(),
    };
    let control = match parts.next().unwrap_or_default() {
        "none" => None,
        "cutoff" => Some(FilterControl::Cutoff),
        "resonance" => Some(FilterControl::Resonance),
        "slope" => Some(FilterControl::Slope),
        "morph" => Some(FilterControl::Morph),
        "shape" => Some(FilterControl::Shape),
        _ => usage(),
    };
    let lanes = parts.next().unwrap_or_else(|| usage());
    let depth = match parts.next() {
        None => false,
        Some("depth") => true,
        _ => usage(),
    };
    if parts.next().is_some() {
        usage();
    }
    let (source_voices, target_voices) = lanes
        .split_once('x')
        .and_then(|(source, target)| Some((source.parse::<u8>().ok()?, target.parse::<u8>().ok()?)))
        .filter(|&(source, target)| (1..=64).contains(&source) && (1..=64).contains(&target))
        .unwrap_or_else(|| usage());
    let group = params.generator_stack.snapshot().groups()[0].id();
    params
        .generator_stack
        .edit(|patch| patch.insert_oscillator(group, 1))
        .unwrap_or_else(|error| {
            fail(&format!(
                "failed to add filter-modulation carrier: {error:?}"
            ))
        });
    let snapshot = params.generator_stack.snapshot();
    let oscillators = snapshot.groups()[0]
        .modules()
        .iter()
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .collect::<Vec<_>>();
    for ((_, slot), voices) in oscillators
        .iter()
        .copied()
        .zip([source_voices, target_voices])
    {
        let mut config = params.generator_stack.oscillator_config(slot);
        config.phase_random = 0.0;
        config.unison_voices = voices;
        params.generator_stack.set_oscillator_config(slot, config);
    }
    let module = params
        .generator_stack
        .edit(|patch| patch.insert_filter(group, 2))
        .unwrap_or_else(|error| {
            fail(&format!(
                "failed to add filter-modulation target: {error:?}"
            ))
        });
    let slot = params.generator_stack.snapshot().groups()[0]
        .modules()
        .iter()
        .find(|candidate| candidate.id() == module)
        .and_then(|module| module.filter_slot())
        .unwrap_or_else(|| fail("filter-modulation target slot was not published"));
    params.generator_stack.set_filter_config(
        slot,
        FilterConfig {
            mode,
            cutoff_hz: if mode == FilterMode::RatioBrickwall {
                12.0
            } else {
                1_000.0
            },
            q: 1.0,
            slope_db_oct: 24.0,
            morph: 0.5,
            shape: 0.5,
        },
    );
    if let Some(control) = control {
        let route = 16;
        let source = 65 + oscillators[0].1.index() as u8;
        assert!(params.modulation_route_overflow.set(route, source, 0.25));
        assert!(
            params
                .modulation_route_targets
                .set(route, ModulationRouteTarget::filter(module, slot, control),)
        );
        if depth {
            assert!(params.modulation_route_overflow.set(
                route + 1,
                65 + oscillators[1].1.index() as u8,
                0.25,
            ));
            assert!(
                params
                    .modulation_route_targets
                    .set(route + 1, ModulationRouteTarget::route_depth(route),)
            );
        }
    }
}

fn configure_generator_graph(params: &KurvParams, kind: &str, lanes: &str) {
    configure_generator_route(params, None, lanes);
    let snapshot = params.generator_stack.snapshot();
    let oscillators = snapshot.groups()[0]
        .modules()
        .iter()
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .collect::<Vec<_>>();
    let source = |slot: usize| 65 + oscillators[slot].1.index() as u8;
    let target = |slot: usize, control| {
        ModulationRouteTarget::oscillator(oscillators[slot].0, oscillators[slot].1, control)
    };
    let set = |route, source, amount, target| {
        assert!(params.modulation_route_overflow.set(route, source, amount));
        assert!(params.modulation_route_targets.set(route, target));
    };
    match kind {
        "self" => set(
            16,
            source(0),
            0.7,
            target(0, OscillatorControl::PhasePosition),
        ),
        "cycle" => {
            set(
                16,
                source(0),
                0.7,
                target(1, OscillatorControl::PhasePosition),
            );
            set(
                17,
                source(1),
                0.7,
                target(0, OscillatorControl::PhasePosition),
            );
        }
        "mixed" => {
            set(16, source(0), 0.7, target(1, OscillatorControl::Level));
            set(
                17,
                source(0),
                0.7,
                target(1, OscillatorControl::PhasePosition),
            );
        }
        "depth" | "self-depth" => {
            set(
                16,
                source(0),
                0.7,
                target(1, OscillatorControl::PhasePosition),
            );
            set(
                17,
                source(usize::from(kind == "depth")),
                0.5,
                ModulationRouteTarget::route_depth(16),
            );
        }
        "lfo-depth" | "lfo-depth-fast" => {
            set(
                16,
                source(0),
                0.7,
                target(1, OscillatorControl::PhasePosition),
            );
            params.lfo1_active.set_value(true);
            params.lfo1_rate.set_value(1_000.0);
            params.lfo1_mode.set_value(2);
            params.lfo1_bipolar.set_value(true);
            params.mod1_source.set_value(1);
            params.mod1_amount.set_value(0.5);
            assert!(
                params
                    .modulation_route_targets
                    .set(0, ModulationRouteTarget::route_depth(16))
            );
            if kind == "lfo-depth-fast"
                && let Ok(mut editor) = params.editor_state.lock()
            {
                editor.fast_audio_rate_modulation = true;
            }
        }
        _ => unreachable!(),
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
    if matches!(
        modulation,
        "-nojitter" | "-ratio-nojitter" | "-nowarp" | "-ratio-nowarp"
    ) {
        for (_, slot) in oscillators {
            let mut config = params.generator_stack.oscillator_config(slot);
            if modulation.ends_with("nojitter") {
                config.unison_jitter = 0.0;
            } else {
                config.phase_warp_amount = 0.0;
            }
            params.generator_stack.set_oscillator_config(slot, config);
        }
        if modulation.starts_with("-ratio-") {
            configure_stress_filter(params, false, FilterMode::RatioBrickwall);
        }
        return;
    }
    let control = match modulation {
        "" => return,
        "-filter" => {
            configure_stress_filter(params, false, FilterMode::Svf);
            return;
        }
        "-filter-mod" => {
            configure_stress_filter(params, true, FilterMode::Svf);
            return;
        }
        "-ratio" => {
            configure_stress_filter(params, false, FilterMode::RatioBrickwall);
            return;
        }
        "-ratio-mod" => {
            configure_stress_filter(params, true, FilterMode::RatioBrickwall);
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

fn configure_stress_filter(params: &KurvParams, modulated: bool, mode: FilterMode) {
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
            mode,
            cutoff_hz: if mode == FilterMode::RatioBrickwall {
                12.0
            } else {
                1_000.0
            },
            q: 1.0,
            slope_db_oct: 24.0,
            morph: 0.5,
            shape: if mode == FilterMode::RatioBrickwall {
                1.0
            } else {
                0.0
            },
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

fn configure_rig(params: &KurvParams, spec: &str) {
    let mut parts = spec.split('-');
    let split_groups = match parts.next().unwrap_or_default() {
        "1g" => false,
        "2g" => true,
        _ => usage(),
    };
    let unison = parts
        .next()
        .and_then(|value| value.strip_prefix('u'))
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| (1..=64).contains(value))
        .unwrap_or_else(|| usage());
    let mut target_kind = parts.next().unwrap_or_else(|| usage());
    let parent_kind = if target_kind == "parent" {
        let parent = parts.next().unwrap_or_else(|| usage());
        target_kind = parts.next().unwrap_or_else(|| usage());
        Some(parent)
    } else {
        None
    };
    let target_index = match target_kind {
        "osc0" | "group0" | "filter0" => 0,
        "osc1" | "group1" | "filter1" => 1,
        "osc2" => 2,
        "noise" => 3,
        "base" | "mix" => 0,
        _ => usage(),
    };
    let target_detail = parts.collect::<Vec<_>>();

    let first_group = params.generator_stack.snapshot().groups()[0].id();
    let second_group = if split_groups {
        params
            .generator_stack
            .edit(|patch| patch.insert_group(1))
            .unwrap_or_else(|error| fail(&format!("failed to add profiling group: {error:?}")))
    } else {
        first_group
    };
    let insert_oscillator = |group, index| {
        params
            .generator_stack
            .edit(|patch| patch.insert_oscillator(group, index))
            .unwrap_or_else(|error| fail(&format!("failed to add profiling oscillator: {error:?}")))
    };
    insert_oscillator(first_group, 1);
    let first_filter_index = 2;
    params
        .generator_stack
        .edit(|patch| patch.insert_filter(first_group, first_filter_index))
        .unwrap_or_else(|error| fail(&format!("failed to add middle filter: {error:?}")));
    let second_oscillator_group = second_group;
    let second_oscillator_index = if split_groups { 0 } else { 3 };
    insert_oscillator(second_oscillator_group, second_oscillator_index);
    insert_oscillator(second_oscillator_group, second_oscillator_index + 1);
    params
        .generator_stack
        .edit(|patch| patch.insert_filter(second_oscillator_group, second_oscillator_index + 2))
        .unwrap_or_else(|error| fail(&format!("failed to add terminal filter: {error:?}")));

    let snapshot = params.generator_stack.snapshot();
    let oscillators = snapshot
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .filter_map(|module| module.oscillator_slot().map(|slot| (module.id(), slot)))
        .collect::<Vec<_>>();
    let filters = snapshot
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .filter_map(|module| module.filter_slot().map(|slot| (module.id(), slot)))
        .collect::<Vec<_>>();
    let groups = snapshot
        .groups()
        .iter()
        .map(|group| group.id())
        .collect::<Vec<_>>();
    if oscillators.len() != 4 || filters.len() != 2 {
        fail("profiling rig topology was not published");
    }
    for (index, (_, slot)) in oscillators.iter().copied().take(3).enumerate() {
        let tuning_mode = [
            OscillatorTuningMode::Semicent,
            OscillatorTuningMode::Hertz,
            OscillatorTuningMode::Ratio,
        ][index];
        params.generator_stack.set_oscillator_config(
            slot,
            OscillatorConfig {
                shape: index as f32,
                pulse_width: 0.37,
                tuning_mode,
                frequency_offset_hz: 7.0,
                frequency_ratio: 1.5,
                level: 0.25,
                unison_voices: unison,
                unison_jitter: 0.35,
                unison_rate: 0.7,
                phase_random: 0.0,
                ..OscillatorConfig::default()
            },
        );
    }
    params.generator_stack.set_oscillator_config(
        oscillators[3].1,
        OscillatorConfig {
            level: 0.25,
            phase_random: 0.0,
            ..OscillatorConfig::for_engine(OscillatorEngineKind::Noise)
        },
    );

    let requested_filter_mode = target_detail
        .first()
        .and_then(|mode| parse_filter_mode(mode));
    for (index, (_, slot)) in filters.iter().copied().enumerate() {
        let mode = if target_kind == format!("filter{index}") {
            requested_filter_mode.unwrap_or(if index == 0 {
                FilterMode::Svf
            } else {
                FilterMode::Phaser
            })
        } else if index == 0 {
            FilterMode::Svf
        } else {
            FilterMode::Phaser
        };
        params.generator_stack.set_filter_config(
            slot,
            FilterConfig {
                mode,
                cutoff_hz: if mode == FilterMode::RatioBrickwall {
                    12.0
                } else {
                    1_200.0
                },
                q: 0.8,
                slope_db_oct: 24.0,
                morph: 0.5,
                shape: 0.5,
            },
        );
    }

    let target = match target_kind {
        "base" => None,
        "osc0" | "osc1" | "osc2" | "noise" => Some(ModulationRouteTarget::oscillator(
            oscillators[target_index].0,
            oscillators[target_index].1,
            parse_oscillator_control(target_detail.first().copied().unwrap_or("")),
        )),
        "filter0" | "filter1" => target_detail
            .get(1)
            .copied()
            .filter(|control| *control != "none")
            .map(|control| {
                ModulationRouteTarget::filter(
                    filters[target_index].0,
                    filters[target_index].1,
                    parse_filter_control(control),
                )
            }),
        "group0" | "group1" if target_index < groups.len() => Some(ModulationRouteTarget::group(
            groups[target_index],
            parse_group_control(target_detail.first().copied().unwrap_or("")),
        )),
        "mix" => {
            let mut route = 0;
            for target in oscillators
                .iter()
                .map(|(module, slot)| {
                    ModulationRouteTarget::oscillator(*module, *slot, OscillatorControl::Level)
                })
                .chain(filters.iter().map(|(module, slot)| {
                    ModulationRouteTarget::filter(*module, *slot, FilterControl::Cutoff)
                }))
                .chain(
                    groups
                        .iter()
                        .map(|group| ModulationRouteTarget::group(*group, GroupControl::Pan)),
                )
            {
                set_lfo_route(params, route, target);
                route = if route == 0 { 16 } else { route + 1 };
            }
            None
        }
        _ => usage(),
    };
    if let Some(target) = target {
        if let Some((parent, source)) = parent_kind
            .and_then(|parent| parent.rsplit_once('_'))
            .filter(|(_, source)| matches!(*source, "osc0" | "osc1" | "osc2" | "noise"))
        {
            set_generator_child_parent(params, parent, source, target, &oscillators);
        } else {
            set_lfo_route(params, 0, target);
            if let Some(parent) = parent_kind {
                set_parent_route(params, parent, &oscillators);
            }
        }
    }
}

fn set_generator_child_parent(
    params: &KurvParams,
    parent: &str,
    source: &str,
    target: ModulationRouteTarget,
    oscillators: &[(ModuleId, OscillatorSlot)],
) {
    if !matches!(
        parent,
        "none" | "sync" | "free" | "envelope" | "osc0" | "osc1" | "osc2" | "noise"
    ) {
        usage();
    }
    let source = match source {
        "osc0" => 0,
        "osc1" => 1,
        "osc2" => 2,
        "noise" => 3,
        _ => usage(),
    };
    assert!(params.modulation_route_overflow.set(
        16,
        65 + oscillators[source].1.index() as u8,
        0.25,
    ));
    assert!(params.modulation_route_targets.set(16, target));
    if parent == "none" {
        return;
    }
    let parent_source = match parent {
        "sync" | "free" | "envelope" => {
            params.lfo2_active.set_value(true);
            params.lfo2_rate.set_value(1_000.0);
            params.lfo2_mode.set_value(i64::from(parent == "sync") * 2);
            params.source2_envelope.set_value(parent == "envelope");
            2
        }
        "osc0" => 65 + oscillators[0].1.index() as u8,
        "osc1" => 65 + oscillators[1].1.index() as u8,
        "osc2" => 65 + oscillators[2].1.index() as u8,
        "noise" => 65 + oscillators[3].1.index() as u8,
        _ => unreachable!(),
    };
    assert!(params.modulation_route_overflow.set(17, parent_source, 0.1));
    assert!(
        params
            .modulation_route_targets
            .set(17, ModulationRouteTarget::route_depth(16))
    );
}

fn set_parent_route(params: &KurvParams, parent: &str, oscillators: &[(ModuleId, OscillatorSlot)]) {
    let (parent, child) = parent.split_once('_').unwrap_or((parent, "sync"));
    params.lfo1_mode.set_value(i64::from(child == "sync") * 2);
    params.source1_envelope.set_value(child == "envelope");
    if !matches!(child, "sync" | "free" | "envelope") {
        usage();
    }
    if parent == "none" {
        return;
    }
    let route = 16;
    let source = match parent {
        "sync" | "free" | "envelope" => {
            params.lfo2_active.set_value(true);
            params.lfo2_rate.set_value(1_000.0);
            params.lfo2_mode.set_value(i64::from(parent == "sync") * 2);
            params.source2_envelope.set_value(parent == "envelope");
            2
        }
        "osc0" => 65 + oscillators[0].1.index() as u8,
        "osc1" => 65 + oscillators[1].1.index() as u8,
        "osc2" => 65 + oscillators[2].1.index() as u8,
        "noise" => 65 + oscillators[3].1.index() as u8,
        "modwheel" => {
            params.mod_wheel.set_value(0.75);
            params.mod_wheel_route_mask.store(1_u64 << route);
            0
        }
        "xyx" => {
            params.xy_source_x.set_value(0.75);
            params.xy_source_x_route_mask.store(1_u64 << route);
            0
        }
        "xyy" => {
            params.xy_source_y.set_value(0.75);
            params.xy_source_y_route_mask.store(1_u64 << route);
            0
        }
        _ => usage(),
    };
    assert!(params.modulation_route_overflow.set(route, source, 0.1));
    assert!(
        params
            .modulation_route_targets
            .set(route, ModulationRouteTarget::route_depth(0))
    );
}

fn set_lfo_route(params: &KurvParams, route: usize, target: ModulationRouteTarget) {
    params.lfo1_active.set_value(true);
    params.lfo1_rate.set_value(17.0);
    params.lfo1_bipolar.set_value(true);
    if route == 0 {
        params.mod1_source.set_value(1);
        params.mod1_amount.set_value(0.25);
    } else {
        assert!(params.modulation_route_overflow.set(route, 1, 0.25));
    }
    assert!(params.modulation_route_targets.set(route, target));
}

fn parse_oscillator_control(control: &str) -> OscillatorControl {
    match control {
        "shape" => OscillatorControl::Shape,
        "pulse" => OscillatorControl::PulseWidth,
        "transpose" => OscillatorControl::Transpose,
        "cents" => OscillatorControl::Cents,
        "level" => OscillatorControl::Level,
        "pan" => OscillatorControl::Pan,
        "phase" => OscillatorControl::PhasePosition,
        "warp" => OscillatorControl::PhaseWarpAmount,
        "jitter" => OscillatorControl::UnisonJitter,
        "rate" => OscillatorControl::UnisonRate,
        "panx" => OscillatorControl::UnisonStereoPosition,
        "pany" => OscillatorControl::UnisonStereoAlternate,
        "ring" => OscillatorControl::RingModAmount,
        _ => usage(),
    }
}

fn parse_filter_mode(mode: &str) -> Option<FilterMode> {
    match mode {
        "svf" => Some(FilterMode::Svf),
        "phaser" => Some(FilterMode::Phaser),
        "scream" => Some(FilterMode::Scream),
        "ratio" => Some(FilterMode::RatioBrickwall),
        _ => None,
    }
}

fn parse_filter_control(control: &str) -> FilterControl {
    match control {
        "cutoff" => FilterControl::Cutoff,
        "resonance" => FilterControl::Resonance,
        "slope" => FilterControl::Slope,
        "morph" => FilterControl::Morph,
        "shape" => FilterControl::Shape,
        _ => usage(),
    }
}

fn parse_group_control(control: &str) -> GroupControl {
    match control {
        "gain" => GroupControl::Gain,
        "pan" => GroupControl::Pan,
        "attack" => GroupControl::AttackCurve,
        "decay" => GroupControl::DecayCurve,
        "release" => GroupControl::ReleaseCurve,
        _ => usage(),
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
        "usage: process_lab <frames> <callbacks> <repeats> [idle|osc|custom|noise|resynth-[sample|grain|rich][-|2g|svf|svf-mod]|dual|group-one|group-two-same|group-two-split|group-empty|group-noise-same|group-noise|group-resynth-empty-same|group-resynth-empty|aux-input|aux-group|pm|am|rm|pan|xoff-NxN|xfm-NxN|xpm-NxN|xam-NxN|xrm-NxN|xpan-NxN|xselfpm-NxN|xcyclepm-NxN|xmixed-NxN|xdepthpm-NxN|xselfdepthpm-NxN|xlfodepthpm-NxN|xlfodepthfastpm-NxN|gfilter-MODE-CONTROL-NxN[-depth]|svf|svf-max|phaser|phaser-max|scream][-mod|-q-mod|-slope-mod|-morph-mod]|stress4[-phase-mod|-shape-mod|-warp-mod|-filter|-filter-mod|-ratio|-ratio-mod]|rig-[1g|2g]-[u1|u8|u64]-[base|oscN-CONTROL|noise-CONTROL|filterN-MODE-CONTROL|groupN-CONTROL|mix] [voices] [sample-rate] [oversampling]"
    );
    std::process::exit(2);
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
