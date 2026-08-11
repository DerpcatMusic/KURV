use crate::*;

#[inline]
pub(crate) fn oscillator_enabled_mask(enabled: [bool; LEGACY_OSCILLATOR_COUNT]) -> OscillatorMask {
    enabled
        .into_iter()
        .enumerate()
        .fold(0, |mask, (oscillator, enabled)| {
            mask | (OscillatorMask::from(enabled) << oscillator)
        })
}

pub(crate) fn slice_is_static(values: &[f32]) -> bool {
    let bits = values[0].to_bits();
    values[1..].iter().all(|value| value.to_bits() == bits)
}

pub(crate) fn generator_configuration(params: &KurvParams) -> (u8, Antialiasing) {
    let factor = params.oversampling.value_u8().clamp(1, 4);
    (factor, Antialiasing::Spline.for_factor(factor))
}

pub(crate) fn lfo_configuration(params: &KurvParams) -> [LfoConfig; LFO_COUNT] {
    let envelope_sources = [
        params.source1_envelope.value(),
        params.source2_envelope.value(),
        params.source3_envelope.value(),
        params.source4_envelope.value(),
        params.source5_envelope.value(),
        params.source6_envelope.value(),
        params.source7_envelope.value(),
        params.source8_envelope.value(),
    ];
    let mut envelopes = [
        EnvelopeConfig {
            attack: params.source1_attack.value(),
            decay: params.source1_decay.value(),
            sustain: params.source1_sustain.value(),
            release: params.source1_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source2_attack.value(),
            decay: params.source2_decay.value(),
            sustain: params.source2_sustain.value(),
            release: params.source2_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source3_attack.value(),
            decay: params.source3_decay.value(),
            sustain: params.source3_sustain.value(),
            release: params.source3_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source4_attack.value(),
            decay: params.source4_decay.value(),
            sustain: params.source4_sustain.value(),
            release: params.source4_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source5_attack.value(),
            decay: params.source5_decay.value(),
            sustain: params.source5_sustain.value(),
            release: params.source5_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source6_attack.value(),
            decay: params.source6_decay.value(),
            sustain: params.source6_sustain.value(),
            release: params.source6_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source7_attack.value(),
            decay: params.source7_decay.value(),
            sustain: params.source7_sustain.value(),
            release: params.source7_release.value(),
            ..EnvelopeConfig::default()
        },
        EnvelopeConfig {
            attack: params.source8_attack.value(),
            decay: params.source8_decay.value(),
            sustain: params.source8_sustain.value(),
            release: params.source8_release.value(),
            ..EnvelopeConfig::default()
        },
    ];
    for (index, envelope) in envelopes.iter_mut().enumerate() {
        if !envelope_sources[index] {
            continue;
        }
        let persisted = params.modulator_rack.rt_config(index);
        envelope.attack_curve = persisted.attack_curve;
        envelope.decay_curve = persisted.decay_curve;
        envelope.release_curve = persisted.release_curve;
    }
    let legacy = [
        LfoConfig {
            rate_hz: params.lfo1_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo1_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo1_mode.value_u8()),
            phase_offset: params.lfo1_phase.value(),
            sync_division: params.lfo1_sync.value_u8(),
            bipolar: params.lfo1_bipolar.value(),
            envelope: envelope_sources[0],
            envelope_config: envelopes[0],
        },
        LfoConfig {
            rate_hz: params.lfo2_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo2_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo2_mode.value_u8()),
            phase_offset: params.lfo2_phase.value(),
            sync_division: params.lfo2_sync.value_u8(),
            bipolar: params.lfo2_bipolar.value(),
            envelope: envelope_sources[1],
            envelope_config: envelopes[1],
        },
        LfoConfig {
            rate_hz: params.lfo3_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo3_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo3_mode.value_u8()),
            phase_offset: params.lfo3_phase.value(),
            sync_division: params.lfo3_sync.value_u8(),
            bipolar: params.lfo3_bipolar.value(),
            envelope: envelope_sources[2],
            envelope_config: envelopes[2],
        },
        LfoConfig {
            rate_hz: params.lfo4_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo4_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo4_mode.value_u8()),
            phase_offset: params.lfo4_phase.value(),
            sync_division: params.lfo4_sync.value_u8(),
            bipolar: params.lfo4_bipolar.value(),
            envelope: envelope_sources[3],
            envelope_config: envelopes[3],
        },
        LfoConfig {
            rate_hz: params.lfo5_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo5_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo5_mode.value_u8()),
            phase_offset: params.lfo5_phase.value(),
            sync_division: params.lfo5_sync.value_u8(),
            bipolar: params.lfo5_bipolar.value(),
            envelope: envelope_sources[4],
            envelope_config: envelopes[4],
        },
        LfoConfig {
            rate_hz: params.lfo6_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo6_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo6_mode.value_u8()),
            phase_offset: params.lfo6_phase.value(),
            sync_division: params.lfo6_sync.value_u8(),
            bipolar: params.lfo6_bipolar.value(),
            envelope: envelope_sources[5],
            envelope_config: envelopes[5],
        },
        LfoConfig {
            rate_hz: params.lfo7_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo7_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo7_mode.value_u8()),
            phase_offset: params.lfo7_phase.value(),
            sync_division: params.lfo7_sync.value_u8(),
            bipolar: params.lfo7_bipolar.value(),
            envelope: envelope_sources[6],
            envelope_config: envelopes[6],
        },
        LfoConfig {
            rate_hz: params.lfo8_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo8_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo8_mode.value_u8()),
            phase_offset: params.lfo8_phase.value(),
            sync_division: params.lfo8_sync.value_u8(),
            bipolar: params.lfo8_bipolar.value(),
            envelope: envelope_sources[7],
            envelope_config: envelopes[7],
        },
    ];
    let mut configs = [LfoConfig::default(); LFO_COUNT];
    configs[..LEGACY_MODULATION_SOURCES].copy_from_slice(&legacy);
    let dynamic_mask = params.modulator_rack.active_mask();
    for (index, target) in configs
        .iter_mut()
        .enumerate()
        .skip(LEGACY_MODULATION_SOURCES)
    {
        if dynamic_mask & (1_u64 << index) == 0 {
            continue;
        }
        let source = params.modulator_rack.rt_config(index);
        *target = LfoConfig {
            rate_hz: source.rate_hz,
            rate_mode: LfoRateMode::from_index(source.rate_mode),
            mode: LfoMode::from_index(source.mode),
            phase_offset: source.phase_offset,
            sync_division: source.sync_division,
            bipolar: source.bipolar,
            envelope: source.kind == SourceKind::Envelope,
            envelope_config: EnvelopeConfig {
                attack: source.attack,
                attack_curve: source.attack_curve,
                decay: source.decay,
                decay_curve: source.decay_curve,
                sustain: source.sustain,
                release: source.release,
                release_curve: source.release_curve,
            },
        };
    }
    configs
}

