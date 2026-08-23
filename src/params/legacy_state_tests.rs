use truce::params::{FloatParamReadF32, Params};
use truce::prelude::{AudioBuffer, AudioConfig, PluginLogic, ProcessContext, TransportInfo};
use truce_core::{
    events::{Event, EventBody, EventList},
    state::{StateParse, apply_params, hash_plugin_id, parse_state},
};

use super::KurvParams;
use crate::wave_curve::WaveCurveData;

struct ExpectedOscillators {
    shape: f32,
    pulse_width: f32,
    unison_voices: i64,
    osc2_shape: f32,
    osc2_transpose: i64,
    osc2_level: f32,
    osc3_enabled: bool,
    osc3_shape: f32,
    osc3_transpose: i64,
    osc3_level: f32,
}

const FIXTURES: [(&str, &[u8], ExpectedOscillators); 9] = [
    (
        "e9bacd36",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/e9bacd36-2130-433e-bc4a-9d21a314a8e6.clap-preset"
        ),
        ExpectedOscillators {
            shape: 2.0,
            pulse_width: 0.5,
            unison_voices: 61,
            osc2_shape: 2.0,
            osc2_transpose: 12,
            osc2_level: 0.42,
            osc3_enabled: false,
            osc3_shape: 0.0,
            osc3_transpose: 0,
            osc3_level: 1.0,
        },
    ),
    (
        "95557a6a",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/95557a6a-70c9-4505-9413-4ffde7674928.clap-preset"
        ),
        ExpectedOscillators {
            shape: 1.859_959_2,
            pulse_width: 0.487_466_7,
            unison_voices: 64,
            osc2_shape: 1.892_589_8,
            osc2_transpose: 0,
            osc2_level: 0.356_667,
            osc3_enabled: false,
            osc3_shape: 2.675_73,
            osc3_transpose: 12,
            osc3_level: 1.0,
        },
    ),
    (
        "d176cb2e",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/d176cb2e-2af6-483d-a73d-7ab2b71ece28.clap-preset"
        ),
        ExpectedOscillators {
            shape: 1.859_959_2,
            pulse_width: 0.487_466_7,
            unison_voices: 64,
            osc2_shape: 1.892_589_8,
            osc2_transpose: 0,
            osc2_level: 0.356_667,
            osc3_enabled: false,
            osc3_shape: 2.675_73,
            osc3_transpose: 12,
            osc3_level: 1.0,
        },
    ),
    (
        "0700aed3",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/0700aed3-972d-41f9-b760-7c046ce06c2f.clap-preset"
        ),
        ExpectedOscillators {
            shape: 2.0,
            pulse_width: 0.5,
            unison_voices: 64,
            osc2_shape: 2.0,
            osc2_transpose: 12,
            osc2_level: 0.72,
            osc3_enabled: true,
            osc3_shape: 2.838_89,
            osc3_transpose: 0,
            osc3_level: 1.0,
        },
    ),
    (
        "5262e61a",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/5262e61a-3386-48e5-b4cb-db161246420f.clap-preset"
        ),
        ExpectedOscillators {
            shape: 2.0,
            pulse_width: 0.5,
            unison_voices: 64,
            osc2_shape: 2.0,
            osc2_transpose: 12,
            osc2_level: 0.86,
            osc3_enabled: true,
            osc3_shape: 2.838_89,
            osc3_transpose: 0,
            osc3_level: 1.0,
        },
    ),
    (
        "63a28bcd",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/63a28bcd-5e7c-4129-a552-8e8d5af68a06.clap-preset"
        ),
        ExpectedOscillators {
            shape: 3.0,
            pulse_width: 0.487_466_7,
            unison_voices: 2,
            osc2_shape: 1.863_76,
            osc2_transpose: 0,
            osc2_level: 1.0,
            osc3_enabled: false,
            osc3_shape: 2.675_73,
            osc3_transpose: 12,
            osc3_level: 1.0,
        },
    ),
    (
        "9927dee4",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/9927dee4-e649-4522-a3dd-91a7696993be.clap-preset"
        ),
        ExpectedOscillators {
            shape: 1.0,
            pulse_width: 0.725_599_77,
            unison_voices: 17,
            osc2_shape: 2.0,
            osc2_transpose: 0,
            osc2_level: 1.0,
            osc3_enabled: false,
            osc3_shape: 2.0,
            osc3_transpose: 0,
            osc3_level: 1.0,
        },
    ),
    (
        "38a20bf5",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/38a20bf5-9fdd-4725-bf00-2ad08f7e87d7.clap-preset"
        ),
        ExpectedOscillators {
            shape: 3.0,
            pulse_width: 0.412_266_52,
            unison_voices: 15,
            osc2_shape: 2.615_8,
            osc2_transpose: 0,
            osc2_level: 1.0,
            osc3_enabled: true,
            osc3_shape: 2.125_34,
            osc3_transpose: 12,
            osc3_level: 0.723_33,
        },
    ),
    (
        "9d854547",
        include_bytes!(
            "../../tests/fixtures/fireflies-kurv/9d854547-4dd9-43e1-a24b-100a1c071fd9.clap-preset"
        ),
        ExpectedOscillators {
            shape: 2.0,
            pulse_width: 0.5,
            unison_voices: 46,
            osc2_shape: 2.0,
            osc2_transpose: 12,
            osc2_level: 1.0,
            osc3_enabled: false,
            osc3_shape: 2.0,
            osc3_transpose: 0,
            osc3_level: 1.0,
        },
    ),
];

