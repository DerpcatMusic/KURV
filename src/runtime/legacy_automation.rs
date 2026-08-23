use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum LegacyOscillatorField {
    Enabled,
    Shape,
    CustomShape,
    PulseWidth,
    Transpose,
    Cents,
    Level,
    Pan,
    UnisonVoices,
    UnisonRange,
    UnisonAmount,
    UnisonCurve,
    UnisonJitter,
    UnisonJitterMode,
    UnisonRate,
    UnisonWidth,
    UnisonWeight,
    PhasePosition,
    PhaseRandom,
    PhaseWarpMode,
    PhaseWarpAmount,
    UnisonAlignment,
    UnisonAlignmentMode,
    UnisonPanCurve,
    UnisonPanCenter,
    UnisonStereoPosition,
    UnisonStereoAlternate,
}

impl LegacyOscillatorField {
    const ALL: [Self; 27] = [
        Self::Enabled,
        Self::Shape,
        Self::CustomShape,
        Self::PulseWidth,
        Self::Transpose,
        Self::Cents,
        Self::Level,
        Self::Pan,
        Self::UnisonVoices,
        Self::UnisonRange,
        Self::UnisonAmount,
        Self::UnisonCurve,
        Self::UnisonJitter,
        Self::UnisonJitterMode,
        Self::UnisonRate,
        Self::UnisonWidth,
        Self::UnisonWeight,
        Self::PhasePosition,
        Self::PhaseRandom,
        Self::PhaseWarpMode,
        Self::PhaseWarpAmount,
        Self::UnisonAlignment,
        Self::UnisonAlignmentMode,
        Self::UnisonPanCurve,
        Self::UnisonPanCenter,
        Self::UnisonStereoPosition,
        Self::UnisonStereoAlternate,
    ];

