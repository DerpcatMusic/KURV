use truce::params::Params;
use truce_core::editor::PluginContext;

use crate::pan_curve::PanShapeCurveData;
use crate::{KurvParams, P};

fn reset_editor_state(editor: &mut crate::KurvEditorState) {
    editor.collapsed_group_ids.clear();
    editor.group_accents.clear();
    editor.group_names.clear();
    editor.collapsed_modulators = 0;
    editor.persistent_modulation_cables = false;
}

/// The header's double-click is the one unambiguous factory-reset gesture.
/// Defaults come from Truce's generated parameter metadata, so this remains
/// correct if a range's normalized representation changes later.
pub(crate) fn reset_to_defaults(state: &PluginContext<KurvParams>) {
    state.generator_stack.reset_default();
    state.resynth_assets.clear();
    if let Ok(mut editor) = state.params().editor_state.lock() {
        reset_editor_state(&mut editor);
    }
    let parameters = [
        P::OutputDb,
        P::Osc1Enabled,
        P::Osc1Transpose,
        P::Osc1Cents,
        P::Osc1Level,
        P::Osc1Pan,
        P::Osc1PhasePosition,
        P::Osc1WarpMode,
        P::Osc1WarpAmount,
        P::Shape,
        P::PulseWidth,
        P::Osc2Enabled,
        P::Osc2Shape,
        P::Osc2PulseWidth,
        P::Osc2Transpose,
        P::Osc2Cents,
        P::Osc2Level,
        P::Osc2Pan,
        P::Osc2PhasePosition,
        P::Osc2WarpMode,
        P::Osc2WarpAmount,
        P::Osc2JitterMode,
        P::Osc2UnisonVoices,
        P::Osc2UnisonDetune,
        P::Osc2UnisonDetuneAmount,
        P::Osc2UnisonStereo,
        P::Osc2PhaseRandom,
        P::Osc2UnisonCurve,
        P::Osc2UnisonJitter,
        P::Osc2UnisonJitterRate,
        P::Osc2StereoAlternate,
        P::Osc2StereoX,
        P::Osc2UnisonWeight,
        P::Osc2PanShapeCenter,
        P::Osc2PanShapeLeft,
        P::Osc2PanShapeRight,
        P::Osc2PanShapeLeftCurve,
        P::Osc2PanShapeRightCurve,
        P::Osc2PanShapeLeftCurveTime,
        P::Osc2PanShapeRightCurveTime,
        P::Osc2PanShapeCenterX,
        P::Osc3Enabled,
        P::Osc3Shape,
        P::Osc3PulseWidth,
        P::Osc3Transpose,
        P::Osc3Cents,
        P::Osc3Level,
        P::Osc3Pan,
        P::Osc3PhasePosition,
        P::Osc3WarpMode,
        P::Osc3WarpAmount,
        P::Osc3JitterMode,
        P::Osc3UnisonVoices,
        P::Osc3UnisonDetune,
        P::Osc3UnisonDetuneAmount,
        P::Osc3UnisonStereo,
        P::Osc3PhaseRandom,
        P::Osc3UnisonCurve,
        P::Osc3UnisonJitter,
        P::Osc3UnisonJitterRate,
        P::Osc3StereoAlternate,
        P::Osc3StereoX,
        P::Osc3UnisonWeight,
        P::Osc3PanShapeCenter,
        P::Osc3PanShapeLeft,
        P::Osc3PanShapeRight,
        P::Osc3PanShapeLeftCurve,
        P::Osc3PanShapeRightCurve,
        P::Osc3PanShapeLeftCurveTime,
        P::Osc3PanShapeRightCurveTime,
        P::Osc3PanShapeCenterX,
        P::Attack,
        P::Decay,
        P::Sustain,
        P::Release,
        P::Drone,
        P::DroneFrequency,
        P::PitchBend,
        P::SustainPedal,
        P::UnisonVoices,
        P::UnisonDetune,
        P::UnisonStereo,
        P::PhaseRandom,
        P::UnisonCurve,
        P::VelocityAmount,
        P::PressureAmount,
        P::TimbreAmount,
        P::MpeBendRange,
        P::UnisonSwarm,
        P::UnisonSwarmRate,
        P::UnisonSwarmMode,
        P::Transpose,
        P::OctaveShift,
        P::VoiceMode,
        P::GlideTime,
        P::StereoPattern,
        P::AttackCurve,
        P::DecayCurve,
        P::ReleaseCurve,
        P::AttackCurveTime,
        P::DecayCurveTime,
        P::ReleaseCurveTime,
        P::StereoAlternate,
        P::StereoX,
        P::UnisonWeight,
        P::Oversampling,
        P::UnisonDetuneAmount,
        P::PanShapeCenter,
        P::PanShapeEdge,
        P::PanShapeCurve,
        P::PanShapeCurveTime,
        P::Antialiasing,
        P::GeneratorEngine,
        P::PanShapeLeft,
        P::PanShapeRight,
        P::PanShapeLeftCurve,
        P::PanShapeRightCurve,
        P::PanShapeLeftCurveTime,
        P::PanShapeRightCurveTime,
        P::PanShapeCenterX,
        P::Lfo1Rate,
        P::Lfo1Mode,
        P::Lfo1Phase,
        P::Lfo1Sync,
        P::Lfo1Bipolar,
        P::Lfo2Rate,
        P::Lfo2Mode,
        P::Lfo2Phase,
        P::Lfo2Sync,
        P::Lfo2Bipolar,
        P::Lfo3Rate,
        P::Lfo3Mode,
        P::Lfo3Phase,
        P::Lfo3Sync,
        P::Lfo3Bipolar,
        P::Lfo4Rate,
        P::Lfo4Mode,
        P::Lfo4Phase,
        P::Lfo4Sync,
        P::Lfo4Bipolar,
        P::Lfo1RateMode,
        P::Lfo1Shape,
        P::Lfo2RateMode,
        P::Lfo2Shape,
        P::Lfo3RateMode,
        P::Lfo3Shape,
        P::Lfo4RateMode,
        P::Lfo4Shape,
        P::Lfo5Rate,
        P::Lfo5Mode,
        P::Lfo5Phase,
        P::Lfo5Sync,
        P::Lfo5Bipolar,
        P::Lfo5RateMode,
        P::Lfo5Shape,
        P::Lfo6Rate,
        P::Lfo6Mode,
        P::Lfo6Phase,
        P::Lfo6Sync,
        P::Lfo6Bipolar,
        P::Lfo6RateMode,
        P::Lfo6Shape,
        P::Lfo7Rate,
        P::Lfo7Mode,
        P::Lfo7Phase,
        P::Lfo7Sync,
        P::Lfo7Bipolar,
        P::Lfo7RateMode,
        P::Lfo7Shape,
        P::Lfo8Rate,
        P::Lfo8Mode,
        P::Lfo8Phase,
        P::Lfo8Sync,
        P::Lfo8Bipolar,
        P::Lfo8RateMode,
        P::Lfo8Shape,
        P::Mod1Source,
        P::Mod1Target,
        P::Mod1Amount,
        P::Mod2Source,
        P::Mod2Target,
        P::Mod2Amount,
        P::Mod3Source,
        P::Mod3Target,
        P::Mod3Amount,
        P::Mod4Source,
        P::Mod4Target,
        P::Mod4Amount,
        P::Mod5Source,
        P::Mod5Target,
        P::Mod5Amount,
        P::Mod6Source,
        P::Mod6Target,
        P::Mod6Amount,
        P::Mod7Source,
        P::Mod7Target,
        P::Mod7Amount,
        P::Mod8Source,
        P::Mod8Target,
        P::Mod8Amount,
        P::Mod9Source,
        P::Mod9Target,
        P::Mod9Amount,
        P::Mod10Source,
        P::Mod10Target,
        P::Mod10Amount,
        P::Mod11Source,
        P::Mod11Target,
        P::Mod11Amount,
        P::Mod12Source,
        P::Mod12Target,
        P::Mod12Amount,
        P::Mod13Source,
        P::Mod13Target,
        P::Mod13Amount,
        P::Mod14Source,
        P::Mod14Target,
        P::Mod14Amount,
        P::Mod15Source,
        P::Mod15Target,
        P::Mod15Amount,
        P::Mod16Source,
        P::Mod16Target,
        P::Mod16Amount,
        P::Lfo1Active,
        P::Lfo2Active,
        P::Lfo3Active,
        P::Lfo4Active,
        P::Lfo5Active,
        P::Lfo6Active,
        P::Lfo7Active,
        P::Lfo8Active,
        P::PitchBendRange,
        P::ModWheel,
        P::XySourceX,
        P::XySourceY,
    ];
    let infos = state.params().param_infos();
    for param in parameters {
        let Some(info) = infos.iter().find(|info| info.id == u32::from(param)) else {
            continue;
        };
        let normalized = info.range.normalize(info.default_plain).clamp(0.0, 1.0);
        crate::editor::begin_edit(state, param);
        state.set_param(param, normalized);
        crate::editor::end_edit(state, param);
    }
    state
        .params()
        .pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    state
        .params()
        .osc2_pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    state
        .params()
        .osc3_pan_shape_curve_state
        .replace(PanShapeCurveData::default());
    state.params().xy_source_x_route_mask.store(0);
    state.params().xy_source_y_route_mask.store(0);
    for curve in [
        &state.params().osc1_wave_curve_state,
        &state.params().osc2_wave_curve_state,
        &state.params().osc3_wave_curve_state,
        &state.params().lfo1_curve_state,
        &state.params().lfo2_curve_state,
        &state.params().lfo3_curve_state,
        &state.params().lfo4_curve_state,
        &state.params().lfo5_curve_state,
        &state.params().lfo6_curve_state,
        &state.params().lfo7_curve_state,
        &state.params().lfo8_curve_state,
    ] {
        curve.replace(crate::wave_curve::WaveCurveData::default());
    }
}

#[cfg(test)]
mod tests {
    use super::reset_editor_state;
    use crate::KurvEditorState;

    #[test]
    fn factory_reset_clears_persistent_editor_choices() {
        let mut editor = KurvEditorState {
            collapsed_group_ids: vec![7],
            group_accents: vec![Default::default()],
            collapsed_modulators: 3,
            persistent_modulation_cables: true,
            ..KurvEditorState::default()
        };
        editor.set_group_name(7, "Bass");

        reset_editor_state(&mut editor);

        assert!(editor.collapsed_group_ids.is_empty());
        assert!(editor.group_accents.is_empty());
        assert_eq!(editor.group_name(7), None);
        assert_eq!(editor.collapsed_modulators, 0);
        assert!(!editor.persistent_modulation_cables);
    }
}