fn clap_state(preset: &[u8]) -> &[u8] {
    assert_eq!(&preset[..4], b"clap");
    let id_len = u32::from_be_bytes(preset[4..8].try_into().expect("CLAP ID length")) as usize;
    assert_eq!(&preset[8..8 + id_len], b"com.prototypelab.kurv");
    &preset[8 + id_len..]
}

fn persist_keys(persist: &[u8]) -> Vec<&str> {
    let mut offset = 0;
    let read_u32 = |offset: &mut usize| {
        let value = u32::from_le_bytes(
            persist[*offset..*offset + 4]
                .try_into()
                .expect("persist u32"),
        );
        *offset += 4;
        value as usize
    };
    let count = read_u32(&mut offset);
    let mut keys = Vec::with_capacity(count);
    for _ in 0..count {
        let key_len = read_u32(&mut offset);
        let key = std::str::from_utf8(&persist[offset..offset + key_len]).expect("persist key");
        offset += key_len;
        let value_len = read_u32(&mut offset);
        offset += value_len;
        keys.push(key);
    }
    assert_eq!(offset, persist.len());
    keys
}

fn assert_near(actual: f32, expected: f32, fixture: &str, field: &str) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "{fixture} {field}: expected {expected}, got {actual}",
    );
}

#[test]
fn fireflies_pre_modular_states_translate_into_the_editable_generator_stack() {
    let plugin_id = hash_plugin_id("com.prototypelab.kurv");
    for (fixture, preset, expected) in FIXTURES {
        let StateParse::Ok(state) = parse_state(clap_state(preset), plugin_id) else {
            panic!("{fixture}: invalid KURV state envelope");
        };
        assert_eq!(state.params.len(), 249, "{fixture}: parameter count");
        let keys = persist_keys(&state.persist);
        assert_eq!(keys.len(), 15, "{fixture}: persist key count");
        assert!(
            !keys.contains(&"generator-stack"),
            "{fixture}: not pre-modular"
        );

        let params = KurvParams::default();
        let mut stale_source = crate::modulators::state::SourceConfig::default();
        stale_source.active = true;
        assert!(params.modulator_rack.set_config(10, stale_source));
        assert!(params.modulation_route_overflow.set(20, 1, 0.5));
        params.mod_wheel_route_mask.store(u64::MAX);
        let stale_slot = crate::generators::OscillatorSlot::from_index(0).unwrap();
        let stale_module = crate::generators::ModuleId::from_raw(1).unwrap();
        assert!(params.host_automation_targets.set(
            0,
            crate::modulators::routing::ModulationRouteTarget::oscillator(
                stale_module,
                stale_slot,
                crate::modulators::routing::OscillatorControl::Shape,
            ),
        ));
        assert!(
            params.generator_stack.is_materialized(),
            "fresh current state"
        );
        for curve in [
            &params.osc1_wave_curve_state,
            &params.osc2_wave_curve_state,
            &params.osc3_wave_curve_state,
        ] {
            curve.edit(|data| data.knots[0].value = 0.5);
            assert_ne!(curve.snapshot(), WaveCurveData::default());
        }

        apply_params(&params, &state);

        assert!(
            params.generator_stack.is_materialized(),
            "{fixture}: a missing generator document must translate into the editable stack",
        );
        assert!(
            !params.modulator_rack.config(10).active,
            "{fixture}: stale source"
        );
        assert_eq!(
            params.modulation_route_overflow.get(20),
            crate::modulators::routing::ExtraModulationRoute::EMPTY,
            "{fixture}: stale overflow route",
        );
        assert_eq!(
            params.mod_wheel_route_mask.load(),
            0,
            "{fixture}: stale MOD mask"
        );
        assert!(
            params.host_automation_targets.get(0).is_none(),
            "{fixture}: stale host binding"
        );
        let patch = params.generator_stack.snapshot();
        assert!(
            params
                .generator_stack
                .legacy_host_automation_bridge_enabled(),
            "{fixture}: historical host automation bridge"
        );
        assert_eq!(patch.groups().len(), 1, "{fixture}: translated group count");
        assert_eq!(
            patch.groups()[0].modules().len(),
            3,
            "{fixture}: translated modules"
        );
        let output = patch.groups()[0].output();
        assert_near(
            output.attack_curve_time,
            params.attack_curve_time.value(),
            fixture,
            "attack curve time",
        );
        assert_near(
            output.decay_curve_time,
            params.decay_curve_time.value(),
            fixture,
            "decay curve time",
        );
        assert_near(
            output.release_curve_time,
            params.release_curve_time.value(),
            fixture,
            "release curve time",
        );
        assert!(
            persist_keys(&params.serialize_persist()).contains(&"generator-stack"),
            "{fixture}: the next save must use the current generator schema",
        );
        let migrated_route = params.modulation_route_targets.get(0);
        match fixture {
            "e9bacd36" => assert!(matches!(
                migrated_route,
                Some(crate::modulators::routing::ModulationRouteTarget::Oscillator {
                    slot,
                    control: crate::modulators::routing::OscillatorControl::PulseWidth,
                    ..
                }) if slot.index() == 0
            )),
            "95557a6a" | "d176cb2e" => assert!(matches!(
                migrated_route,
                Some(crate::modulators::routing::ModulationRouteTarget::Oscillator {
                    slot,
                    control: crate::modulators::routing::OscillatorControl::Level,
                    ..
                }) if slot.index() == 1
            )),
            _ => assert!(migrated_route.is_none(), "{fixture}: stale migrated route"),
        }
        assert_near(params.shape.value(), expected.shape, fixture, "shape");
        assert_near(
            params.pulse_width.value(),
            expected.pulse_width,
            fixture,
            "pulse width",
        );
        assert_eq!(
            params.unison_voices.value(),
            expected.unison_voices,
            "{fixture}"
        );
        assert_near(
            params.osc2_shape.value(),
            expected.osc2_shape,
            fixture,
            "osc2 shape",
        );
        assert_eq!(
            params.osc2_transpose.value(),
            expected.osc2_transpose,
            "{fixture}"
        );
        assert_near(
            params.osc2_level.value(),
            expected.osc2_level,
            fixture,
            "osc2 level",
        );
        assert_eq!(
            params.osc3_enabled.value(),
            expected.osc3_enabled,
            "{fixture}"
        );
        assert_near(
            params.osc3_shape.value(),
            expected.osc3_shape,
            fixture,
            "osc3 shape",
        );
        assert_eq!(
            params.osc3_transpose.value(),
            expected.osc3_transpose,
            "{fixture}"
        );
        assert_near(
            params.osc3_level.value(),
            expected.osc3_level,
            fixture,
            "osc3 level",
        );

        let first = params
            .generator_stack
            .oscillator_config(crate::generators::OscillatorSlot::from_index(0).unwrap());
        let second = params
            .generator_stack
            .oscillator_config(crate::generators::OscillatorSlot::from_index(1).unwrap());
        let third = params
            .generator_stack
            .oscillator_config(crate::generators::OscillatorSlot::from_index(2).unwrap());
        assert_near(first.shape, expected.shape, fixture, "editable osc1 shape");
        assert_eq!(
            first.unison_voices, expected.unison_voices as u8,
            "{fixture}"
        );
        assert_near(
            second.shape,
            expected.osc2_shape,
            fixture,
            "editable osc2 shape",
        );
        assert_near(
            second.level,
            expected.osc2_level,
            fixture,
            "editable osc2 level",
        );
        assert_eq!(third.enabled, expected.osc3_enabled, "{fixture}");
        assert_near(
            third.shape,
            expected.osc3_shape,
            fixture,
            "editable osc3 shape",
        );
        assert_near(
            third.level,
            expected.osc3_level,
            fixture,
            "editable osc3 level",
        );
        let normalized_rate =
            |hz: f32| ((hz.max(0.02) / 0.02).ln() / 5_000.0_f32.ln()).clamp(0.0, 1.0);
        for (
            index,
            (
                config,
                enabled,
                custom_shape,
                pulse_width,
                transpose,
                cents,
                level,
                pan,
                voices,
                range,
                amount,
                curve,
                jitter,
                mode,
                rate,
                width,
                weight,
                phase_position,
                phase_random,
                warp_mode,
                warp_amount,
                alignment,
                alignment_mode,
                center_x,
                stereo_x,
                alternate,
            ),
        ) in [
            (
                first,
                params.osc1_enabled.value(),
                params.osc1_custom_shape.value(),
                params.pulse_width.value(),
                params.osc1_transpose.value_f32(),
                params.osc1_cents.value(),
                params.osc1_level.value(),
                params.osc1_pan.value(),
                params.unison_voices.value_u8(),
                params.unison_detune.value(),
                params.unison_detune_amount.value(),
                params.unison_curve.value(),
                params.unison_swarm.value(),
                params.unison_swarm_mode.value_u8(),
                params.unison_swarm_rate.value(),
                params.unison_stereo.value(),
                params.unison_weight.value(),
                params.osc1_phase_position.value(),
                params.phase_random.value(),
                params.osc1_warp_mode.value_u8(),
                params.osc1_warp_amount.value(),
                params.unison_harmonic_align.value(),
                params.unison_alignment_mode.value_u8(),
                params.pan_shape_center_x.value(),
                params.stereo_x.value(),
                params.stereo_alternate.value(),
            ),
            (
                second,
                params.osc2_enabled.value(),
                params.osc2_custom_shape.value(),
                params.osc2_pulse_width.value(),
                params.osc2_transpose.value_f32(),
                params.osc2_cents.value(),
                params.osc2_level.value(),
                params.osc2_pan.value(),
                params.osc2_unison_voices.value_u8(),
                params.osc2_unison_detune.value(),
                params.osc2_unison_detune_amount.value(),
                params.osc2_unison_curve.value(),
                params.osc2_unison_jitter.value(),
                params.osc2_jitter_mode.value_u8(),
                params.osc2_unison_jitter_rate.value(),
                params.osc2_unison_stereo.value(),
                params.osc2_unison_weight.value(),
                params.osc2_phase_position.value(),
                params.osc2_phase_random.value(),
                params.osc2_warp_mode.value_u8(),
                params.osc2_warp_amount.value(),
                params.osc2_unison_harmonic_align.value(),
                params.osc2_unison_alignment_mode.value_u8(),
                params.osc2_pan_shape_center_x.value(),
                params.osc2_stereo_x.value(),
                params.osc2_stereo_alternate.value(),
            ),
            (
                third,
                params.osc3_enabled.value(),
                params.osc3_custom_shape.value(),
                params.osc3_pulse_width.value(),
                params.osc3_transpose.value_f32(),
                params.osc3_cents.value(),
                params.osc3_level.value(),
                params.osc3_pan.value(),
                params.osc3_unison_voices.value_u8(),
                params.osc3_unison_detune.value(),
                params.osc3_unison_detune_amount.value(),
                params.osc3_unison_curve.value(),
                params.osc3_unison_jitter.value(),
                params.osc3_jitter_mode.value_u8(),
                params.osc3_unison_jitter_rate.value(),
                params.osc3_unison_stereo.value(),
                params.osc3_unison_weight.value(),
                params.osc3_phase_position.value(),
                params.osc3_phase_random.value(),
                params.osc3_warp_mode.value_u8(),
                params.osc3_warp_amount.value(),
                params.osc3_unison_harmonic_align.value(),
                params.osc3_unison_alignment_mode.value_u8(),
                params.osc3_pan_shape_center_x.value(),
                params.osc3_stereo_x.value(),
                params.osc3_stereo_alternate.value(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(config.enabled, enabled, "{fixture}: osc {index} enabled");
            for (actual, wanted, name) in [
                (config.custom_shape, custom_shape, "custom shape"),
                (config.pulse_width, pulse_width, "pulse width"),
                (config.transpose, transpose, "transpose"),
                (config.cents, cents, "cents"),
                (config.level, level, "level"),
                (config.pan, pan, "pan"),
                (config.unison_range, range, "range"),
                (config.unison_amount, amount, "amount"),
                (config.unison_curve, curve, "curve"),
                (config.unison_jitter, jitter, "jitter"),
                (config.unison_rate, normalized_rate(rate), "rate"),
                (config.unison_width, width, "width"),
                (config.unison_weight, weight, "weight"),
                (config.phase_position, phase_position, "phase position"),
                (config.phase_random, phase_random, "phase random"),
                (config.phase_warp_amount, warp_amount, "warp"),
                (config.unison_alignment, alignment, "alignment"),
                (config.unison_pan_center_x, center_x, "center x"),
                (config.unison_stereo_x, stereo_x, "stereo x"),
                (config.unison_stereo_alternate, alternate, "alternate"),
            ] {
                assert_near(actual, wanted, fixture, name);
            }
            assert_eq!(
                config.unison_voices, voices,
                "{fixture}: osc {index} voices"
            );
            assert_eq!(
                config.unison_jitter_mode, mode,
                "{fixture}: osc {index} jitter mode"
            );
            assert_eq!(
                config.phase_warp_mode, warp_mode,
                "{fixture}: osc {index} warp mode"
            );
            assert_eq!(
                config.unison_alignment_mode, alignment_mode,
                "{fixture}: osc {index} alignment mode"
            );
        }

        // These exact sine documents are the oscillator curves stored by all
        // nine presets. Starting from a deliberately different curve proves
        // they were decoded rather than merely left at a constructor default.
        for curve in [
            &params.osc1_wave_curve_state,
            &params.osc2_wave_curve_state,
            &params.osc3_wave_curve_state,
        ] {
            assert_eq!(curve.snapshot(), WaveCurveData::default(), "{fixture}");
        }

        // A host save immediately after migration must reopen as the same
        // editable patch, not select the hidden compatibility renderer again.
        let migrated_persist = params.serialize_persist();
        let reopened = KurvParams::default();
        reopened.load_persist(&migrated_persist);
        assert!(
            reopened.generator_stack.is_materialized(),
            "{fixture}: reopen"
        );
        assert!(
            reopened
                .generator_stack
                .legacy_host_automation_bridge_enabled(),
            "{fixture}: historical automation bridge round trip"
        );
        assert_eq!(
            reopened.modulation_route_targets.snapshot(),
            params.modulation_route_targets.snapshot(),
            "{fixture}: migrated route round trip",
        );
        assert_eq!(
            reopened.generator_stack.snapshot().groups()[0]
                .modules()
                .len(),
            3
        );
        for index in 0..3 {
            let slot = crate::generators::OscillatorSlot::from_index(index).unwrap();
            assert_eq!(
                reopened.generator_stack.oscillator_config(slot),
                params.generator_stack.oscillator_config(slot),
                "{fixture}: oscillator {index} round trip",
            );
            assert_eq!(
                reopened.generator_stack.va_table(slot).snapshot(),
                params.generator_stack.va_table(slot).snapshot(),
                "{fixture}: VA table {index} round trip",
            );
        }
    }
}

#[test]
fn current_generator_documents_still_restore_as_materialized() {
    let current = KurvParams::default();
    assert!(current.generator_stack.is_materialized());
    assert!(
        !current
            .generator_stack
            .legacy_host_automation_bridge_enabled()
    );
    let persist = current.serialize_persist();
    assert!(persist_keys(&persist).contains(&"generator-stack"));

    let restored = KurvParams::default();
    restored.generator_stack.reset_legacy();
    assert!(!restored.generator_stack.is_materialized());
    restored.load_persist(&persist);

    assert!(restored.generator_stack.is_materialized());
    assert!(
        !restored
            .generator_stack
            .legacy_host_automation_bridge_enabled()
    );
}

#[test]
fn empty_persist_does_not_turn_a_fresh_instance_into_legacy_state() {
    let params = KurvParams::default();
    params.load_persist(&[]);
    assert!(params.generator_stack.is_materialized());
}

#[test]
fn unmaterialized_placeholder_stack_translates_the_hidden_legacy_patch() {
    let source = KurvParams::default();
    source.shape.set_value(3.0);
    source.unison_voices.set_value(64);
    source.osc2_enabled.set_value(true);
    source.osc2_shape.set_value(2.0);
    source.osc2_level.set_value(0.8);
    source.osc2_transpose.set_value(12);
    source.generator_stack.reset_legacy();
    assert!(
        !source.generator_stack.is_materialized(),
        "compatibility renderer selected"
    );
    assert_eq!(
        source.generator_stack.snapshot().groups()[0]
            .modules()
            .len(),
        1
    );

    let persist = source.serialize_persist();
    assert!(persist_keys(&persist).contains(&"generator-stack"));

    let restored = KurvParams::default();
    restored.shape.set_value(3.0);
    restored.unison_voices.set_value(64);
    restored.osc2_enabled.set_value(true);
    restored.osc2_shape.set_value(2.0);
    restored.osc2_level.set_value(0.8);
    restored.osc2_transpose.set_value(12);
    restored.load_persist(&persist);

    assert!(
        restored.generator_stack.is_materialized(),
        "old three-oscillator sessions must become the editable stack, not stay on the hidden renderer"
    );
    assert_eq!(
        restored.generator_stack.snapshot().groups()[0]
            .modules()
            .len(),
        3
    );
    let first = restored
        .generator_stack
        .oscillator_config(crate::generators::OscillatorSlot::from_index(0).unwrap());
    let second = restored
        .generator_stack
        .oscillator_config(crate::generators::OscillatorSlot::from_index(1).unwrap());
    assert_near(first.shape, 3.0, "reopen", "pulse shape");
    assert_eq!(first.unison_voices, 64);
    assert!(second.enabled);
    assert_near(second.level, 0.8, "reopen", "osc2 level");
    assert_near(second.transpose, 12.0, "reopen", "osc2 transpose");
}

#[test]
fn unmaterialized_structural_documents_start_rendering_the_saved_patch() {
    let source = KurvParams::default();
    let group = source.generator_stack.snapshot().groups()[0].id();
    assert!(
        source
            .generator_stack
            .edit(|patch| patch.insert_oscillator(group, 1))
            .is_ok()
    );
    source.generator_stack.publish_unmaterialized_for_test();
    assert!(!source.generator_stack.is_materialized());
    assert_eq!(
        source.generator_stack.snapshot().groups()[0]
            .modules()
            .len(),
        2
    );

    let persist = source.serialize_persist();
    let restored = KurvParams::default();
    restored.load_persist(&persist);

    assert!(restored.generator_stack.is_materialized());
    assert_eq!(
        restored.generator_stack.snapshot().groups()[0]
            .modules()
            .len(),
        2,
        "authored topology must not be replaced by the three-oscillator translation"
    );
}

#[test]
fn unmaterialized_rt_snapshot_does_not_select_the_hidden_three_osc_engine() {
    let params = KurvParams::default();
    params.osc2_enabled.set_value(true);
    params.unison_voices.set_value(64);
    params.generator_stack.reset_legacy();
    let snapshot = params
        .generator_stack
        .try_rt_snapshot()
        .expect("published RT snapshot");
    assert_eq!(snapshot.group_count(), 1);
    assert_eq!(
        snapshot.groups()[0].modules().len(),
        1,
        "audio must play the document, not a forged three-oscillator overlay"
    );
    assert_eq!(snapshot.groups()[0].oscillator_mask(), 1);
}

#[test]
fn fireflies_launch_plays_the_translated_modular_stack() {
    let plugin_id = hash_plugin_id("com.prototypelab.kurv");
    let StateParse::Ok(loaded) = parse_state(clap_state(FIXTURES[0].1), plugin_id) else {
        panic!("legacy fixture state");
    };
    let params = KurvParams::default();
    apply_params(&params, &loaded);
    assert!(params.generator_stack.is_materialized());
    let snapshot = params
        .generator_stack
        .try_rt_snapshot()
        .expect("translated RT snapshot");
    assert_eq!(snapshot.group_count(), 1);
    assert_eq!(snapshot.groups()[0].modules().len(), 3);
    assert_eq!(snapshot.groups()[0].oscillator_mask(), 0b111);
    assert_near(snapshot.oscillators()[0].shape, 2.0, "e9bacd36", "shape");
    assert_eq!(snapshot.oscillators()[0].unison_voices, 61);
    assert_near(
        snapshot.oscillators()[1].level,
        0.42,
        "e9bacd36",
        "osc2 level",
    );
}

fn process_note(params: &KurvParams, state: &mut crate::KurvDspState) {
    params.set_sample_rate(48_000.0);
    params.snap_smoothers();
    let frames = 64;
    let mut input_events = EventList::with_capacity(1);
    input_events.push(Event::new(
        0,
        EventBody::NoteOn {
            group: 0,
            channel: 1,
            note: 60,
            velocity: 127,
        },
    ));
    let mut output_events = EventList::with_capacity(0);
    let transport = TransportInfo::default();
    let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let inputs: [&[f32]; 0] = [];
    let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
    let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);
    let _ = <crate::Kurv as PluginLogic>::process(
        state,
        params,
        &mut buffer,
        &input_events,
        &mut context,
    );
}

#[test]
fn launch_reset_translates_unmaterialized_legacy_into_the_modular_stack() {
    let params = KurvParams::default();
    params.shape.set_value(3.0);
    params.unison_voices.set_value(64);
    params.osc2_enabled.set_value(true);
    params.osc2_shape.set_value(2.0);
    params.osc2_level.set_value(0.8);
    params.generator_stack.reset_legacy();
    assert!(!params.generator_stack.is_materialized());

    let mut state = crate::KurvDspState::default();
    <crate::Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, 64));

    assert!(
        params.generator_stack.is_materialized(),
        "activate/reset must copy the hidden host oscillators into the editable stack"
    );
    assert_eq!(
        params.generator_stack.snapshot().groups()[0]
            .modules()
            .len(),
        3
    );
    let first = params
        .generator_stack
        .oscillator_config(crate::generators::OscillatorSlot::from_index(0).unwrap());
    let second = params
        .generator_stack
        .oscillator_config(crate::generators::OscillatorSlot::from_index(1).unwrap());
    assert_near(first.shape, 3.0, "launch", "pulse shape");
    assert_eq!(first.unison_voices, 64);
    assert!(second.enabled);
    assert_near(second.level, 0.8, "launch", "osc2 level");

    process_note(&params, &mut state);
    assert!(
        state.generator_materialized,
        "audio must play the translated stack, not the hidden three-oscillator renderer"
    );
    assert_near(state.generator_oscillators[0].shape, 3.0, "launch", "shape");
    assert_eq!(state.generator_oscillators[0].unison_voices, 64);
    assert!(state.generator_oscillators[1].enabled);
    assert_near(state.generator_oscillators[1].level, 0.8, "launch", "level");
}