    const fn mask(self) -> u32 {
        1_u32 << self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum LegacyGroupField {
    Attack,
    AttackCurve,
    AttackCurveTime,
    Decay,
    DecayCurve,
    DecayCurveTime,
    Sustain,
    Release,
    ReleaseCurve,
    ReleaseCurveTime,
}

impl LegacyGroupField {
    const ALL: [Self; 10] = [
        Self::Attack,
        Self::AttackCurve,
        Self::AttackCurveTime,
        Self::Decay,
        Self::DecayCurve,
        Self::DecayCurveTime,
        Self::Sustain,
        Self::Release,
        Self::ReleaseCurve,
        Self::ReleaseCurveTime,
    ];

    const fn mask(self) -> u16 {
        1_u16 << self as u8
    }
}

const fn legacy_oscillator_automation_target(id: u32) -> Option<(usize, LegacyOscillatorField)> {
    use LegacyOscillatorField as F;
    match id {
        53 => Some((0, F::Enabled)),
        58 => Some((1, F::Enabled)),
        65 => Some((2, F::Enabled)),
        1 => Some((0, F::Shape)),
        59 => Some((1, F::Shape)),
        66 => Some((2, F::Shape)),
        119 => Some((0, F::CustomShape)),
        120 => Some((1, F::CustomShape)),
        121 => Some((2, F::CustomShape)),
        2 => Some((0, F::PulseWidth)),
        60 => Some((1, F::PulseWidth)),
        67 => Some((2, F::PulseWidth)),
        54 => Some((0, F::Transpose)),
        61 => Some((1, F::Transpose)),
        68 => Some((2, F::Transpose)),
        55 => Some((0, F::Cents)),
        62 => Some((1, F::Cents)),
        69 => Some((2, F::Cents)),
        56 => Some((0, F::Level)),
        63 => Some((1, F::Level)),
        70 => Some((2, F::Level)),
        57 => Some((0, F::Pan)),
        64 => Some((1, F::Pan)),
        71 => Some((2, F::Pan)),
        11 => Some((0, F::UnisonVoices)),
        72 => Some((1, F::UnisonVoices)),
        91 => Some((2, F::UnisonVoices)),
        12 => Some((0, F::UnisonRange)),
        73 => Some((1, F::UnisonRange)),
        92 => Some((2, F::UnisonRange)),
        34 => Some((0, F::UnisonAmount)),
        74 => Some((1, F::UnisonAmount)),
        93 => Some((2, F::UnisonAmount)),
        15 => Some((0, F::UnisonCurve)),
        77 => Some((1, F::UnisonCurve)),
        96 => Some((2, F::UnisonCurve)),
        20 => Some((0, F::UnisonJitter)),
        78 => Some((1, F::UnisonJitter)),
        97 => Some((2, F::UnisonJitter)),
        118 => Some((0, F::UnisonJitterMode)),
        110 => Some((1, F::UnisonJitterMode)),
        111 => Some((2, F::UnisonJitterMode)),
        21 => Some((0, F::UnisonRate)),
        79 => Some((1, F::UnisonRate)),
        98 => Some((2, F::UnisonRate)),
        13 => Some((0, F::UnisonWidth)),
        75 => Some((1, F::UnisonWidth)),
        94 => Some((2, F::UnisonWidth)),
        32 => Some((0, F::UnisonWeight)),
        82 => Some((1, F::UnisonWeight)),
        101 => Some((2, F::UnisonWeight)),
        250 => Some((0, F::PhasePosition)),
        251 => Some((1, F::PhasePosition)),
        252 => Some((2, F::PhasePosition)),
        14 => Some((0, F::PhaseRandom)),
        76 => Some((1, F::PhaseRandom)),
        95 => Some((2, F::PhaseRandom)),
        112 => Some((0, F::PhaseWarpMode)),
        113 => Some((1, F::PhaseWarpMode)),
        114 => Some((2, F::PhaseWarpMode)),
        115 => Some((0, F::PhaseWarpAmount)),
        116 => Some((1, F::PhaseWarpAmount)),
        117 => Some((2, F::PhaseWarpAmount)),
        244 => Some((0, F::UnisonAlignment)),
        245 => Some((1, F::UnisonAlignment)),
        246 => Some((2, F::UnisonAlignment)),
        247 => Some((0, F::UnisonAlignmentMode)),
        248 => Some((1, F::UnisonAlignmentMode)),
        249 => Some((2, F::UnisonAlignmentMode)),
        37 => Some((0, F::UnisonPanCurve)),
        46 => Some((0, F::UnisonPanCenter)),
        90 => Some((1, F::UnisonPanCenter)),
        109 => Some((2, F::UnisonPanCenter)),
        31 => Some((0, F::UnisonStereoPosition)),
        81 => Some((1, F::UnisonStereoPosition)),
        100 => Some((2, F::UnisonStereoPosition)),
        30 => Some((0, F::UnisonStereoAlternate)),
        80 => Some((1, F::UnisonStereoAlternate)),
        99 => Some((2, F::UnisonStereoAlternate)),
        _ => None,
    }
}

const fn legacy_group_automation_target(id: u32) -> Option<LegacyGroupField> {
    use LegacyGroupField as F;
    match id {
        3 => Some(F::Attack),
        24 => Some(F::AttackCurve),
        27 => Some(F::AttackCurveTime),
        4 => Some(F::Decay),
        25 => Some(F::DecayCurve),
        28 => Some(F::DecayCurveTime),
        5 => Some(F::Sustain),
        6 => Some(F::Release),
        26 => Some(F::ReleaseCurve),
        29 => Some(F::ReleaseCurveTime),
        _ => None,
    }
}

fn copy_legacy_group_field(
    target: &mut generators::GroupOutput,
    source: generators::GroupOutput,
    field: LegacyGroupField,
) {
    use LegacyGroupField as F;
    match field {
        F::Attack => target.attack = source.attack,
        F::AttackCurve => target.attack_curve = source.attack_curve,
        F::AttackCurveTime => target.attack_curve_time = source.attack_curve_time,
        F::Decay => target.decay = source.decay,
        F::DecayCurve => target.decay_curve = source.decay_curve,
        F::DecayCurveTime => target.decay_curve_time = source.decay_curve_time,
        F::Sustain => target.sustain = source.sustain,
        F::Release => target.release = source.release,
        F::ReleaseCurve => target.release_curve = source.release_curve,
        F::ReleaseCurveTime => target.release_curve_time = source.release_curve_time,
    }
}

fn legacy_group_field_matches(
    left: generators::GroupOutput,
    right: generators::GroupOutput,
    field: LegacyGroupField,
) -> bool {
    let mut candidate = left;
    copy_legacy_group_field(&mut candidate, right, field);
    candidate == left
}

fn copy_legacy_oscillator_field(
    target: &mut generators::OscillatorConfig,
    source: generators::OscillatorConfig,
    field: LegacyOscillatorField,
) {
    use LegacyOscillatorField as F;
    match field {
        F::Enabled => target.enabled = source.enabled,
        F::Shape => target.shape = source.shape,
        F::CustomShape => target.custom_shape = source.custom_shape,
        F::PulseWidth => target.pulse_width = source.pulse_width,
        F::Transpose => target.transpose = source.transpose,
        F::Cents => target.cents = source.cents,
        F::Level => target.level = source.level,
        F::Pan => target.pan = source.pan,
        F::UnisonVoices => target.unison_voices = source.unison_voices,
        F::UnisonRange => target.unison_range = source.unison_range,
        F::UnisonAmount => target.unison_amount = source.unison_amount,
        F::UnisonCurve => target.unison_curve = source.unison_curve,
        F::UnisonJitter => target.unison_jitter = source.unison_jitter,
        F::UnisonJitterMode => target.unison_jitter_mode = source.unison_jitter_mode,
        F::UnisonRate => target.unison_rate = source.unison_rate,
        F::UnisonWidth => target.unison_width = source.unison_width,
        F::UnisonWeight => target.unison_weight = source.unison_weight,
        F::PhasePosition => target.phase_position = source.phase_position,
        F::PhaseRandom => target.phase_random = source.phase_random,
        F::PhaseWarpMode => target.phase_warp_mode = source.phase_warp_mode,
        F::PhaseWarpAmount => target.phase_warp_amount = source.phase_warp_amount,
        F::UnisonAlignment => target.unison_alignment = source.unison_alignment,
        F::UnisonAlignmentMode => target.unison_alignment_mode = source.unison_alignment_mode,
        F::UnisonPanCurve => target.unison_pan_curve = source.unison_pan_curve,
        F::UnisonPanCenter => target.unison_pan_center_x = source.unison_pan_center_x,
        F::UnisonStereoPosition => target.unison_stereo_x = source.unison_stereo_x,
        F::UnisonStereoAlternate => {
            target.unison_stereo_alternate = source.unison_stereo_alternate;
        }
    }
}

fn legacy_oscillator_field_matches(
    left: generators::OscillatorConfig,
    right: generators::OscillatorConfig,
    field: LegacyOscillatorField,
) -> bool {
    let mut candidate = left;
    copy_legacy_oscillator_field(&mut candidate, right, field);
    candidate == left
}

/// Track historical host automation as an RT-only overlay on migrated legacy
/// modules. The overlay survives blocks without events, but a structural edit
/// to that same field releases it until the host sends another old-ID event.
pub(crate) fn refresh_legacy_materialized_automation(
    state: &mut KurvDspState,
    params: &KurvParams,
    events: &EventList,
) -> bool {
    let stack = &params.generator_stack;
    let bridge_enabled = stack.legacy_host_automation_bridge_enabled();
    if !bridge_enabled {
        let changed = state.legacy_automation_initialized
            || state
                .legacy_oscillator_automation_masks
                .iter()
                .any(|mask| *mask != 0)
            || state.legacy_group_automation_mask != 0;
        state.legacy_automation_initialized = false;
        state.legacy_oscillator_automation_masks.fill(0);
        state.legacy_group_automation_mask = 0;
        state.legacy_oscillator_automation_released.fill(0);
        state.legacy_group_automation_released = 0;
        state.legacy_pan_automation_masks.fill(0);
        state.legacy_pan_automation_released.fill(0);
        if changed {
            stack.set_legacy_automation_masks([0; 3], 0, [0; 3], 0, [0; 3], [0; 3]);
        }
        return changed;
    }

    let legacy = params.legacy_oscillator_configs();
    let legacy_group = params.legacy_group_output();
    let epoch = stack.legacy_automation_epoch();
    let group_index = state.generator_group_ids[..state.generator_group_count]
        .iter()
        .position(|id| *id == 1);
    let mut changed = false;
    let mut masks_changed = false;
    if !state.legacy_automation_initialized || state.legacy_automation_epoch != epoch {
        let (
            oscillator_masks,
            group_mask,
            oscillator_released,
            group_released,
            pan_masks,
            pan_released,
        ) = stack.legacy_automation_masks();
        state.legacy_oscillator_automation_masks = oscillator_masks;
        state.legacy_group_automation_mask = if group_index.is_some() { group_mask } else { 0 };
        state.legacy_oscillator_automation_released = oscillator_released;
        state.legacy_group_automation_released = if group_index.is_some() {
            group_released
        } else {
            0
        };
        state.legacy_pan_automation_masks = pan_masks;
        state.legacy_pan_automation_released = pan_released;
        state.legacy_oscillator_automation_values = legacy;
        state.legacy_oscillator_automation_bases[..LEGACY_OSCILLATOR_COUNT]
            .copy_from_slice(&state.generator_oscillators[..LEGACY_OSCILLATOR_COUNT]);
        state.legacy_oscillator_automation_observed = legacy;
        state.legacy_group_automation_value = legacy_group;
        state.legacy_group_automation_base = group_index
            .map_or_else(generators::GroupOutput::default, |index| {
                state.generator_group_outputs[index]
            });
        state.legacy_group_automation_observed = legacy_group;
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            for field in LegacyOscillatorField::ALL {
                let bit = field.mask();
                if state.legacy_oscillator_automation_masks[oscillator] & bit == 0
                    && state.legacy_oscillator_automation_released[oscillator] & bit == 0
                    && !legacy_oscillator_field_matches(
                        state.generator_oscillators[oscillator],
                        legacy[oscillator],
                        field,
                    )
                {
                    state.legacy_oscillator_automation_masks[oscillator] |= bit;
                    masks_changed = true;
                }
            }
        }
        if let Some(group_index) = group_index {
            for field in LegacyGroupField::ALL {
                let bit = field.mask();
                if state.legacy_group_automation_mask & bit == 0
                    && state.legacy_group_automation_released & bit == 0
                    && !legacy_group_field_matches(
                        state.generator_group_outputs[group_index],
                        legacy_group,
                        field,
                    )
                {
                    state.legacy_group_automation_mask |= bit;
                    masks_changed = true;
                }
            }
        }
        state.legacy_automation_epoch = epoch;
        state.legacy_automation_initialized = true;
        changed = true;
        masks_changed |= group_index.is_none() && group_mask != 0;
    }

    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        if state.generator_module_ids[oscillator] != oscillator as u64 + 1 {
            masks_changed |= state.legacy_oscillator_automation_masks[oscillator] != 0;
            state.legacy_oscillator_automation_masks[oscillator] = 0;
            continue;
        }
        for field in LegacyOscillatorField::ALL {
            let bit = field.mask();
            if state.legacy_oscillator_automation_masks[oscillator] & bit != 0
                && !legacy_oscillator_field_matches(
                    state.generator_oscillators[oscillator],
                    state.legacy_oscillator_automation_bases[oscillator],
                    field,
                )
            {
                state.legacy_oscillator_automation_masks[oscillator] &= !bit;
                state.legacy_oscillator_automation_released[oscillator] |= bit;
                changed = true;
                masks_changed = true;
            }
            if !legacy_oscillator_field_matches(
                state.legacy_oscillator_automation_observed[oscillator],
                legacy[oscillator],
                field,
            ) {
                copy_legacy_oscillator_field(
                    &mut state.legacy_oscillator_automation_observed[oscillator],
                    legacy[oscillator],
                    field,
                );
                copy_legacy_oscillator_field(
                    &mut state.legacy_oscillator_automation_values[oscillator],
                    legacy[oscillator],
                    field,
                );
                copy_legacy_oscillator_field(
                    &mut state.legacy_oscillator_automation_bases[oscillator],
                    state.generator_oscillators[oscillator],
                    field,
                );
                state.legacy_oscillator_automation_masks[oscillator] |= bit;
                state.legacy_oscillator_automation_released[oscillator] &= !bit;
                changed = true;
                masks_changed = true;
            }
        }
    }

