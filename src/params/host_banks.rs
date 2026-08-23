//! Canonical indexed projections of KURV's frozen numbered host parameters.

use super::P;

#[derive(Clone, Copy)]
pub(crate) struct HostLfoParameterIds {
    pub active: P,
    pub envelope: P,
    pub rate: P,
    pub rate_mode: P,
    pub mode: P,
    pub phase: P,
    pub sync: P,
    pub bipolar: P,
    pub shape: P,
    pub attack: P,
    pub decay: P,
    pub sustain: P,
    pub release: P,
}

#[derive(Clone, Copy)]
pub(crate) struct HostRouteParameterIds {
    pub source: P,
    pub target: P,
    pub amount: P,
    pub target_ext: P,
}

macro_rules! host_lfo_schema {
    ($consumer:ident $(, $params:expr)?) => {
        $consumer! {
            $($params;)?
            (0, lfo1_active, Lfo1Active, source1_envelope, Source1Envelope, lfo1_rate, Lfo1Rate, lfo1_rate_mode, Lfo1RateMode, lfo1_mode, Lfo1Mode, lfo1_phase, Lfo1Phase, lfo1_sync, Lfo1Sync, lfo1_bipolar, Lfo1Bipolar, lfo1_shape, Lfo1Shape, source1_attack, Source1Attack, source1_decay, Source1Decay, source1_sustain, Source1Sustain, source1_release, Source1Release, lfo1_phase_meter, lfo1_value_meter, lfo1_curve_state),
            (1, lfo2_active, Lfo2Active, source2_envelope, Source2Envelope, lfo2_rate, Lfo2Rate, lfo2_rate_mode, Lfo2RateMode, lfo2_mode, Lfo2Mode, lfo2_phase, Lfo2Phase, lfo2_sync, Lfo2Sync, lfo2_bipolar, Lfo2Bipolar, lfo2_shape, Lfo2Shape, source2_attack, Source2Attack, source2_decay, Source2Decay, source2_sustain, Source2Sustain, source2_release, Source2Release, lfo2_phase_meter, lfo2_value_meter, lfo2_curve_state),
            (2, lfo3_active, Lfo3Active, source3_envelope, Source3Envelope, lfo3_rate, Lfo3Rate, lfo3_rate_mode, Lfo3RateMode, lfo3_mode, Lfo3Mode, lfo3_phase, Lfo3Phase, lfo3_sync, Lfo3Sync, lfo3_bipolar, Lfo3Bipolar, lfo3_shape, Lfo3Shape, source3_attack, Source3Attack, source3_decay, Source3Decay, source3_sustain, Source3Sustain, source3_release, Source3Release, lfo3_phase_meter, lfo3_value_meter, lfo3_curve_state),
            (3, lfo4_active, Lfo4Active, source4_envelope, Source4Envelope, lfo4_rate, Lfo4Rate, lfo4_rate_mode, Lfo4RateMode, lfo4_mode, Lfo4Mode, lfo4_phase, Lfo4Phase, lfo4_sync, Lfo4Sync, lfo4_bipolar, Lfo4Bipolar, lfo4_shape, Lfo4Shape, source4_attack, Source4Attack, source4_decay, Source4Decay, source4_sustain, Source4Sustain, source4_release, Source4Release, lfo4_phase_meter, lfo4_value_meter, lfo4_curve_state),
            (4, lfo5_active, Lfo5Active, source5_envelope, Source5Envelope, lfo5_rate, Lfo5Rate, lfo5_rate_mode, Lfo5RateMode, lfo5_mode, Lfo5Mode, lfo5_phase, Lfo5Phase, lfo5_sync, Lfo5Sync, lfo5_bipolar, Lfo5Bipolar, lfo5_shape, Lfo5Shape, source5_attack, Source5Attack, source5_decay, Source5Decay, source5_sustain, Source5Sustain, source5_release, Source5Release, lfo5_phase_meter, lfo5_value_meter, lfo5_curve_state),
            (5, lfo6_active, Lfo6Active, source6_envelope, Source6Envelope, lfo6_rate, Lfo6Rate, lfo6_rate_mode, Lfo6RateMode, lfo6_mode, Lfo6Mode, lfo6_phase, Lfo6Phase, lfo6_sync, Lfo6Sync, lfo6_bipolar, Lfo6Bipolar, lfo6_shape, Lfo6Shape, source6_attack, Source6Attack, source6_decay, Source6Decay, source6_sustain, Source6Sustain, source6_release, Source6Release, lfo6_phase_meter, lfo6_value_meter, lfo6_curve_state),
            (6, lfo7_active, Lfo7Active, source7_envelope, Source7Envelope, lfo7_rate, Lfo7Rate, lfo7_rate_mode, Lfo7RateMode, lfo7_mode, Lfo7Mode, lfo7_phase, Lfo7Phase, lfo7_sync, Lfo7Sync, lfo7_bipolar, Lfo7Bipolar, lfo7_shape, Lfo7Shape, source7_attack, Source7Attack, source7_decay, Source7Decay, source7_sustain, Source7Sustain, source7_release, Source7Release, lfo7_phase_meter, lfo7_value_meter, lfo7_curve_state),
            (7, lfo8_active, Lfo8Active, source8_envelope, Source8Envelope, lfo8_rate, Lfo8Rate, lfo8_rate_mode, Lfo8RateMode, lfo8_mode, Lfo8Mode, lfo8_phase, Lfo8Phase, lfo8_sync, Lfo8Sync, lfo8_bipolar, Lfo8Bipolar, lfo8_shape, Lfo8Shape, source8_attack, Source8Attack, source8_decay, Source8Decay, source8_sustain, Source8Sustain, source8_release, Source8Release, lfo8_phase_meter, lfo8_value_meter, lfo8_curve_state),
        }
    };
}