#[test]
fn process_never_selects_the_hidden_legacy_engine() {
    let params = KurvParams::default();
    params.shape.set_value(3.0);
    params.unison_voices.set_value(64);
    params.osc2_enabled.set_value(true);
    params.generator_stack.reset_legacy();
    assert!(!params.generator_stack.is_materialized());

    let mut state = crate::KurvDspState::default();
    process_note(&params, &mut state);

    assert!(
        state.generator_materialized,
        "process must not fall back to VoiceSettings when the document is still a placeholder"
    );
    assert_eq!(
        state.generator_oscillators[0].unison_voices, 1,
        "untranslated audio must play the document, not host unison"
    );
    assert!(
        !state.generator_oscillators[1].enabled,
        "untranslated audio must not invent the hidden second oscillator"
    );
}

#[test]
fn fireflies_process_plays_the_translated_modular_stack() {
    let plugin_id = hash_plugin_id("com.prototypelab.kurv");
    let StateParse::Ok(loaded) = parse_state(clap_state(FIXTURES[0].1), plugin_id) else {
        panic!("legacy fixture state");
    };
    let params = KurvParams::default();
    apply_params(&params, &loaded);
    assert!(params.generator_stack.is_materialized());

    let mut state = crate::KurvDspState::default();
    <crate::Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, 64));
    process_note(&params, &mut state);

    assert!(state.generator_materialized);
    assert_eq!(state.generator_group_count, 1);
    assert_near(
        state.generator_oscillators[0].shape,
        2.0,
        "e9bacd36",
        "shape",
    );
    assert_eq!(state.generator_oscillators[0].unison_voices, 61);
    assert!(state.generator_oscillators[1].enabled);
    assert_near(
        state.generator_oscillators[1].level,
        0.42,
        "e9bacd36",
        "osc2 level",
    );
}