    if let Some(group_index) = group_index {
        for field in LegacyGroupField::ALL {
            let bit = field.mask();
            if state.legacy_group_automation_mask & bit != 0
                && !legacy_group_field_matches(
                    state.generator_group_outputs[group_index],
                    state.legacy_group_automation_base,
                    field,
                )
            {
                state.legacy_group_automation_mask &= !bit;
                state.legacy_group_automation_released |= bit;
                changed = true;
                masks_changed = true;
            }
            if !legacy_group_field_matches(
                state.legacy_group_automation_observed,
                legacy_group,
                field,
            ) {
                copy_legacy_group_field(
                    &mut state.legacy_group_automation_observed,
                    legacy_group,
                    field,
                );
                copy_legacy_group_field(
                    &mut state.legacy_group_automation_value,
                    legacy_group,
                    field,
                );
                copy_legacy_group_field(
                    &mut state.legacy_group_automation_base,
                    state.generator_group_outputs[group_index],
                    field,
                );
                state.legacy_group_automation_mask |= bit;
                state.legacy_group_automation_released &= !bit;
                changed = true;
                masks_changed = true;
            }
        }
    } else if state.legacy_group_automation_mask != 0 {
        state.legacy_group_automation_mask = 0;
        changed = true;
        masks_changed = true;
    }