macro_rules! host_route_schema {
    ($consumer:ident $(, $params:expr)?) => {
        $consumer! {
            $($params;)?
            (mod1_source, Mod1Source, mod1_target, Mod1Target, mod1_amount, Mod1Amount, mod1_target_ext, Mod1TargetExt),
            (mod2_source, Mod2Source, mod2_target, Mod2Target, mod2_amount, Mod2Amount, mod2_target_ext, Mod2TargetExt),
            (mod3_source, Mod3Source, mod3_target, Mod3Target, mod3_amount, Mod3Amount, mod3_target_ext, Mod3TargetExt),
            (mod4_source, Mod4Source, mod4_target, Mod4Target, mod4_amount, Mod4Amount, mod4_target_ext, Mod4TargetExt),
            (mod5_source, Mod5Source, mod5_target, Mod5Target, mod5_amount, Mod5Amount, mod5_target_ext, Mod5TargetExt),
            (mod6_source, Mod6Source, mod6_target, Mod6Target, mod6_amount, Mod6Amount, mod6_target_ext, Mod6TargetExt),
            (mod7_source, Mod7Source, mod7_target, Mod7Target, mod7_amount, Mod7Amount, mod7_target_ext, Mod7TargetExt),
            (mod8_source, Mod8Source, mod8_target, Mod8Target, mod8_amount, Mod8Amount, mod8_target_ext, Mod8TargetExt),
            (mod9_source, Mod9Source, mod9_target, Mod9Target, mod9_amount, Mod9Amount, mod9_target_ext, Mod9TargetExt),
            (mod10_source, Mod10Source, mod10_target, Mod10Target, mod10_amount, Mod10Amount, mod10_target_ext, Mod10TargetExt),
            (mod11_source, Mod11Source, mod11_target, Mod11Target, mod11_amount, Mod11Amount, mod11_target_ext, Mod11TargetExt),
            (mod12_source, Mod12Source, mod12_target, Mod12Target, mod12_amount, Mod12Amount, mod12_target_ext, Mod12TargetExt),
            (mod13_source, Mod13Source, mod13_target, Mod13Target, mod13_amount, Mod13Amount, mod13_target_ext, Mod13TargetExt),
            (mod14_source, Mod14Source, mod14_target, Mod14Target, mod14_amount, Mod14Amount, mod14_target_ext, Mod14TargetExt),
            (mod15_source, Mod15Source, mod15_target, Mod15Target, mod15_amount, Mod15Amount, mod15_target_ext, Mod15TargetExt),
            (mod16_source, Mod16Source, mod16_target, Mod16Target, mod16_amount, Mod16Amount, mod16_target_ext, Mod16TargetExt),
        }
    };
}

macro_rules! define_host_parameter_ids {
    ($(($index:literal, $active_field:ident, $active:ident, $envelope_field:ident, $envelope:ident, $rate_field:ident, $rate:ident, $rate_mode_field:ident, $rate_mode:ident, $mode_field:ident, $mode:ident, $phase_field:ident, $phase:ident, $sync_field:ident, $sync:ident, $bipolar_field:ident, $bipolar:ident, $shape_field:ident, $shape:ident, $attack_field:ident, $attack:ident, $decay_field:ident, $decay:ident, $sustain_field:ident, $sustain:ident, $release_field:ident, $release:ident, $phase_meter:ident, $value_meter:ident, $curve:ident)),+ $(,)?) => {
        pub(crate) const HOST_LFO_PARAMETER_IDS: [HostLfoParameterIds; 8] = [$(
            HostLfoParameterIds {
                active: P::$active,
                envelope: P::$envelope,
                rate: P::$rate,
                rate_mode: P::$rate_mode,
                mode: P::$mode,
                phase: P::$phase,
                sync: P::$sync,
                bipolar: P::$bipolar,
                shape: P::$shape,
                attack: P::$attack,
                decay: P::$decay,
                sustain: P::$sustain,
                release: P::$release,
            },
        )+];
    };
}

macro_rules! define_host_route_parameter_ids {
    ($(($source_field:ident, $source:ident, $target_field:ident, $target:ident, $amount_field:ident, $amount:ident, $target_ext_field:ident, $target_ext:ident)),+ $(,)?) => {
        pub(crate) const HOST_ROUTE_PARAMETER_IDS: [HostRouteParameterIds; 16] = [$(
            HostRouteParameterIds {
                source: P::$source,
                target: P::$target,
                amount: P::$amount,
                target_ext: P::$target_ext,
            },
        )+];
    };
}

host_lfo_schema!(define_host_parameter_ids);
host_route_schema!(define_host_route_parameter_ids);

pub(crate) use host_lfo_schema;
pub(crate) use host_route_schema;