#[test]
fn historical_generator_automation_tracks_only_explicit_old_id_fields() {
    let plugin_id = hash_plugin_id("com.prototypelab.kurv");
    let StateParse::Ok(loaded) = parse_state(clap_state(FIXTURES[0].1), plugin_id) else {
        panic!("legacy fixture state");
    };
    let params = KurvParams::default();
    apply_params(&params, &loaded);
    assert!(
        params
            .generator_stack
            .legacy_host_automation_bridge_enabled()
    );

    let snapshot = params
        .generator_stack
        .try_rt_snapshot()
        .expect("materialized RT snapshot");
    let mut state = crate::KurvDspState::default();
    state.generator_oscillators = *snapshot.oscillators();
    state.generator_module_ids = *snapshot.module_ids();
    state.generator_group_count = snapshot.group_count();
    for (index, group) in snapshot.groups().iter().copied().enumerate() {
        state.generator_group_ids[index] = group.id();
        state.generator_group_outputs[index] = group.output();
    }

    // Establish the loaded legacy baseline before simulating a stopped-host
    // parameter flush and independent modern editor changes.
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default(),
        )
    );

    let structural_shape = 2.75;
    let structural_pulse = 0.41;
    state.generator_oscillators[0].shape = structural_shape;
    state.generator_oscillators[0].pulse_width = structural_pulse;
    params.shape.set_value(0.625);
    params.attack_curve.set_value(0.375);
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (automated, _, automated_groups) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(automated[0].shape, 0.625, "automation", "shape");
    assert_near(
        automated_groups[0].attack_curve,
        0.375,
        "automation",
        "attack curve",
    );
    // Persisted masks preserve held lanes when a host restores parameter values
    // without replaying process events (the CLAP state-load path).
    let persisted = params.serialize_persist();
    let reopened = KurvParams::default();
    reopened.load_persist(&persisted);
    reopened.shape.set_value(0.625);
    reopened.attack_curve.set_value(0.375);
    let reopened_snapshot = reopened
        .generator_stack
        .try_rt_snapshot()
        .expect("reopened RT snapshot");
    let mut reopened_state = crate::KurvDspState::default();
    reopened_state.generator_oscillators = *reopened_snapshot.oscillators();
    reopened_state.generator_module_ids = *reopened_snapshot.module_ids();
    reopened_state.generator_group_count = reopened_snapshot.group_count();
    for (index, group) in reopened_snapshot.groups().iter().copied().enumerate() {
        reopened_state.generator_group_ids[index] = group.id();
        reopened_state.generator_group_outputs[index] = group.output();
    }
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut reopened_state,
            &reopened,
            &EventList::default(),
        )
    );
    let (reopened_effective, _, reopened_groups) =
        crate::runtime::render::host_automated_generator_configuration(&reopened_state, &reopened);
    assert_near(
        reopened_effective[0].shape,
        0.625,
        "automation reopen",
        "shape",
    );
    assert_near(
        reopened_groups[0].attack_curve,
        0.375,
        "automation reopen",
        "attack curve",
    );

    assert_near(
        automated[0].pulse_width,
        structural_pulse,
        "automation",
        "unrelated structural pulse width",
    );

    // Hosts need not resend a constant lane every block.
    assert!(
        !crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (held, _, held_groups) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(held[0].shape, 0.625, "automation", "held shape");
    assert_near(
        held_groups[0].attack_curve,
        0.375,
        "automation",
        "held attack curve",
    );

    // An editor change to a different structural field coexists with the lane.
    state.generator_oscillators[0].pulse_width = 0.73;
    assert!(
        !crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (coexisting, _, _) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(coexisting[0].shape, 0.625, "automation", "coexisting shape");
    assert_near(
        coexisting[0].pulse_width,
        0.73,
        "automation",
        "coexisting structural edit",
    );

    // Editing the automated structural field releases the old overlay until a
    // subsequent old-ID event explicitly retakes control.
    state.generator_oscillators[0].shape = 1.25;
    state.generator_group_outputs[0].attack_curve = -0.45;
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (released, _, released_groups) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(released[0].shape, 1.25, "automation", "released shape");
    assert_near(
        released_groups[0].attack_curve,
        -0.45,
        "automation",
        "released attack curve",
    );

    // An explicit old-ID event retakes ownership even when its value is
    // identical to the last observed value and therefore invisible in the
    // atomic parameter snapshot.
    let mut same_value_events = EventList::default();
    same_value_events.push(Event::new(
        0,
        EventBody::ParamChange {
            id: 1,
            value: 0.625,
        },
    ));
    same_value_events.push(Event::new(
        0,
        EventBody::ParamChange {
            id: 24,
            value: 0.375,
        },
    ));
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &same_value_events,
        )
    );
    let (retaken, _, retaken_groups) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(retaken[0].shape, 0.625, "automation", "retaken shape");
    assert_near(
        retaken_groups[0].attack_curve,
        0.375,
        "automation",
        "retaken attack curve",
    );
}