    // Snapshot differences cannot reveal a same-value host event. Inspect the
    // explicit process stream as well so a released historical lane is always
    // re-armed by the next old-ID event, including one deferred from a stopped
    // CLAP `params_flush`.
    for event in events.iter() {
        let EventBody::ParamChange { id, .. } = event.body else {
            continue;
        };
        if let Some((oscillator, field)) = legacy_oscillator_automation_target(id)
            && state.generator_module_ids[oscillator] == oscillator as u64 + 1
        {
            let bit = field.mask();
            copy_legacy_oscillator_field(
                &mut state.legacy_oscillator_automation_observed[oscillator],
                legacy[oscillator],
                field,
            );
            copy_legacy_oscillator_field(
                &mut state.legacy_oscillator_automation_values[oscillator],
                legacy[oscillator],
                field,
            );
            copy_legacy_oscillator_field(
                &mut state.legacy_oscillator_automation_bases[oscillator],
                state.generator_oscillators[oscillator],
                field,
            );
            state.legacy_oscillator_automation_masks[oscillator] |= bit;
            state.legacy_oscillator_automation_released[oscillator] &= !bit;
            changed = true;
            masks_changed = true;
        }
        if let (Some(field), Some(group_index)) = (legacy_group_automation_target(id), group_index)
        {
            let bit = field.mask();
            copy_legacy_group_field(
                &mut state.legacy_group_automation_observed,
                legacy_group,
                field,
            );
            copy_legacy_group_field(
                &mut state.legacy_group_automation_value,
                legacy_group,
                field,
            );
            copy_legacy_group_field(
                &mut state.legacy_group_automation_base,
                state.generator_group_outputs[group_index],
                field,
            );
            state.legacy_group_automation_mask |= bit;
            state.legacy_group_automation_released &= !bit;
            changed = true;
            masks_changed = true;
        }
    }