pub(crate) fn configured_lfo_mask(params: &KurvParams) -> u64 {
    let legacy = [
        params.lfo1_active.value(),
        params.lfo2_active.value(),
        params.lfo3_active.value(),
        params.lfo4_active.value(),
        params.lfo5_active.value(),
        params.lfo6_active.value(),
        params.lfo7_active.value(),
        params.lfo8_active.value(),
    ]
    .into_iter()
    .enumerate()
    .fold(0, |mask, (index, active)| {
        mask | if active { 1_u64 << index } else { 0 }
    });
    legacy | (params.modulator_rack.active_mask() & !((1_u64 << LEGACY_MODULATION_SOURCES) - 1))
}

pub(crate) fn modulation_routes(params: &KurvParams) -> [RouteConfig; ROUTE_COUNT] {
    [
        RouteConfig {
            source: params.mod1_source.value_u8(),
            target: resolved_modulation_target(
                params.mod1_target.value_u8(),
                params.mod1_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod2_source.value_u8(),
            target: resolved_modulation_target(
                params.mod2_target.value_u8(),
                params.mod2_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod3_source.value_u8(),
            target: resolved_modulation_target(
                params.mod3_target.value_u8(),
                params.mod3_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod4_source.value_u8(),
            target: resolved_modulation_target(
                params.mod4_target.value_u8(),
                params.mod4_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod5_source.value_u8(),
            target: resolved_modulation_target(
                params.mod5_target.value_u8(),
                params.mod5_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod6_source.value_u8(),
            target: resolved_modulation_target(
                params.mod6_target.value_u8(),
                params.mod6_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod7_source.value_u8(),
            target: resolved_modulation_target(
                params.mod7_target.value_u8(),
                params.mod7_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod8_source.value_u8(),
            target: resolved_modulation_target(
                params.mod8_target.value_u8(),
                params.mod8_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod9_source.value_u8(),
            target: resolved_modulation_target(
                params.mod9_target.value_u8(),
                params.mod9_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod10_source.value_u8(),
            target: resolved_modulation_target(
                params.mod10_target.value_u8(),
                params.mod10_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod11_source.value_u8(),
            target: resolved_modulation_target(
                params.mod11_target.value_u8(),
                params.mod11_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod12_source.value_u8(),
            target: resolved_modulation_target(
                params.mod12_target.value_u8(),
                params.mod12_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod13_source.value_u8(),
            target: resolved_modulation_target(
                params.mod13_target.value_u8(),
                params.mod13_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod14_source.value_u8(),
            target: resolved_modulation_target(
                params.mod14_target.value_u8(),
                params.mod14_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod15_source.value_u8(),
            target: resolved_modulation_target(
                params.mod15_target.value_u8(),
                params.mod15_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod16_source.value_u8(),
            target: resolved_modulation_target(
                params.mod16_target.value_u8(),
                params.mod16_target_ext.value_u8(),
            ),
        },
    ]
}

pub(crate) const fn resolved_modulation_target(legacy: u8, extended: u8) -> u8 {
    if extended == 0 {
        legacy
    } else {
        modulation_target::LEGACY_TARGET_COUNT + extended
    }
}

fn push_legacy_route(
    active: &mut ActiveRoutes,
    source: ResolvedRouteSource,
    target: u8,
    host_amount_index: Option<u8>,
    overflow_amount_index: Option<u8>,
    amount: f32,
    oscillator_enabled: &[bool; LEGACY_OSCILLATOR_COUNT],
) {
    if target == 0
        || modulation_target::target_oscillator(target)
            .is_some_and(|oscillator| !oscillator_enabled[oscillator])
    {
        return;
    }
    let descriptor = modulation_target::descriptor(target);
    active.entries[active.len] = ActiveRoute {
        host_amount_index,
        overflow_amount_index,
        amount,
        source,
        descriptor: descriptor.copied(),
    };
    active.len += 1;
    active.include_source(source);
    let Some(descriptor) = descriptor else {
        return;
    };
    match descriptor.kind {
        modulation_target::TargetKind::Oscillator {
            oscillator,
            control,
        } => {
            active.oscillator_mask |= 1 << oscillator;
            if matches!(control, modulation_target::OscTarget::Shape) {
                active.oscillator_shape_mask |= 1 << oscillator;
            }
        }
        modulation_target::TargetKind::Unison {
            oscillator,
            control,
        } => {
            if matches!(
                control,
                modulation_target::UnisonTarget::DetuneAmount
                    | modulation_target::UnisonTarget::DetuneRange
                    | modulation_target::UnisonTarget::HarmonicAlign
                    | modulation_target::UnisonTarget::Stereo
                    | modulation_target::UnisonTarget::Curve
                    | modulation_target::UnisonTarget::StereoX
                    | modulation_target::UnisonTarget::StereoY
                    | modulation_target::UnisonTarget::Weight
                    | modulation_target::UnisonTarget::PanCenter
                    | modulation_target::UnisonTarget::PanLeft
                    | modulation_target::UnisonTarget::PanRight
                    | modulation_target::UnisonTarget::PanCenterX
            ) {
                active.unison_frame_mask |= 1 << oscillator;
            } else {
                active.unison_layout_mask |= 1 << oscillator;
            }
        }
        modulation_target::TargetKind::Global(control) => {
            active.global_mask |= match control {
                modulation_target::GlobalTarget::Output => GLOBAL_OUTPUT_MASK,
                modulation_target::GlobalTarget::Attack
                | modulation_target::GlobalTarget::Decay
                | modulation_target::GlobalTarget::Sustain
                | modulation_target::GlobalTarget::Release
                | modulation_target::GlobalTarget::AttackCurve
                | modulation_target::GlobalTarget::DecayCurve
                | modulation_target::GlobalTarget::ReleaseCurve
                | modulation_target::GlobalTarget::AttackCurveTime
                | modulation_target::GlobalTarget::DecayCurveTime
                | modulation_target::GlobalTarget::ReleaseCurveTime => GLOBAL_ENVELOPE_MASK,
                modulation_target::GlobalTarget::Velocity => GLOBAL_VELOCITY_MASK,
                modulation_target::GlobalTarget::Pressure => GLOBAL_PRESSURE_MASK,
                modulation_target::GlobalTarget::Timbre => GLOBAL_TIMBRE_MASK,
                modulation_target::GlobalTarget::Glide => GLOBAL_GLIDE_MASK,
            };
        }
    }
}

pub(crate) fn active_modulation_routes(
    routes: &[RouteConfig; ROUTE_COUNT],
    modular_targets: &ModulationRouteTargetSnapshot,
    overflow_routes: &ExtraModulationRouteSnapshot,
    mod_wheel_route_mask: u64,
    module_ids: &[u64; generators::MAX_OSCILLATORS],
    filter_module_ids: &[u64; generators::MAX_FILTERS],
    group_ids: &[u64; generators::MAX_OUTPUT_PAIRS],
    group_count: usize,
    oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
) -> ActiveRoutes {
    let mut active = ActiveRoutes::default();
    for (index, route) in routes.iter().copied().enumerate() {
        let source = ResolvedRouteSource::decode(route.source, mod_wheel_route_mask, index);
        if let Some(target) = modular_targets[index] {
            if let ModulationRouteTarget::Legacy { target } = target {
                if let Some(source) = source {
                    push_legacy_route(
                        &mut active,
                        source,
                        target,
                        Some(index as u8),
                        None,
                        0.0,
                        &oscillator_enabled,
                    );
                }
                continue;
            }
            let target = resolve_modular_target(
                target,
                module_ids,
                filter_module_ids,
                group_ids,
                group_count,
            );
            if let (Some(source), Some(target)) = (source, target) {
                active.modular_entries[active.modular_len] = ActiveModularRoute {
                    host_amount_index: Some(index as u8),
                    overflow_amount_index: None,
                    amount: 0.0,
                    source,
                    target: Some(target),
                };
                active.modular_len += 1;
                active.include_source(source);
                if let ResolvedModularTarget::Group { index, .. } = target {
                    active.modular_group_mask |= 1 << index;
                }
            }
            continue;
        }
        if let Some(source) = source {
            push_legacy_route(
                &mut active,
                source,
                route.target,
                Some(index as u8),
                None,
                0.0,
                &oscillator_enabled,
            );
        }
    }
    for (offset, route) in overflow_routes.iter().copied().enumerate() {
        let route_index = HOST_MODULATION_ROUTE_COUNT + offset;
        let Some(source) =
            ResolvedRouteSource::decode(route.source, mod_wheel_route_mask, route_index)
        else {
            continue;
        };
        let Some(target) = modular_targets[route_index] else {
            continue;
        };
        if let ModulationRouteTarget::Legacy { target } = target {
            push_legacy_route(
                &mut active,
                source,
                target,
                None,
                Some(offset as u8),
                route.amount,
                &oscillator_enabled,
            );
            continue;
        }
        let Some(target) = resolve_modular_target(
            target,
            module_ids,
            filter_module_ids,
            group_ids,
            group_count,
        ) else {
            continue;
        };
        active.modular_entries[active.modular_len] = ActiveModularRoute {
            host_amount_index: None,
            overflow_amount_index: Some(offset as u8),
            amount: route.amount,
            source,
            target: Some(target),
        };
        active.modular_len += 1;
        active.include_source(source);
        if let ResolvedModularTarget::Group { index, .. } = target {
            active.modular_group_mask |= 1 << index;
        }
    }
    active
}

pub(crate) fn resolve_modular_target(
    target: ModulationRouteTarget,
    module_ids: &[u64; generators::MAX_OSCILLATORS],
    filter_module_ids: &[u64; generators::MAX_FILTERS],
    group_ids: &[u64; generators::MAX_OUTPUT_PAIRS],
    group_count: usize,
) -> Option<ResolvedModularTarget> {
    if !target.supports_internal_modulation() {
        return None;
    }
    match target {
        ModulationRouteTarget::Legacy { .. } => None,
        ModulationRouteTarget::Oscillator {
            module_id,
            slot,
            control,
        } => (module_ids[slot.index()] == module_id).then_some(ResolvedModularTarget::Oscillator {
            slot: slot.index() as u8,
            control,
        }),
        ModulationRouteTarget::Group { group_id, control } => group_ids
            [..group_count.min(group_ids.len())]
            .iter()
            .position(|id| *id == group_id)
            .map(|index| ResolvedModularTarget::Group {
                index: index as u8,
                control,
            }),
        ModulationRouteTarget::Filter {
            module_id,
            slot,
            control,
        } => (filter_module_ids[slot.index()] == module_id).then_some(
            ResolvedModularTarget::Filter {
                slot: slot.index() as u8,
                control,
            },
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "each oscillator exposes the pan-shaper coordinates as independent host parameters"
)]
pub(crate) fn oscillator_pan_shape_settings(
    segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
    initialized: bool,
    center: f32,
    left: f32,
    right: f32,
    left_curve: f32,
    right_curve: f32,
    left_time: f32,
    right_time: f32,
    center_x: f32,
) -> PanShapeSettings {
    let (left_segments, right_segments) = segments;
    let center = if initialized {
        left_segments.seg_p0[0]
    } else {
        center
    };
    let left = if initialized {
        left_segments.seg_p3[usize::from(left_segments.count.saturating_sub(1))]
    } else {
        left
    };
    let right = if initialized {
        right_segments.seg_p3[usize::from(right_segments.count.saturating_sub(1))]
    } else {
        right
    };
    PanShapeSettings::new(center, 1.0, 0.0)
        .with_center_x(center_x)
        .with_sides(left, right, left_curve, right_curve)
        .with_curve_times(left_time, right_time)
        .with_segments((left_segments, right_segments))
}

pub(crate) fn unison_configurations(
    params: &KurvParams,
    state: &KurvDspState,
) -> [UnisonSettings; LEGACY_OSCILLATOR_COUNT] {
    [
        UnisonSettings::new(
            params.unison_voices.value_u8(),
            params.unison_detune.value() * 100.0,
            params.unison_stereo.value(),
            params.phase_random.value(),
            params.unison_curve.value(),
        )
        .with_detune_amount(params.unison_detune_amount.value())
        .with_harmonic_align(params.unison_harmonic_align.value())
        .with_alignment_mode(params.unison_alignment_mode.value_u8())
        .with_pan_shape(
            PanShapeSettings::new(
                params.pan_shape_center.value(),
                params.pan_shape_edge.value(),
                params.pan_shape_curve.value(),
            )
            .with_center_x(params.pan_shape_center_x.value())
            .with_sides(
                params.pan_shape_left.value(),
                params.pan_shape_right.value(),
                params.pan_shape_left_curve.value(),
                params.pan_shape_right_curve.value(),
            )
            .with_curve_times(
                params.pan_shape_left_curve_time.value(),
                params.pan_shape_right_curve_time.value(),
            )
            .with_segments(state.pan_shape_segments[0]),
        )
        .with_stereo_square(params.stereo_alternate.value(), params.stereo_x.value())
        .with_swarm(
            params.unison_swarm.value(),
            params.unison_swarm_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.unison_swarm_mode.value_u8()))
        .with_level_curve(params.unison_weight.value()),
        UnisonSettings::new(
            params.osc2_unison_voices.value_u8(),
            params.osc2_unison_detune.value() * 100.0,
            params.osc2_unison_stereo.value(),
            params.osc2_phase_random.value(),
            params.osc2_unison_curve.value(),
        )
        .with_detune_amount(params.osc2_unison_detune_amount.value())
        .with_harmonic_align(params.osc2_unison_harmonic_align.value())
        .with_alignment_mode(params.osc2_unison_alignment_mode.value_u8())
        .with_pan_shape(oscillator_pan_shape_settings(
            state.pan_shape_segments[1],
            params.osc2_pan_shape_curve_state.is_initialized(),
            params.osc2_pan_shape_center.value(),
            params.osc2_pan_shape_left.value(),
            params.osc2_pan_shape_right.value(),
            params.osc2_pan_shape_left_curve.value(),
            params.osc2_pan_shape_right_curve.value(),
            params.osc2_pan_shape_left_curve_time.value(),
            params.osc2_pan_shape_right_curve_time.value(),
            params.osc2_pan_shape_center_x.value(),
        ))
        .with_stereo_square(
            params.osc2_stereo_alternate.value(),
            params.osc2_stereo_x.value(),
        )
        .with_swarm(
            params.osc2_unison_jitter.value(),
            params.osc2_unison_jitter_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.osc2_jitter_mode.value_u8()))
        .with_level_curve(params.osc2_unison_weight.value()),
        UnisonSettings::new(
            params.osc3_unison_voices.value_u8(),
            params.osc3_unison_detune.value() * 100.0,
            params.osc3_unison_stereo.value(),
            params.osc3_phase_random.value(),
            params.osc3_unison_curve.value(),
        )
        .with_detune_amount(params.osc3_unison_detune_amount.value())
        .with_harmonic_align(params.osc3_unison_harmonic_align.value())
        .with_alignment_mode(params.osc3_unison_alignment_mode.value_u8())
        .with_pan_shape(oscillator_pan_shape_settings(
            state.pan_shape_segments[2],
            params.osc3_pan_shape_curve_state.is_initialized(),
            params.osc3_pan_shape_center.value(),
            params.osc3_pan_shape_left.value(),
            params.osc3_pan_shape_right.value(),
            params.osc3_pan_shape_left_curve.value(),
            params.osc3_pan_shape_right_curve.value(),
            params.osc3_pan_shape_left_curve_time.value(),
            params.osc3_pan_shape_right_curve_time.value(),
            params.osc3_pan_shape_center_x.value(),
        ))
        .with_stereo_square(
            params.osc3_stereo_alternate.value(),
            params.osc3_stereo_x.value(),
        )
        .with_swarm(
            params.osc3_unison_jitter.value(),
            params.osc3_unison_jitter_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.osc3_jitter_mode.value_u8()))
        .with_level_curve(params.osc3_unison_weight.value()),
    ]
}