fn legacy_automation_test_state(params: &KurvParams) -> crate::KurvDspState {
    let snapshot = params
        .generator_stack
        .try_rt_snapshot()
        .expect("materialized RT snapshot");
    let mut state = crate::KurvDspState::default();
    state.generator_oscillators = *snapshot.oscillators();
    state.generator_module_ids = *snapshot.module_ids();
    state.generator_group_count = snapshot.group_count();
    for (index, group) in snapshot.groups().iter().copied().enumerate() {
        state.generator_group_ids[index] = group.id();
        state.generator_group_outputs[index] = group.output();
    }
    crate::runtime::render::refresh_legacy_materialized_automation(
        &mut state,
        params,
        &EventList::default(),
    );
    for oscillator in 0..3 {
        let slot = crate::generators::OscillatorSlot::from_index(oscillator).unwrap();
        state.pan_shape_segments[oscillator] = params
            .generator_stack
            .pan_shape_curve(slot)
            .try_segments_rt()
            .expect("pan RT segments");
    }
    crate::runtime::render::refresh_legacy_pan_automation(
        &mut state,
        params,
        &EventList::default(),
    );
    state
}

#[test]
fn historical_pan_shape_ids_overlay_fixed_rt_segments_and_round_trip() {
    let plugin_id = hash_plugin_id("com.prototypelab.kurv");
    let StateParse::Ok(loaded) = parse_state(clap_state(FIXTURES[0].1), plugin_id) else {
        panic!("legacy fixture state");
    };
    let params = KurvParams::default();
    apply_params(&params, &loaded);
    let mut state = legacy_automation_test_state(&params);

    // IDs 35, 36, 37, 39..=45: symmetric compatibility controls and
    // independent modernized side controls all remain audible.
    params.pan_shape_center.set_value(0.21);
    params.pan_shape_edge.set_value(0.71);
    params.pan_shape_curve.set_value(-0.4);
    params.pan_shape_curve_time.set_value(0.31);
    params.pan_shape_left.set_value(0.61);
    params.pan_shape_right.set_value(0.62);
    params.pan_shape_left_curve.set_value(-0.3);
    params.pan_shape_right_curve.set_value(0.4);
    params.pan_shape_left_curve_time.set_value(0.33);
    params.pan_shape_right_curve_time.set_value(0.66);

    // IDs 83..=89 and 102..=108 cover both secondary oscillator curves.
    params.osc2_pan_shape_center.set_value(0.22);
    params.osc2_pan_shape_left.set_value(0.52);
    params.osc2_pan_shape_right.set_value(0.72);
    params.osc2_pan_shape_left_curve.set_value(-0.2);
    params.osc2_pan_shape_right_curve.set_value(0.2);
    params.osc2_pan_shape_left_curve_time.set_value(0.34);
    params.osc2_pan_shape_right_curve_time.set_value(0.64);
    params.osc3_pan_shape_center.set_value(0.23);
    params.osc3_pan_shape_left.set_value(0.53);
    params.osc3_pan_shape_right.set_value(0.73);
    params.osc3_pan_shape_left_curve.set_value(-0.1);
    params.osc3_pan_shape_right_curve.set_value(0.1);
    params.osc3_pan_shape_left_curve_time.set_value(0.36);
    params.osc3_pan_shape_right_curve_time.set_value(0.63);
    assert!(crate::runtime::render::refresh_legacy_pan_automation(
        &mut state,
        &params,
        &EventList::default()
    ));

    let primary = crate::runtime::render::effective_legacy_pan_segments(&state, 0);
    assert_near(primary.0.seg_p0[0], 0.21, "pan IDs", "primary center");
    assert_near(primary.1.seg_p0[0], 0.21, "pan IDs", "primary center right");
    assert_near(primary.0.seg_p3[0], 0.61, "pan IDs", "primary left");
    assert_near(primary.1.seg_p3[0], 0.62, "pan IDs", "primary right");
    assert_near(primary.0.seg_p1[0], 0.35, "pan IDs", "primary left curve");
    assert_near(primary.1.seg_p1[0], 0.7, "pan IDs", "primary right curve");
    assert_near(primary.0.seg_cx1[0], 0.33, "pan IDs", "primary left time");
    assert_near(primary.1.seg_cx1[0], 0.66, "pan IDs", "primary right time");

    let second = crate::runtime::render::effective_legacy_pan_segments(&state, 1);
    assert_near(second.0.seg_p0[0], 0.22, "pan IDs", "osc2 center");
    assert_near(second.0.seg_p3[0], 0.52, "pan IDs", "osc2 left");
    assert_near(second.1.seg_p3[0], 0.72, "pan IDs", "osc2 right");
    let third = crate::runtime::render::effective_legacy_pan_segments(&state, 2);
    assert_near(third.0.seg_p0[0], 0.23, "pan IDs", "osc3 center");
    assert_near(third.0.seg_p3[0], 0.53, "pan IDs", "osc3 left");
    assert_near(third.1.seg_p3[0], 0.73, "pan IDs", "osc3 right");

    // IDs 90 and 109 are center-X scalar overlays in the oscillator config.
    params.osc2_pan_shape_center_x.set_value(0.42);
    params.osc3_pan_shape_center_x.set_value(0.58);
    assert!(
        crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (effective, _, _) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(
        effective[1].unison_pan_center_x,
        0.42,
        "pan IDs",
        "osc2 center X",
    );
    assert_near(
        effective[2].unison_pan_center_x,
        0.58,
        "pan IDs",
        "osc3 center X",
    );

    let persisted = params.serialize_persist();
    let reopened = KurvParams::default();
    reopened.load_persist(&persisted);
    reopened.pan_shape_left.set_value(0.61);
    reopened.osc2_pan_shape_right.set_value(0.72);
    reopened.osc3_pan_shape_left_curve_time.set_value(0.36);
    let reopened_state = legacy_automation_test_state(&reopened);
    let reopened_primary =
        crate::runtime::render::effective_legacy_pan_segments(&reopened_state, 0);
    let reopened_second = crate::runtime::render::effective_legacy_pan_segments(&reopened_state, 1);
    let reopened_third = crate::runtime::render::effective_legacy_pan_segments(&reopened_state, 2);
    assert_near(
        reopened_primary.0.seg_p3[0],
        0.61,
        "pan reopen",
        "primary left",
    );
    assert_near(
        reopened_second.1.seg_p3[0],
        0.72,
        "pan reopen",
        "osc2 right",
    );
    assert_near(
        reopened_third.0.seg_cx1[0],
        0.36,
        "pan reopen",
        "osc3 left time",
    );

    // Editing one structural side releases only that side's historical lane.
    state.pan_shape_segments[0].0.seg_p3[0] = 0.91;
    assert!(crate::runtime::render::refresh_legacy_pan_automation(
        &mut state,
        &params,
        &EventList::default()
    ));
    let released = crate::runtime::render::effective_legacy_pan_segments(&state, 0);
    assert_near(
        released.0.seg_p3[0],
        0.91,
        "pan release",
        "left structural edit",
    );
    assert_near(released.1.seg_p3[0], 0.62, "pan release", "right lane held");

    let mut same_value_pan_event = EventList::default();
    same_value_pan_event.push(Event::new(
        0,
        EventBody::ParamChange {
            id: 40,
            value: 0.61,
        },
    ));
    assert!(crate::runtime::render::refresh_legacy_pan_automation(
        &mut state,
        &params,
        &same_value_pan_event,
    ));
    let retaken = crate::runtime::render::effective_legacy_pan_segments(&state, 0);
    assert_near(
        retaken.0.seg_p3[0],
        0.61,
        "pan release",
        "same-value event retakes left lane",
    );
}

#[test]
fn current_structural_documents_ignore_historical_generator_events() {
    let params = KurvParams::default();
    assert!(
        !params
            .generator_stack
            .legacy_host_automation_bridge_enabled()
    );
    let snapshot = params
        .generator_stack
        .try_rt_snapshot()
        .expect("RT snapshot");
    let mut state = crate::KurvDspState::default();
    state.generator_oscillators = *snapshot.oscillators();
    state.generator_module_ids = *snapshot.module_ids();
    state.generator_oscillators[0].shape = 2.4;
    params.shape.set_value(0.2);
    assert!(
        !crate::runtime::render::refresh_legacy_materialized_automation(
            &mut state,
            &params,
            &EventList::default()
        )
    );
    let (effective, _, _) =
        crate::runtime::render::host_automated_generator_configuration(&state, &params);
    assert_near(effective[0].shape, 2.4, "current", "shape");
}