    if masks_changed {
        stack.set_legacy_automation_masks(
            state.legacy_oscillator_automation_masks,
            state.legacy_group_automation_mask,
            state.legacy_oscillator_automation_released,
            state.legacy_group_automation_released,
            state.legacy_pan_automation_masks,
            state.legacy_pan_automation_released,
        );
    }
    changed
}

const LEGACY_PAN_CENTER: usize = 0;
const LEGACY_PAN_EDGE: usize = 1;
const LEGACY_PAN_CURVE: usize = 2;
const LEGACY_PAN_CURVE_TIME: usize = 3;
const LEGACY_PAN_LEFT: usize = 4;
const LEGACY_PAN_RIGHT: usize = 5;
const LEGACY_PAN_LEFT_CURVE: usize = 6;
const LEGACY_PAN_RIGHT_CURVE: usize = 7;
const LEGACY_PAN_LEFT_TIME: usize = 8;
const LEGACY_PAN_RIGHT_TIME: usize = 9;
const LEGACY_PAN_FIELD_COUNT: usize = 10;
const LEGACY_PAN_PRIMARY_FIELDS: u16 = (1 << LEGACY_PAN_FIELD_COUNT) - 1;
const LEGACY_PAN_SECONDARY_FIELDS: u16 = (1 << LEGACY_PAN_CENTER)
    | (1 << LEGACY_PAN_LEFT)
    | (1 << LEGACY_PAN_RIGHT)
    | (1 << LEGACY_PAN_LEFT_CURVE)
    | (1 << LEGACY_PAN_RIGHT_CURVE)
    | (1 << LEGACY_PAN_LEFT_TIME)
    | (1 << LEGACY_PAN_RIGHT_TIME);

const fn legacy_pan_automation_target(id: u32) -> Option<(usize, usize)> {
    match id {
        35 => Some((0, LEGACY_PAN_CENTER)),
        36 => Some((0, LEGACY_PAN_EDGE)),
        37 => Some((0, LEGACY_PAN_CURVE)),
        39 => Some((0, LEGACY_PAN_CURVE_TIME)),
        40 => Some((0, LEGACY_PAN_LEFT)),
        41 => Some((0, LEGACY_PAN_RIGHT)),
        42 => Some((0, LEGACY_PAN_LEFT_CURVE)),
        43 => Some((0, LEGACY_PAN_RIGHT_CURVE)),
        44 => Some((0, LEGACY_PAN_LEFT_TIME)),
        45 => Some((0, LEGACY_PAN_RIGHT_TIME)),
        83 => Some((1, LEGACY_PAN_CENTER)),
        84 => Some((1, LEGACY_PAN_LEFT)),
        85 => Some((1, LEGACY_PAN_RIGHT)),
        86 => Some((1, LEGACY_PAN_LEFT_CURVE)),
        87 => Some((1, LEGACY_PAN_RIGHT_CURVE)),
        88 => Some((1, LEGACY_PAN_LEFT_TIME)),
        89 => Some((1, LEGACY_PAN_RIGHT_TIME)),
        102 => Some((2, LEGACY_PAN_CENTER)),
        103 => Some((2, LEGACY_PAN_LEFT)),
        104 => Some((2, LEGACY_PAN_RIGHT)),
        105 => Some((2, LEGACY_PAN_LEFT_CURVE)),
        106 => Some((2, LEGACY_PAN_RIGHT_CURVE)),
        107 => Some((2, LEGACY_PAN_LEFT_TIME)),
        108 => Some((2, LEGACY_PAN_RIGHT_TIME)),
        _ => None,
    }
}

fn legacy_pan_values(params: &KurvParams) -> [[f32; 11]; LEGACY_OSCILLATOR_COUNT] {
    [
        [
            params.pan_shape_center.value(),
            params.pan_shape_edge.value(),
            params.pan_shape_curve.value(),
            params.pan_shape_curve_time.value(),
            params.pan_shape_left.value(),
            params.pan_shape_right.value(),
            params.pan_shape_left_curve.value(),
            params.pan_shape_right_curve.value(),
            params.pan_shape_left_curve_time.value(),
            params.pan_shape_right_curve_time.value(),
            params.pan_shape_center_x.value(),
        ],
        [
            params.osc2_pan_shape_center.value(),
            0.0,
            0.0,
            0.0,
            params.osc2_pan_shape_left.value(),
            params.osc2_pan_shape_right.value(),
            params.osc2_pan_shape_left_curve.value(),
            params.osc2_pan_shape_right_curve.value(),
            params.osc2_pan_shape_left_curve_time.value(),
            params.osc2_pan_shape_right_curve_time.value(),
            params.osc2_pan_shape_center_x.value(),
        ],
        [
            params.osc3_pan_shape_center.value(),
            0.0,
            0.0,
            0.0,
            params.osc3_pan_shape_left.value(),
            params.osc3_pan_shape_right.value(),
            params.osc3_pan_shape_left_curve.value(),
            params.osc3_pan_shape_right_curve.value(),
            params.osc3_pan_shape_left_curve_time.value(),
            params.osc3_pan_shape_right_curve_time.value(),
            params.osc3_pan_shape_center_x.value(),
        ],
    ]
}

