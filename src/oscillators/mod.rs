mod resynth;
mod va;

#[cfg(test)]
pub(crate) use resynth::analyze_wav;
pub(crate) use resynth::{
    GrainDirection, ImportError as ResynthImportError, MAX_RESYNTH_DECODED_FRAMES,
    MAX_RESYNTH_SOURCE_BYTES, MAX_RESYNTH_SOURCE_NAME_BYTES, PitchMode, RESYNTH_ALGORITHM_COUNT,
    ResynthAlgorithm, ResynthAnalysisModel, ResynthControls, ResynthQuality, ResynthRtArtifact,
    ResynthSourceMaster, ResynthVisualModel, ScaleId, TargetSet, analyze_wav_with_cancel,
    analyze_wav_with_root_override, analyze_wav_with_root_override_and_visuals_with_cancel,
    compile_rt_artifact_with_cancel, compile_source_audition,
};

pub(crate) use resynth::decode::{AUDIO_IMPORT_EXTENSIONS, is_supported_audio_import_name};

pub(crate) use resynth::visual::{
    AlgorithmVisualCache, SampleLoopVisualMetadata, SourceWaveBin,
    analyze_sounding_artifact_visuals, analyze_sounding_artifact_visuals_with_cancel,
};

pub(crate) use resynth::artifact::{
    GRAIN_MAX_SOURCE_FRAMES, GRAIN_TELEMETRY, GrainSchedulerState, GrainSourceArtifact,
    LEGACY_RICH_FRAME_COUNT, LEGACY_RICH_FRAME_SAMPLES, LEGACY_RICH_ZONE_SAMPLES,
    ProductionResynthArtifact, RICH_FRAME_COUNT, RICH_FRAME_SAMPLES, RICH_ZONE_COUNT,
    RICH_ZONE_SAMPLES, RichVocoderArtifact, RichVocoderFrame, RichVocoderState, RichZoneArtifact,
    SAMPLE_MAX_FRAMES, SampleLoopArtifact, SourceAuditionArtifact, SourceAuditionState,
    VOCODER_ENVELOPE_BINS, VOCODER_MAX_FRAMES,
};

pub(crate) use va::{
    Antialiasing, ImportedVaTable, MAX_VA_TABLE_FRAMES, MAX_WAVETABLE_FILE_BYTES, PhaseWarpMode,
    VA_KEYFRAME_EPSILON, VaOscillator, VaTableData, VaTableRt, VaTableState,
    accumulate_custom4_block, accumulate_custom4_block_constant, accumulate_custom8_block,
    accumulate_custom8_block_constant, accumulate_saw4_block, accumulate_saw4_block_constant,
    accumulate_saw4_block_dynamic_gains, accumulate_saw4_block_static_gains, accumulate_saw8_block,
    accumulate_saw8_block_constant, accumulate_saw8_block_dynamic_gains,
    accumulate_saw8_block_static_gains, accumulate_saw8_block_static_gains_narrow_spline,
    accumulate_shape4_block_constant, accumulate_shape4_block_constant_warped,
    accumulate_shape4_block_dynamic, accumulate_shape4_block_morphing,
    accumulate_shape4_block_steps, accumulate_shape8_block_constant,
    accumulate_shape8_block_constant_warped, accumulate_shape8_block_dynamic,
    accumulate_shape8_block_morphing, accumulate_shape8_block_steps, calibrate_spline_backends,
    encode_surge_wt, generate_custom4, generate_custom8, generate_pulse4, generate_pulse8,
    generate_saw4, generate_saw8, generate_shape4, generate_shape4_pair,
    generate_shape4_pair_warped, generate_shape4_warped, generate_shape8, generate_shape8_pair,
    generate_shape8_pair_warped, generate_shape8_warped, generate_sine4, generate_sine8,
    generate_triangle4, generate_triangle8, is_narrow_spline_ramp, nearest_frame_index,
    parse_surge_wt, position_for_frame, sample_custom_shape_with_antialiasing_warped,
    shape_morph_gain,
};