fn pan_segment_last(segments: crate::pan_curve::PanShapeSegmentsRt) -> usize {
    usize::from(segments.count.saturating_sub(1))
}

fn changed_legacy_pan_fields(
    before: (
        crate::pan_curve::PanShapeSegmentsRt,
        crate::pan_curve::PanShapeSegmentsRt,
    ),
    after: (
        crate::pan_curve::PanShapeSegmentsRt,
        crate::pan_curve::PanShapeSegmentsRt,
    ),
) -> u16 {
    if before.0.count == 0 || before.1.count == 0 || after.0.count == 0 || after.1.count == 0 {
        return LEGACY_PAN_PRIMARY_FIELDS;
    }
    let mut changed = 0;
    if before.0.seg_p0[0].to_bits() != after.0.seg_p0[0].to_bits()
        || before.1.seg_p0[0].to_bits() != after.1.seg_p0[0].to_bits()
    {
        changed |= 1 << LEGACY_PAN_CENTER;
    }
    if before.0.seg_p3[pan_segment_last(before.0)].to_bits()
        != after.0.seg_p3[pan_segment_last(after.0)].to_bits()
    {
        changed |= (1 << LEGACY_PAN_EDGE) | (1 << LEGACY_PAN_LEFT);
    }
    if before.1.seg_p3[pan_segment_last(before.1)].to_bits()
        != after.1.seg_p3[pan_segment_last(after.1)].to_bits()
    {
        changed |= (1 << LEGACY_PAN_EDGE) | (1 << LEGACY_PAN_RIGHT);
    }
    if before.0.seg_p1[0].to_bits() != after.0.seg_p1[0].to_bits() {
        changed |= (1 << LEGACY_PAN_CURVE) | (1 << LEGACY_PAN_LEFT_CURVE);
    }
    if before.1.seg_p1[0].to_bits() != after.1.seg_p1[0].to_bits() {
        changed |= (1 << LEGACY_PAN_CURVE) | (1 << LEGACY_PAN_RIGHT_CURVE);
    }
    if before.0.seg_cx1[0].to_bits() != after.0.seg_cx1[0].to_bits() {
        changed |= (1 << LEGACY_PAN_CURVE_TIME) | (1 << LEGACY_PAN_LEFT_TIME);
    }
    if before.1.seg_cx1[0].to_bits() != after.1.seg_cx1[0].to_bits() {
        changed |= (1 << LEGACY_PAN_CURVE_TIME) | (1 << LEGACY_PAN_RIGHT_TIME);
    }
    changed
}

/// Observe historical pan-shape parameters after the structural curves have
/// been refreshed. Each old lane owns only its corresponding fixed RT segment
/// coordinate; no `Vec` construction or curve compilation occurs on audio.
pub(crate) fn refresh_legacy_pan_automation(
    state: &mut KurvDspState,
    params: &KurvParams,
    events: &EventList,
) -> bool {
    let stack = &params.generator_stack;
    if !stack.legacy_host_automation_bridge_enabled() {
        let changed = state.legacy_pan_automation_initialized
            || state
                .legacy_pan_automation_masks
                .iter()
                .any(|mask| *mask != 0);
        state.legacy_pan_automation_initialized = false;
        state.legacy_pan_automation_masks.fill(0);
        state.legacy_pan_automation_released.fill(0);
        return changed;
    }

    let epoch = stack.legacy_automation_epoch();
    let values = legacy_pan_values(params);
    let mut changed = false;
    let mut masks_changed = false;
    if !state.legacy_pan_automation_initialized || state.legacy_pan_automation_epoch != epoch {
        state.legacy_pan_automation_observed = values;
        state.legacy_pan_automation_values = values;
        state.legacy_pan_automation_bases =
            std::array::from_fn(|index| state.pan_shape_segments[index]);
        state.legacy_pan_automation_epoch = epoch;
        state.legacy_pan_automation_initialized = true;
        changed = state
            .legacy_pan_automation_masks
            .iter()
            .any(|mask| *mask != 0);
    }

    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        let relevant = if oscillator == 0 {
            LEGACY_PAN_PRIMARY_FIELDS
        } else {
            LEGACY_PAN_SECONDARY_FIELDS
        };
        if state.generator_module_ids[oscillator] != oscillator as u64 + 1 {
            masks_changed |= state.legacy_pan_automation_masks[oscillator] != 0;
            state.legacy_pan_automation_masks[oscillator] = 0;
            state.legacy_pan_automation_released[oscillator] |= relevant;
            continue;
        }

        let raw_segments = state.pan_shape_segments[oscillator];
        let structurally_changed =
            changed_legacy_pan_fields(state.legacy_pan_automation_bases[oscillator], raw_segments);
        let released = state.legacy_pan_automation_masks[oscillator] & structurally_changed;
        if released != 0 {
            state.legacy_pan_automation_released[oscillator] |= released;
            state.legacy_pan_automation_masks[oscillator] &= !released;
            changed = true;
            masks_changed = true;
        }
        state.legacy_pan_automation_bases[oscillator] = raw_segments;

        for field in 0..LEGACY_PAN_FIELD_COUNT {
            let bit = 1_u16 << field;
            if relevant & bit == 0
                || state.legacy_pan_automation_observed[oscillator][field].to_bits()
                    == values[oscillator][field].to_bits()
            {
                continue;
            }
            state.legacy_pan_automation_observed[oscillator][field] = values[oscillator][field];
            state.legacy_pan_automation_values[oscillator][field] = values[oscillator][field];
            state.legacy_pan_automation_masks[oscillator] |= bit;
            state.legacy_pan_automation_released[oscillator] &= !bit;
            changed = true;
            masks_changed = true;
        }
    }

    for event in events.iter() {
        let EventBody::ParamChange { id, .. } = event.body else {
            continue;
        };
        let Some((oscillator, field)) = legacy_pan_automation_target(id) else {
            continue;
        };
        if state.generator_module_ids[oscillator] != oscillator as u64 + 1 {
            continue;
        }
        let bit = 1_u16 << field;
        state.legacy_pan_automation_observed[oscillator][field] = values[oscillator][field];
        state.legacy_pan_automation_values[oscillator][field] = values[oscillator][field];
        state.legacy_pan_automation_bases[oscillator] = state.pan_shape_segments[oscillator];
        state.legacy_pan_automation_masks[oscillator] |= bit;
        state.legacy_pan_automation_released[oscillator] &= !bit;
        changed = true;
        masks_changed = true;
    }

    if masks_changed {
        stack.set_legacy_automation_masks(
            state.legacy_oscillator_automation_masks,
            state.legacy_group_automation_mask,
            state.legacy_oscillator_automation_released,
            state.legacy_group_automation_released,
            state.legacy_pan_automation_masks,
            state.legacy_pan_automation_released,
        );
    }
    changed
}

fn set_pan_segment_center(segments: &mut crate::pan_curve::PanShapeSegmentsRt, value: f32) {
    if segments.count != 0 {
        segments.seg_p0[0] = value.clamp(0.0, 1.0);
    }
}

fn set_pan_segment_edge(segments: &mut crate::pan_curve::PanShapeSegmentsRt, value: f32) {
    if segments.count != 0 {
        let last = usize::from(segments.count - 1);
        let value = value.clamp(0.0, 1.0);
        segments.seg_p2[last] = value;
        segments.seg_p3[last] = value;
    }
}

fn set_pan_segment_curve(segments: &mut crate::pan_curve::PanShapeSegmentsRt, value: f32) {
    if segments.count != 0 {
        segments.seg_p1[0] = value.clamp(-1.0, 1.0).mul_add(0.5, 0.5);
    }
}

fn set_pan_segment_time(segments: &mut crate::pan_curve::PanShapeSegmentsRt, value: f32) {
    if segments.count != 0 {
        segments.seg_cx1[0] = value.clamp(0.05, 0.95);
    }
}

pub(crate) fn effective_legacy_pan_segments(
    state: &KurvDspState,
    oscillator: usize,
) -> (
    crate::pan_curve::PanShapeSegmentsRt,
    crate::pan_curve::PanShapeSegmentsRt,
) {
    let mut result = state.pan_shape_segments[oscillator];
    if oscillator >= LEGACY_OSCILLATOR_COUNT {
        return result;
    }
    let mask = state.legacy_pan_automation_masks[oscillator];
    let values = state.legacy_pan_automation_values[oscillator];
    if mask & (1 << LEGACY_PAN_CENTER) != 0 {
        set_pan_segment_center(&mut result.0, values[LEGACY_PAN_CENTER]);
        set_pan_segment_center(&mut result.1, values[LEGACY_PAN_CENTER]);
    }
    if mask & (1 << LEGACY_PAN_EDGE) != 0 {
        set_pan_segment_edge(&mut result.0, values[LEGACY_PAN_EDGE]);
        set_pan_segment_edge(&mut result.1, values[LEGACY_PAN_EDGE]);
    }
    if mask & (1 << LEGACY_PAN_CURVE) != 0 {
        set_pan_segment_curve(&mut result.0, values[LEGACY_PAN_CURVE]);
        set_pan_segment_curve(&mut result.1, values[LEGACY_PAN_CURVE]);
    }
    if mask & (1 << LEGACY_PAN_CURVE_TIME) != 0 {
        set_pan_segment_time(&mut result.0, values[LEGACY_PAN_CURVE_TIME]);
        set_pan_segment_time(&mut result.1, values[LEGACY_PAN_CURVE_TIME]);
    }
    for (field, side, setter) in [
        (LEGACY_PAN_LEFT, 0, set_pan_segment_edge as fn(&mut _, _)),
        (LEGACY_PAN_RIGHT, 1, set_pan_segment_edge as fn(&mut _, _)),
        (
            LEGACY_PAN_LEFT_CURVE,
            0,
            set_pan_segment_curve as fn(&mut _, _),
        ),
        (
            LEGACY_PAN_RIGHT_CURVE,
            1,
            set_pan_segment_curve as fn(&mut _, _),
        ),
        (
            LEGACY_PAN_LEFT_TIME,
            0,
            set_pan_segment_time as fn(&mut _, _),
        ),
        (
            LEGACY_PAN_RIGHT_TIME,
            1,
            set_pan_segment_time as fn(&mut _, _),
        ),
    ] {
        if mask & (1 << field) != 0 {
            setter(
                if side == 0 {
                    &mut result.0
                } else {
                    &mut result.1
                },
                values[field],
            );
        }
    }
    result
}

fn apply_legacy_oscillator_automation(
    oscillators: &mut [generators::OscillatorConfig; generators::MAX_OSCILLATORS],
    masks: &[u32; LEGACY_OSCILLATOR_COUNT],
    values: &[generators::OscillatorConfig; LEGACY_OSCILLATOR_COUNT],
) {
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        for field in LegacyOscillatorField::ALL {
            if masks[oscillator] & field.mask() != 0 {
                copy_legacy_oscillator_field(
                    &mut oscillators[oscillator],
                    values[oscillator],
                    field,
                );
            }
        }
    }
}

pub(crate) fn refresh_host_automation_targets(
    state: &mut KurvDspState,
    params: &KurvParams,
) -> bool {
    let Some((generation, targets)) = params
        .host_automation_targets
        .try_rt_snapshot_after(state.host_automation_generation)
    else {
        return false;
    };
    state.host_automation_generation = generation;
    state.host_automation_targets = targets.map(|target| {
        target.filter(|target| !matches!(target, ModulationRouteTarget::Legacy { .. }))
    });
    state.host_automation_len = 0;
    for (slot, target) in state.host_automation_targets.iter().enumerate() {
        if target.is_some() {
            state.host_automation_slots[usize::from(state.host_automation_len)] = slot as u8;
            state.host_automation_len += 1;
        }
    }
    true
}

pub(crate) fn host_automated_generator_configuration(
    state: &KurvDspState,
    params: &KurvParams,
) -> (
    [generators::OscillatorConfig; generators::MAX_OSCILLATORS],
    [filters::FilterConfig; generators::MAX_FILTERS],
    [generators::GroupOutput; generators::MAX_OUTPUT_PAIRS],
) {
    let mut oscillators = state.generator_oscillators;
    let mut filters = state.generator_filters;
    let mut groups = state.generator_group_outputs;
    apply_legacy_oscillator_automation(
        &mut oscillators,
        &state.legacy_oscillator_automation_masks,
        &state.legacy_oscillator_automation_values,
    );
    if let Some(group_index) = state.generator_group_ids[..state.generator_group_count]
        .iter()
        .position(|id| *id == 1)
    {
        for field in LegacyGroupField::ALL {
            if state.legacy_group_automation_mask & field.mask() != 0 {
                copy_legacy_group_field(
                    &mut groups[group_index],
                    state.legacy_group_automation_value,
                    field,
                );
            }
        }
    }
    for slot in state.host_automation_slots[..usize::from(state.host_automation_len)]
        .iter()
        .copied()
        .map(usize::from)
    {
        let Some(target) = state.host_automation_targets[slot] else {
            continue;
        };
        let normalized = params
            .get_normalized(u32::from(HOST_AUTOMATION_PARAMS[slot]))
            .unwrap_or_default() as f32;
        match target {
            ModulationRouteTarget::Legacy { .. } => {}
            ModulationRouteTarget::Oscillator {
                module_id,
                slot,
                control,
            } => {
                let index = slot.index();
                if state.generator_module_ids[index] != module_id {
                    continue;
                }
                control.apply_normalized(&mut oscillators[index], normalized);
            }
            ModulationRouteTarget::Group { group_id, control } => {
                let Some(index) = state.generator_group_ids[..state.generator_group_count]
                    .iter()
                    .position(|id| *id == group_id)
                else {
                    continue;
                };
                control.apply_normalized(&mut groups[index], normalized);
            }
            ModulationRouteTarget::Filter {
                module_id,
                slot,
                control,
            } => {
                let index = slot.index();
                if state.generator_filter_module_ids[index] != module_id {
                    continue;
                }
                control.apply_normalized(&mut filters[index], normalized);
            }
        }
    }
    (oscillators, filters, groups)
}
