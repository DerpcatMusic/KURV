use truce::prelude::*;
use truce_core::midi::{norm_7bit, norm_pitch_bend, per_note_bend_semitones};

mod core;
mod diagnostics;
mod editor;
mod editor_controls;
mod editor_filter;
mod editor_generator;
mod editor_history;
mod editor_lfo;
mod editor_modulation;
mod editor_oscillator;
mod editor_presets;
mod editor_shell;
mod editor_theme;
mod editor_unison;
mod editor_widgets;
mod filters;
pub mod generators;
mod modulation_target;
mod modulators;
mod oscillators;
mod pan_curve;
mod params;
mod performance;
mod runtime;
mod shell;
mod voices;
mod wave_curve;

use runtime::{configuration::*, events::*, metering::*, render::*, transitions::*};

use core::oversampling::{self, DEFAULT_FACTOR, StereoOversampler};
use modulators::lfo::envelope::EnvelopeConfig;
use modulators::lfo::{
    self, HOST_LFO_COUNT, LFO_COUNT, LfoBank, LfoConfig, LfoMode, LfoRateMode, ROUTE_COUNT,
    RouteConfig,
};
use modulators::routing::{
    EXTRA_MODULATION_ROUTE_COUNT, ExtraModulationRoute, ExtraModulationRouteSnapshot,
    FilterControl, GroupControl, HOST_AUTOMATION_SLOT_COUNT, HOST_MODULATION_ROUTE_COUNT,
    HostAutomationTargetSnapshot, MODULATION_ROUTE_COUNT, ModulationRouteTarget,
    ModulationRouteTargetSnapshot, OscillatorControl, ResolvedRouteSource,
};
use modulators::state::{LEGACY_MODULATION_SOURCES, SourceKind};
use oscillators::{Antialiasing, PhaseWarpMode, VaTableRt};
use pan_curve::PanShapeSegmentsRt;
pub(crate) use params::{HOST_AUTOMATION_PARAMS, P};
pub use params::{KurvEditorState, KurvParams, KurvParamsParamId};
pub use shell::Kurv;
#[cfg(test)]
use voices::VaVoice;
use voices::{
    BLOCK_INTERNAL_SAMPLES, EnvelopeSettings, FACTOR3_BLOCK_INTERNAL_SAMPLES, InternalRtPool,
    LEGACY_OSCILLATOR_COUNT, MAX_JOB_SAMPLES, OscillatorDspConfig, OscillatorMask,
    OscillatorSettings, PanShapeSettings, PolySynth, StructuralOscillatorFrameControl, SwarmMode,
    UnisonSettings, VoiceSettings, fast_exp2,
};
use wave_curve::WaveCurveRt;

const CONTROL_BLOCK: usize = 1_024;
const FILTER_SMOOTH_SECONDS: f32 = 0.003;
const MAX_FILTER_MODULATION_STRIDE: u8 = 64;

#[derive(Clone, Copy)]
struct UnisonPitchControlBlock {
    detune_cents: [f32; CONTROL_BLOCK],
    detune_amount: [f32; CONTROL_BLOCK],
    harmonic_align: [f32; CONTROL_BLOCK],
    curve: [f32; CONTROL_BLOCK],
    phase_random: [f32; CONTROL_BLOCK],
    jitter_amount: [f32; CONTROL_BLOCK],
    jitter_rate: [f32; CONTROL_BLOCK],
    stereo: [f32; CONTROL_BLOCK],
    stereo_x: [f32; CONTROL_BLOCK],
    stereo_y: [f32; CONTROL_BLOCK],
    weight: [f32; CONTROL_BLOCK],
    pan_center: [f32; CONTROL_BLOCK],
    pan_left: [f32; CONTROL_BLOCK],
    pan_right: [f32; CONTROL_BLOCK],
    pan_center_x: [f32; CONTROL_BLOCK],
}

impl Default for UnisonPitchControlBlock {
    fn default() -> Self {
        Self {
            detune_cents: [0.0; CONTROL_BLOCK],
            detune_amount: [0.0; CONTROL_BLOCK],
            harmonic_align: [0.0; CONTROL_BLOCK],
            curve: [0.0; CONTROL_BLOCK],
            phase_random: [0.0; CONTROL_BLOCK],
            jitter_amount: [0.0; CONTROL_BLOCK],
            jitter_rate: [0.0; CONTROL_BLOCK],
            stereo: [0.0; CONTROL_BLOCK],
            stereo_x: [0.0; CONTROL_BLOCK],
            stereo_y: [0.0; CONTROL_BLOCK],
            weight: [0.0; CONTROL_BLOCK],
            pan_center: [0.0; CONTROL_BLOCK],
            pan_left: [0.0; CONTROL_BLOCK],
            pan_right: [0.0; CONTROL_BLOCK],
            pan_center_x: [0.0; CONTROL_BLOCK],
        }
    }
}

impl UnisonPitchControlBlock {
    fn is_static(&self, start: usize, len: usize) -> bool {
        let end = start + len;
        slice_is_static(&self.detune_cents[start..end])
            && slice_is_static(&self.detune_amount[start..end])
            && slice_is_static(&self.harmonic_align[start..end])
            && slice_is_static(&self.curve[start..end])
            && slice_is_static(&self.phase_random[start..end])
            && slice_is_static(&self.jitter_amount[start..end])
            && slice_is_static(&self.jitter_rate[start..end])
            && slice_is_static(&self.stereo[start..end])
            && slice_is_static(&self.stereo_x[start..end])
            && slice_is_static(&self.stereo_y[start..end])
            && slice_is_static(&self.weight[start..end])
            && slice_is_static(&self.pan_center[start..end])
            && slice_is_static(&self.pan_left[start..end])
            && slice_is_static(&self.pan_right[start..end])
            && slice_is_static(&self.pan_center_x[start..end])
    }
}

struct ControlBlock {
    shape: [f32; CONTROL_BLOCK],
    pulse_width: [f32; CONTROL_BLOCK],
    osc1_warp_amount: [f32; CONTROL_BLOCK],
    osc1_custom_shape: [f32; CONTROL_BLOCK],
    osc1_cents: [f32; CONTROL_BLOCK],
    osc1_curve_fade: [f32; CONTROL_BLOCK],
    osc1_level: [f32; CONTROL_BLOCK],
    osc1_pan: [f32; CONTROL_BLOCK],
    osc2_shape: [f32; CONTROL_BLOCK],
    osc2_pulse_width: [f32; CONTROL_BLOCK],
    osc2_warp_amount: [f32; CONTROL_BLOCK],
    osc2_custom_shape: [f32; CONTROL_BLOCK],
    osc2_cents: [f32; CONTROL_BLOCK],
    osc2_curve_fade: [f32; CONTROL_BLOCK],
    osc2_level: [f32; CONTROL_BLOCK],
    osc2_pan: [f32; CONTROL_BLOCK],
    osc3_shape: [f32; CONTROL_BLOCK],
    osc3_pulse_width: [f32; CONTROL_BLOCK],
    osc3_warp_amount: [f32; CONTROL_BLOCK],
    osc3_custom_shape: [f32; CONTROL_BLOCK],
    osc3_cents: [f32; CONTROL_BLOCK],
    osc3_curve_fade: [f32; CONTROL_BLOCK],
    osc3_level: [f32; CONTROL_BLOCK],
    osc3_pan: [f32; CONTROL_BLOCK],
    velocity: [f32; CONTROL_BLOCK],
    pressure: [f32; CONTROL_BLOCK],
    timbre: [f32; CONTROL_BLOCK],
    attack: [f32; CONTROL_BLOCK],
    decay: [f32; CONTROL_BLOCK],
    sustain: [f32; CONTROL_BLOCK],
    release: [f32; CONTROL_BLOCK],
    attack_curve: [f32; CONTROL_BLOCK],
    decay_curve: [f32; CONTROL_BLOCK],
    release_curve: [f32; CONTROL_BLOCK],
    attack_curve_time: [f32; CONTROL_BLOCK],
    decay_curve_time: [f32; CONTROL_BLOCK],
    release_curve_time: [f32; CONTROL_BLOCK],
    glide_time: [f32; CONTROL_BLOCK],
    pitch_bend: [f32; CONTROL_BLOCK],
    lfo_rate: [[f32; CONTROL_BLOCK]; HOST_LFO_COUNT],
    lfo_phase: [[f32; CONTROL_BLOCK]; HOST_LFO_COUNT],
    output_db: [f32; CONTROL_BLOCK],
    unison_pitch: [UnisonPitchControlBlock; LEGACY_OSCILLATOR_COUNT],
    modulation_amounts: [[f32; CONTROL_BLOCK]; ROUTE_COUNT],
    cached_len: usize,
    cached_oscillator_mask: OscillatorMask,
    cached_static: bool,
    cached_static_except_shape: bool,
}

#[derive(Clone, Copy)]
struct ActiveRoute {
    amount_index: usize,
    source: ResolvedRouteSource,
    descriptor: Option<modulation_target::TargetDescriptor>,
}

impl Default for ActiveRoute {
    fn default() -> Self {
        Self {
            amount_index: 0,
            source: ResolvedRouteSource::Rack(0),
            descriptor: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveModularRoute {
    host_amount_index: Option<u8>,
    overflow_amount_index: Option<u8>,
    amount: f32,
    source: ResolvedRouteSource,
    target: Option<ResolvedModularTarget>,
}

#[derive(Clone, Copy)]
enum ResolvedModularTarget {
    Oscillator {
        slot: u8,
        control: OscillatorControl,
    },
    Group {
        index: u8,
        control: GroupControl,
    },
    Filter {
        slot: u8,
        control: FilterControl,
    },
}

impl Default for ActiveModularRoute {
    fn default() -> Self {
        Self {
            host_amount_index: None,
            overflow_amount_index: None,
            amount: 0.0,
            source: ResolvedRouteSource::Rack(0),
            target: None,
        }
    }
}

#[derive(Clone, Copy)]
struct RouteAmountRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    initialized: bool,
}

impl Default for RouteAmountRamp {
    fn default() -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            step: 0.0,
            remaining: 0,
            initialized: false,
        }
    }
}

impl RouteAmountRamp {
    fn retarget(&mut self, target: f32, sample_rate: f32) {
        let target = target.clamp(-1.0, 1.0);
        if !self.initialized {
            self.current = target;
            self.target = target;
            self.initialized = true;
            return;
        }
        if self.target.to_bits() == target.to_bits() {
            return;
        }
        self.target = target;
        self.remaining = (sample_rate.max(1.0) * 0.005).round().max(1.0) as u32;
        self.step = (self.target - self.current) / self.remaining as f32;
    }

    #[inline(always)]
    fn next(&mut self) -> f32 {
        if self.remaining == 0 {
            return self.current;
        }
        self.current += self.step;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.current = self.target;
        }
        self.current
    }

    fn finish(&mut self) {
        self.current = self.target;
        self.step = 0.0;
        self.remaining = 0;
    }

    fn may_be_nonzero(self) -> bool {
        self.current.abs() > f32::EPSILON || self.target.abs() > f32::EPSILON
    }
}

struct ActiveRoutes {
    entries: [ActiveRoute; ROUTE_COUNT],
    len: usize,
    modular_entries: [ActiveModularRoute; MODULATION_ROUTE_COUNT],
    modular_len: usize,
    modular_group_mask: u8,
    source_mask: u64,
    mod_wheel_active: bool,
    unison_layout_mask: OscillatorMask,
    oscillator_mask: OscillatorMask,
    oscillator_shape_mask: OscillatorMask,
    unison_frame_mask: OscillatorMask,
    global_mask: u16,
}

const GLOBAL_OUTPUT_MASK: u16 = 1 << 0;
const GLOBAL_ENVELOPE_MASK: u16 = 1 << 1;
const GLOBAL_VELOCITY_MASK: u16 = 1 << 2;
const GLOBAL_PRESSURE_MASK: u16 = 1 << 3;
const GLOBAL_TIMBRE_MASK: u16 = 1 << 4;
const GLOBAL_GLIDE_MASK: u16 = 1 << 5;

impl Default for ActiveRoutes {
    fn default() -> Self {
        Self {
            entries: [ActiveRoute::default(); ROUTE_COUNT],
            len: 0,
            modular_entries: [ActiveModularRoute::default(); MODULATION_ROUTE_COUNT],
            modular_len: 0,
            modular_group_mask: 0,
            source_mask: 0,
            mod_wheel_active: false,
            unison_layout_mask: 0,
            oscillator_mask: 0,
            oscillator_shape_mask: 0,
            unison_frame_mask: 0,
            global_mask: 0,
        }
    }
}

impl ActiveRoutes {
    fn as_slice(&self) -> &[ActiveRoute] {
        &self.entries[..self.len]
    }

    fn modular_slice(&self) -> &[ActiveModularRoute] {
        &self.modular_entries[..self.modular_len]
    }

    fn include_source(&mut self, source: ResolvedRouteSource) {
        match source {
            ResolvedRouteSource::Rack(index) => self.source_mask |= 1_u64 << index,
            ResolvedRouteSource::ModWheel => self.mod_wheel_active = true,
        }
    }

    fn active_filter_modulation(
        &self,
        controls: &ControlBlock,
        len: usize,
        overflow_ramps: &[RouteAmountRamp; EXTRA_MODULATION_ROUTE_COUNT],
    ) -> (u32, u64, bool) {
        let mut filter_mask = 0_u32;
        let mut source_mask = 0_u64;
        let mut mod_wheel = false;
        for route in self.modular_slice() {
            let Some(ResolvedModularTarget::Filter { slot, .. }) = route.target else {
                continue;
            };
            let active = route.host_amount_index.map_or_else(
                || {
                    route
                        .overflow_amount_index
                        .is_some_and(|index| overflow_ramps[usize::from(index)].may_be_nonzero())
                },
                |index| {
                    controls.modulation_amounts[usize::from(index)][..len]
                        .iter()
                        .any(|amount| amount.abs() > f32::EPSILON)
                },
            );
            if !active {
                continue;
            }
            filter_mask |= 1 << slot;
            match route.source {
                ResolvedRouteSource::Rack(index) => source_mask |= 1_u64 << index,
                ResolvedRouteSource::ModWheel => mod_wheel = true,
            }
        }
        (filter_mask, source_mask, mod_wheel)
    }
}

#[derive(Clone, Copy, Default)]
struct StructuralOscillatorDelta {
    pitch_semitones: f32,
    shape: f32,
    pulse_width: f32,
    phase_position: f32,
    warp: f32,
    level: f32,
    pan: f32,
    unison_jitter: f32,
    unison_rate: f32,
}

#[derive(Clone, Copy, Default)]
struct StructuralGroupDelta {
    gain: f32,
    pan: f32,
}

#[derive(Clone, Copy, Default)]
struct StructuralFilterDelta {
    cutoff_octaves: f32,
    resonance_octaves: f32,
}

#[derive(Clone, Copy)]
struct StructuralModulationFrame {
    oscillator_mask: OscillatorMask,
    oscillators: [StructuralOscillatorDelta; generators::MAX_OSCILLATORS],
    group_mask: u8,
    groups: [StructuralGroupDelta; generators::MAX_OUTPUT_PAIRS],
    filter_mask: u32,
    filters: [StructuralFilterDelta; generators::MAX_FILTERS],
}

impl Default for StructuralModulationFrame {
    fn default() -> Self {
        Self {
            oscillator_mask: 0,
            oscillators: [StructuralOscillatorDelta::default(); generators::MAX_OSCILLATORS],
            group_mask: 0,
            groups: [StructuralGroupDelta::default(); generators::MAX_OUTPUT_PAIRS],
            filter_mask: 0,
            filters: [StructuralFilterDelta::default(); generators::MAX_FILTERS],
        }
    }
}

fn block_morph_modulation(routes: &ActiveRoutes) -> bool {
    routes.modular_len == 0
        && routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.unison_frame_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.oscillator_mask == routes.oscillator_shape_mask
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Shape,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

fn block_pitch_modulation(routes: &ActiveRoutes) -> bool {
    routes.modular_len == 0
        && routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.as_slice().iter().any(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Pitch,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::DetuneAmount
                            | modulation_target::UnisonTarget::DetuneRange
                            | modulation_target::UnisonTarget::HarmonicAlign
                            | modulation_target::UnisonTarget::Curve
                            | modulation_target::UnisonTarget::Stereo
                            | modulation_target::UnisonTarget::StereoX
                            | modulation_target::UnisonTarget::StereoY
                            | modulation_target::UnisonTarget::Weight
                            | modulation_target::UnisonTarget::PanCenter
                            | modulation_target::UnisonTarget::PanLeft
                            | modulation_target::UnisonTarget::PanRight
                            | modulation_target::UnisonTarget::PanCenterX,
                        ..
                    },
                    ..
                })
            )
        })
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Pitch,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::DetuneAmount
                            | modulation_target::UnisonTarget::DetuneRange
                            | modulation_target::UnisonTarget::HarmonicAlign
                            | modulation_target::UnisonTarget::Curve
                            | modulation_target::UnisonTarget::Stereo
                            | modulation_target::UnisonTarget::StereoX
                            | modulation_target::UnisonTarget::StereoY
                            | modulation_target::UnisonTarget::Weight
                            | modulation_target::UnisonTarget::PanCenter
                            | modulation_target::UnisonTarget::PanLeft
                            | modulation_target::UnisonTarget::PanRight
                            | modulation_target::UnisonTarget::PanCenterX,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

fn block_parameter_modulation(routes: &ActiveRoutes) -> bool {
    routes.modular_len == 0
        && routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.unison_frame_mask == 0
        && routes.global_mask
            & !(GLOBAL_OUTPUT_MASK
                | GLOBAL_VELOCITY_MASK
                | GLOBAL_PRESSURE_MASK
                | GLOBAL_TIMBRE_MASK
                | GLOBAL_ENVELOPE_MASK)
            == 0
        && routes.as_slice().iter().any(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator { .. },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Velocity
                            | modulation_target::GlobalTarget::Pressure
                            | modulation_target::GlobalTarget::Timbre
                            | modulation_target::GlobalTarget::Attack
                            | modulation_target::GlobalTarget::Decay
                            | modulation_target::GlobalTarget::Sustain
                            | modulation_target::GlobalTarget::Release
                            | modulation_target::GlobalTarget::AttackCurve
                            | modulation_target::GlobalTarget::DecayCurve
                            | modulation_target::GlobalTarget::ReleaseCurve
                            | modulation_target::GlobalTarget::AttackCurveTime
                            | modulation_target::GlobalTarget::DecayCurveTime
                            | modulation_target::GlobalTarget::ReleaseCurveTime,
                    ),
                    ..
                })
            )
        })
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator { .. },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output
                            | modulation_target::GlobalTarget::Velocity
                            | modulation_target::GlobalTarget::Pressure
                            | modulation_target::GlobalTarget::Timbre
                            | modulation_target::GlobalTarget::Attack
                            | modulation_target::GlobalTarget::Decay
                            | modulation_target::GlobalTarget::Sustain
                            | modulation_target::GlobalTarget::Release
                            | modulation_target::GlobalTarget::AttackCurve
                            | modulation_target::GlobalTarget::DecayCurve
                            | modulation_target::GlobalTarget::ReleaseCurve
                            | modulation_target::GlobalTarget::AttackCurveTime
                            | modulation_target::GlobalTarget::DecayCurveTime
                            | modulation_target::GlobalTarget::ReleaseCurveTime,
                    ),
                    ..
                })
            )
        })
}

fn block_motion_modulation(routes: &ActiveRoutes) -> bool {
    routes.modular_len == 0
        && routes.len != 0
        && routes.unison_layout_mask != 0
        && routes.unison_frame_mask == 0
        && routes.oscillator_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::PhaseRandom
                            | modulation_target::UnisonTarget::JitterAmount
                            | modulation_target::UnisonTarget::JitterRate,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

impl Default for ControlBlock {
    fn default() -> Self {
        Self {
            shape: [0.0; CONTROL_BLOCK],
            pulse_width: [0.0; CONTROL_BLOCK],
            osc1_warp_amount: [0.0; CONTROL_BLOCK],
            osc1_custom_shape: [0.0; CONTROL_BLOCK],
            osc1_cents: [0.0; CONTROL_BLOCK],
            osc1_curve_fade: [1.0; CONTROL_BLOCK],
            osc1_level: [0.0; CONTROL_BLOCK],
            osc1_pan: [0.0; CONTROL_BLOCK],
            osc2_shape: [0.0; CONTROL_BLOCK],
            osc2_pulse_width: [0.0; CONTROL_BLOCK],
            osc2_warp_amount: [0.0; CONTROL_BLOCK],
            osc2_custom_shape: [0.0; CONTROL_BLOCK],
            osc2_cents: [0.0; CONTROL_BLOCK],
            osc2_curve_fade: [1.0; CONTROL_BLOCK],
            osc2_level: [0.0; CONTROL_BLOCK],
            osc2_pan: [0.0; CONTROL_BLOCK],
            osc3_shape: [0.0; CONTROL_BLOCK],
            osc3_pulse_width: [0.0; CONTROL_BLOCK],
            osc3_warp_amount: [0.0; CONTROL_BLOCK],
            osc3_custom_shape: [0.0; CONTROL_BLOCK],
            osc3_cents: [0.0; CONTROL_BLOCK],
            osc3_curve_fade: [1.0; CONTROL_BLOCK],
            osc3_level: [0.0; CONTROL_BLOCK],
            osc3_pan: [0.0; CONTROL_BLOCK],
            velocity: [0.0; CONTROL_BLOCK],
            pressure: [0.0; CONTROL_BLOCK],
            timbre: [0.0; CONTROL_BLOCK],
            attack: [0.0; CONTROL_BLOCK],
            decay: [0.0; CONTROL_BLOCK],
            sustain: [0.0; CONTROL_BLOCK],
            release: [0.0; CONTROL_BLOCK],
            attack_curve: [0.0; CONTROL_BLOCK],
            decay_curve: [0.0; CONTROL_BLOCK],
            release_curve: [0.0; CONTROL_BLOCK],
            attack_curve_time: [0.0; CONTROL_BLOCK],
            decay_curve_time: [0.0; CONTROL_BLOCK],
            release_curve_time: [0.0; CONTROL_BLOCK],
            glide_time: [0.0; CONTROL_BLOCK],
            pitch_bend: [0.0; CONTROL_BLOCK],
            lfo_rate: [[0.0; CONTROL_BLOCK]; HOST_LFO_COUNT],
            lfo_phase: [[0.0; CONTROL_BLOCK]; HOST_LFO_COUNT],
            output_db: [0.0; CONTROL_BLOCK],
            unison_pitch: [UnisonPitchControlBlock::default(); LEGACY_OSCILLATOR_COUNT],
            modulation_amounts: [[0.0; CONTROL_BLOCK]; ROUTE_COUNT],
            cached_len: 0,
            cached_oscillator_mask: 0,
            cached_static: false,
            cached_static_except_shape: false,
        }
    }
}

impl ControlBlock {
    fn read(
        &mut self,
        params: &KurvParams,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
        active_routes: &ActiveRoutes,
        lfo_mask: u64,
    ) -> Option<f32> {
        params.shape.read_into(&mut self.shape[..len]);
        params.pulse_width.read_into(&mut self.pulse_width[..len]);
        params
            .osc1_warp_amount
            .read_into(&mut self.osc1_warp_amount[..len]);
        params
            .osc1_custom_shape
            .read_into(&mut self.osc1_custom_shape[..len]);
        params.osc1_cents.read_into(&mut self.osc1_cents[..len]);
        params.osc1_level.read_into(&mut self.osc1_level[..len]);
        params.osc1_pan.read_into(&mut self.osc1_pan[..len]);
        if oscillator_enabled[1] {
            params.osc2_shape.read_into(&mut self.osc2_shape[..len]);
            params
                .osc2_pulse_width
                .read_into(&mut self.osc2_pulse_width[..len]);
            params
                .osc2_warp_amount
                .read_into(&mut self.osc2_warp_amount[..len]);
            params
                .osc2_custom_shape
                .read_into(&mut self.osc2_custom_shape[..len]);
            params.osc2_cents.read_into(&mut self.osc2_cents[..len]);
            params.osc2_level.read_into(&mut self.osc2_level[..len]);
            params.osc2_pan.read_into(&mut self.osc2_pan[..len]);
        }
        if oscillator_enabled[2] {
            params.osc3_shape.read_into(&mut self.osc3_shape[..len]);
            params
                .osc3_pulse_width
                .read_into(&mut self.osc3_pulse_width[..len]);
            params
                .osc3_warp_amount
                .read_into(&mut self.osc3_warp_amount[..len]);
            params
                .osc3_custom_shape
                .read_into(&mut self.osc3_custom_shape[..len]);
            params.osc3_cents.read_into(&mut self.osc3_cents[..len]);
            params.osc3_level.read_into(&mut self.osc3_level[..len]);
            params.osc3_pan.read_into(&mut self.osc3_pan[..len]);
        }
        params.velocity_amount.read_into(&mut self.velocity[..len]);
        params.pressure_amount.read_into(&mut self.pressure[..len]);
        params.timbre_amount.read_into(&mut self.timbre[..len]);
        params.attack.read_into(&mut self.attack[..len]);
        params.decay.read_into(&mut self.decay[..len]);
        params.sustain.read_into(&mut self.sustain[..len]);
        params.release.read_into(&mut self.release[..len]);
        params.attack_curve.read_into(&mut self.attack_curve[..len]);
        params.decay_curve.read_into(&mut self.decay_curve[..len]);
        params
            .release_curve
            .read_into(&mut self.release_curve[..len]);
        params
            .attack_curve_time
            .read_into(&mut self.attack_curve_time[..len]);
        params
            .decay_curve_time
            .read_into(&mut self.decay_curve_time[..len]);
        params
            .release_curve_time
            .read_into(&mut self.release_curve_time[..len]);
        params.glide_time.read_into(&mut self.glide_time[..len]);
        params.pitch_bend.read_into(&mut self.pitch_bend[..len]);
        params.output_db.read_into(&mut self.output_db[..len]);
        let unison_pitch_params = [
            (
                &params.unison_detune,
                &params.unison_detune_amount,
                &params.unison_harmonic_align,
                &params.unison_curve,
                &params.phase_random,
                &params.unison_swarm,
                &params.unison_swarm_rate,
                &params.unison_stereo,
                &params.stereo_x,
                &params.stereo_alternate,
                &params.unison_weight,
                &params.pan_shape_center,
                &params.pan_shape_left,
                &params.pan_shape_right,
                &params.pan_shape_center_x,
            ),
            (
                &params.osc2_unison_detune,
                &params.osc2_unison_detune_amount,
                &params.osc2_unison_harmonic_align,
                &params.osc2_unison_curve,
                &params.osc2_phase_random,
                &params.osc2_unison_jitter,
                &params.osc2_unison_jitter_rate,
                &params.osc2_unison_stereo,
                &params.osc2_stereo_x,
                &params.osc2_stereo_alternate,
                &params.osc2_unison_weight,
                &params.osc2_pan_shape_center,
                &params.osc2_pan_shape_left,
                &params.osc2_pan_shape_right,
                &params.osc2_pan_shape_center_x,
            ),
            (
                &params.osc3_unison_detune,
                &params.osc3_unison_detune_amount,
                &params.osc3_unison_harmonic_align,
                &params.osc3_unison_curve,
                &params.osc3_phase_random,
                &params.osc3_unison_jitter,
                &params.osc3_unison_jitter_rate,
                &params.osc3_unison_stereo,
                &params.osc3_stereo_x,
                &params.osc3_stereo_alternate,
                &params.osc3_unison_weight,
                &params.osc3_pan_shape_center,
                &params.osc3_pan_shape_left,
                &params.osc3_pan_shape_right,
                &params.osc3_pan_shape_center_x,
            ),
        ];
        for (
            control,
            (
                detune,
                amount,
                align,
                curve,
                phase_random,
                jitter_amount,
                jitter_rate,
                stereo,
                stereo_x,
                stereo_y,
                weight,
                pan_center,
                pan_left,
                pan_right,
                pan_center_x,
            ),
        ) in self.unison_pitch.iter_mut().zip(unison_pitch_params)
        {
            detune.read_into(&mut control.detune_cents[..len]);
            amount.read_into(&mut control.detune_amount[..len]);
            align.read_into(&mut control.harmonic_align[..len]);
            curve.read_into(&mut control.curve[..len]);
            phase_random.read_into(&mut control.phase_random[..len]);
            jitter_amount.read_into(&mut control.jitter_amount[..len]);
            jitter_rate.read_into(&mut control.jitter_rate[..len]);
            stereo.read_into(&mut control.stereo[..len]);
            stereo_x.read_into(&mut control.stereo_x[..len]);
            stereo_y.read_into(&mut control.stereo_y[..len]);
            weight.read_into(&mut control.weight[..len]);
            pan_center.read_into(&mut control.pan_center[..len]);
            pan_left.read_into(&mut control.pan_left[..len]);
            pan_right.read_into(&mut control.pan_right[..len]);
            pan_center_x.read_into(&mut control.pan_center_x[..len]);
        }
        if lfo_mask != 0 {
            let lfo_params = [
                (&params.lfo1_rate, &params.lfo1_phase),
                (&params.lfo2_rate, &params.lfo2_phase),
                (&params.lfo3_rate, &params.lfo3_phase),
                (&params.lfo4_rate, &params.lfo4_phase),
                (&params.lfo5_rate, &params.lfo5_phase),
                (&params.lfo6_rate, &params.lfo6_phase),
                (&params.lfo7_rate, &params.lfo7_phase),
                (&params.lfo8_rate, &params.lfo8_phase),
            ];
            for (index, (rate, phase)) in lfo_params.into_iter().enumerate() {
                if lfo_mask & (1 << index) == 0 {
                    continue;
                }
                rate.read_into(&mut self.lfo_rate[index][..len]);
                phase.read_into(&mut self.lfo_phase[index][..len]);
            }
        }
        let amount_params = [
            &params.mod1_amount,
            &params.mod2_amount,
            &params.mod3_amount,
            &params.mod4_amount,
            &params.mod5_amount,
            &params.mod6_amount,
            &params.mod7_amount,
            &params.mod8_amount,
            &params.mod9_amount,
            &params.mod10_amount,
            &params.mod11_amount,
            &params.mod12_amount,
            &params.mod13_amount,
            &params.mod14_amount,
            &params.mod15_amount,
            &params.mod16_amount,
        ];
        let mut amount_mask = 0_u16;
        for route in active_routes.as_slice() {
            amount_mask |= 1 << route.amount_index;
        }
        for route in active_routes.modular_slice() {
            if let Some(index) = route.host_amount_index {
                amount_mask |= 1 << index;
            }
        }
        while amount_mask != 0 {
            let index = amount_mask.trailing_zeros() as usize;
            amount_mask &= amount_mask - 1;
            amount_params[index].read_into(&mut self.modulation_amounts[index][..len]);
        }
        let oscillator_mask = oscillator_enabled_mask(oscillator_enabled);
        self.cached_len = len;
        self.cached_oscillator_mask = oscillator_mask;
        self.cached_static = self.compute_is_static(0, len, oscillator_enabled);
        self.cached_static_except_shape =
            self.cached_static || self.compute_is_static_except_shape(0, len, oscillator_enabled);
        (self.output_db[0].to_bits() == self.output_db[len - 1].to_bits())
            .then(|| db_to_linear(self.output_db[0]))
    }

    fn active_lfo_mask(&self, routes: &ActiveRoutes, len: usize) -> u64 {
        let mut mask = routes.as_slice().iter().fold(0, |mask, route| {
            route.source.rack_index().map_or(mask, |source| {
                if self.modulation_amounts[route.amount_index][..len]
                    .iter()
                    .any(|amount| amount.abs() > f32::EPSILON)
                {
                    mask | (1_u64 << source)
                } else {
                    mask
                }
            })
        });
        for route in routes.modular_slice() {
            let active =
                route
                    .host_amount_index
                    .map_or(route.amount.abs() > f32::EPSILON, |index| {
                        self.modulation_amounts[usize::from(index)][..len]
                            .iter()
                            .any(|amount| amount.abs() > f32::EPSILON)
                    });
            if active && let Some(source) = route.source.rack_index() {
                mask |= 1_u64 << source;
            }
        }
        mask
    }

    fn lfo_control_dynamic_mask(
        &self,
        mask: u64,
        len: usize,
        configs: &[LfoConfig; LFO_COUNT],
    ) -> u8 {
        let mut dynamic = 0;
        for index in 0..HOST_LFO_COUNT {
            if mask & (1_u64 << index) == 0 || configs[index].envelope {
                continue;
            }
            let rate = &self.lfo_rate[index][..len];
            let phase = &self.lfo_phase[index][..len];
            if !slice_is_static(rate)
                || !slice_is_static(phase)
                || rate[0].to_bits() != configs[index].rate_hz.to_bits()
                || phase[0].to_bits() != configs[index].phase_offset.to_bits()
            {
                dynamic |= 1 << index;
            }
        }
        dynamic
    }

    fn unison_pitch_active_mask(
        &self,
        len: usize,
        base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    ) -> OscillatorMask {
        let mut active = 0;
        for (oscillator, (control, base)) in self.unison_pitch.iter().zip(base_unison).enumerate() {
            let static_control = control.detune_cents[..len]
                .iter()
                .all(|value| value.to_bits() == (base.detune_cents() / 100.0).to_bits())
                && control.detune_amount[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.detune_amount().to_bits())
                && control.harmonic_align[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.harmonic_align().to_bits())
                && control.curve[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.curve().to_bits())
                && control.stereo[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo().to_bits())
                && control.stereo_x[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo_x().to_bits())
                && control.stereo_y[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo_alternate().to_bits())
                && control.weight[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.level_curve().to_bits())
                && control.pan_center[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().center.to_bits())
                && control.pan_left[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().left_edge.to_bits())
                && control.pan_right[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().right_edge.to_bits())
                && control.pan_center_x[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().center_x.to_bits());
            if !static_control {
                active |= 1 << oscillator;
            }
        }
        active
    }

    fn unison_motion_active_mask(
        &self,
        len: usize,
        base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    ) -> OscillatorMask {
        let mut active = 0;
        for (oscillator, (control, base)) in self.unison_pitch.iter().zip(base_unison).enumerate() {
            let static_control = control.phase_random[..len]
                .iter()
                .all(|value| value.to_bits() == base.phase_random().to_bits())
                && control.jitter_amount[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.swarm_amount().to_bits())
                && control.jitter_rate[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.swarm_rate().to_bits());
            if !static_control {
                active |= 1 << oscillator;
            }
        }
        active
    }

    fn envelope_is_static(&self, start: usize, len: usize) -> bool {
        let end = start + len;
        [
            &self.attack[start..end],
            &self.decay[start..end],
            &self.sustain[start..end],
            &self.release[start..end],
            &self.attack_curve[start..end],
            &self.decay_curve[start..end],
            &self.release_curve[start..end],
            &self.attack_curve_time[start..end],
            &self.decay_curve_time[start..end],
            &self.release_curve_time[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
        ]
        .into_iter()
        .all(slice_is_static)
    }

    fn is_static(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
    ) -> bool {
        if self.cached_static && self.cached_range_matches(start, len, oscillator_enabled) {
            return true;
        }
        self.compute_is_static(start, len, oscillator_enabled)
    }

    fn compute_is_static(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
    ) -> bool {
        let end = start + len;
        let primary_static = [
            &self.shape[start..end],
            &self.pulse_width[start..end],
            &self.osc1_warp_amount[start..end],
            &self.osc1_custom_shape[start..end],
            &self.osc1_cents[start..end],
            &self.osc1_curve_fade[start..end],
            &self.osc1_level[start..end],
            &self.osc1_pan[start..end],
            &self.velocity[start..end],
            &self.pressure[start..end],
            &self.timbre[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
            &self.output_db[start..end],
        ]
        .into_iter()
        .all(|values| {
            let bits = values[0].to_bits();
            values[1..].iter().all(|value| value.to_bits() == bits)
        });
        primary_static
            && self.envelope_is_static(start, len)
            && self
                .unison_pitch
                .iter()
                .all(|control| control.is_static(start, len))
            && (!oscillator_enabled[1]
                || [
                    &self.osc2_shape[start..end],
                    &self.osc2_pulse_width[start..end],
                    &self.osc2_warp_amount[start..end],
                    &self.osc2_custom_shape[start..end],
                    &self.osc2_cents[start..end],
                    &self.osc2_curve_fade[start..end],
                    &self.osc2_level[start..end],
                    &self.osc2_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
            && (!oscillator_enabled[2]
                || [
                    &self.osc3_shape[start..end],
                    &self.osc3_pulse_width[start..end],
                    &self.osc3_warp_amount[start..end],
                    &self.osc3_custom_shape[start..end],
                    &self.osc3_cents[start..end],
                    &self.osc3_curve_fade[start..end],
                    &self.osc3_level[start..end],
                    &self.osc3_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
    }

    fn is_static_except_shape(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
    ) -> bool {
        if self.cached_static_except_shape
            && self.cached_range_matches(start, len, oscillator_enabled)
        {
            return true;
        }
        self.compute_is_static_except_shape(start, len, oscillator_enabled)
    }

    fn compute_is_static_except_shape(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
    ) -> bool {
        let end = start + len;
        let primary_static = [
            &self.pulse_width[start..end],
            &self.osc1_warp_amount[start..end],
            &self.osc1_custom_shape[start..end],
            &self.osc1_cents[start..end],
            &self.osc1_curve_fade[start..end],
            &self.osc1_level[start..end],
            &self.osc1_pan[start..end],
            &self.velocity[start..end],
            &self.pressure[start..end],
            &self.timbre[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
            &self.output_db[start..end],
        ]
        .into_iter()
        .all(slice_is_static);
        primary_static
            && self.envelope_is_static(start, len)
            && self
                .unison_pitch
                .iter()
                .all(|control| control.is_static(start, len))
            && (!oscillator_enabled[1]
                || [
                    &self.osc2_pulse_width[start..end],
                    &self.osc2_warp_amount[start..end],
                    &self.osc2_custom_shape[start..end],
                    &self.osc2_cents[start..end],
                    &self.osc2_curve_fade[start..end],
                    &self.osc2_level[start..end],
                    &self.osc2_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
            && (!oscillator_enabled[2]
                || [
                    &self.osc3_pulse_width[start..end],
                    &self.osc3_warp_amount[start..end],
                    &self.osc3_custom_shape[start..end],
                    &self.osc3_cents[start..end],
                    &self.osc3_curve_fade[start..end],
                    &self.osc3_level[start..end],
                    &self.osc3_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
    }

    fn cached_range_matches(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; LEGACY_OSCILLATOR_COUNT],
    ) -> bool {
        let oscillator_mask = oscillator_enabled_mask(oscillator_enabled);
        self.cached_len != 0
            && self.cached_oscillator_mask == oscillator_mask
            && start <= self.cached_len
            && len <= self.cached_len - start
    }

    fn expanded_shapes(
        &self,
        start: usize,
        host_frames: usize,
        factor: usize,
    ) -> [[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT] {
        let controls = [&self.shape, &self.osc2_shape, &self.osc3_shape];
        std::array::from_fn(|oscillator| {
            let mut output = [0.0; MAX_JOB_SAMPLES];
            for frame in 0..host_frames {
                output[frame * factor..(frame + 1) * factor]
                    .fill(controls[oscillator][start + frame]);
            }
            output
        })
    }
}

pub struct KurvDspState {
    synth: PolySynth,
    internal_pool: InternalRtPool,
    host_sample_rate: f32,
    dsp_sample_rate: f32,
    oversampler: StereoOversampler,
    group_oversamplers: Box<[StereoOversampler; generators::MAX_OUTPUT_PAIRS]>,
    decimator_tail: u8,
    mpe_bend_range: f32,
    pitch_bend_range: f32,
    glide_time_control: f32,
    pitch_bend_control: f32,
    controls: Box<ControlBlock>,
    meter_left: f32,
    meter_right: f32,
    pan_shape_segments: [(PanShapeSegmentsRt, PanShapeSegmentsRt); generators::MAX_OSCILLATORS],
    pan_shape_generations: [u32; generators::MAX_OSCILLATORS],
    base_wave_curve: WaveCurveRt,
    wave_curves: [WaveCurveTransition; LEGACY_OSCILLATOR_COUNT],
    va_tables: Box<[VaTableRt]>,
    va_table_transitions: Box<[VaTableTransition]>,
    va_table_generations: [u32; generators::MAX_OSCILLATORS],
    generator_rt_generation: u32,
    generator_materialized: bool,
    generator_oscillators: [generators::OscillatorConfig; generators::MAX_OSCILLATORS],
    effective_generator_oscillators: [generators::OscillatorConfig; generators::MAX_OSCILLATORS],
    generator_filters: [filters::FilterConfig; generators::MAX_FILTERS],
    effective_generator_filters: [filters::FilterConfig; generators::MAX_FILTERS],
    generator_filter_coefficients: [filters::FilterCoefficients; generators::MAX_FILTERS],
    generator_filter_targets: [filters::FilterCoefficients; generators::MAX_FILTERS],
    generator_filter_modulated_configs: [filters::FilterConfig; generators::MAX_FILTERS],
    generator_filter_smoothing: [u16; generators::MAX_FILTERS],
    generator_filter_smoothing_mask: u32,
    generator_filter_mask: u32,
    generator_filter_modulation_mask: u32,
    generator_filter_modulation_tick: u8,
    generator_filter_modulation_stride: u8,
    generator_filters_were_silent: bool,
    generator_module_ids: [u64; generators::MAX_OSCILLATORS],
    generator_filter_module_ids: [u64; generators::MAX_FILTERS],
    generator_groups: [generators::GeneratorRtGroup; generators::MAX_OUTPUT_PAIRS],
    generator_group_masks: [u32; generators::MAX_OUTPUT_PAIRS],
    generator_group_ids: [u64; generators::MAX_OUTPUT_PAIRS],
    generator_group_outputs: [generators::GroupOutput; generators::MAX_OUTPUT_PAIRS],
    effective_generator_group_outputs: [generators::GroupOutput; generators::MAX_OUTPUT_PAIRS],
    generator_oscillator_groups: [u8; generators::MAX_OSCILLATORS],
    generator_group_count: usize,
    generator_has_filters: bool,
    generator_active_mask: u32,
    modular_route_generation: u32,
    modular_route_targets: ModulationRouteTargetSnapshot,
    overflow_route_generation: u32,
    overflow_routes: ExtraModulationRouteSnapshot,
    overflow_route_ramps: [RouteAmountRamp; EXTRA_MODULATION_ROUTE_COUNT],
    mod_wheel_ramp: RouteAmountRamp,
    host_automation_generation: u32,
    host_automation_targets: HostAutomationTargetSnapshot,
    host_automation_slots: [u8; HOST_AUTOMATION_SLOT_COUNT],
    host_automation_len: u8,
    lfos: LfoBank,
    lfo_curve_generations: [u32; LFO_COUNT],
    lfo_modulation_block: [modulators::lfo::ModulationFrame; BLOCK_INTERNAL_SAMPLES],
    #[cfg(test)]
    block_major_enabled: bool,
    #[cfg(test)]
    block_major_chunks: usize,
    #[cfg(test)]
    internal_pool_enabled: bool,
    #[cfg(test)]
    internal_pool_coarse_jobs: usize,
    #[cfg(test)]
    internal_pool_partial_serial_jobs: usize,
}

impl Default for KurvDspState {
    fn default() -> Self {
        diagnostics::startup();
        diagnostics::lifecycle("dsp-default-enter");
        performance::initialize();
        let base_wave_curve = WaveCurveRt::default();
        let state = Self {
            synth: PolySynth::default(),
            internal_pool: InternalRtPool::new(),
            host_sample_rate: 44_100.0,
            dsp_sample_rate: 44_100.0 * f32::from(DEFAULT_FACTOR),
            oversampler: StereoOversampler::default(),
            group_oversamplers: Box::new(std::array::from_fn(|_| StereoOversampler::default())),
            decimator_tail: 0,
            mpe_bend_range: 48.0,
            pitch_bend_range: 2.0,
            glide_time_control: f32::NAN,
            pitch_bend_control: f32::NAN,
            controls: Box::new(ControlBlock::default()),
            meter_left: 0.0,
            meter_right: 0.0,
            pan_shape_segments: [(
                PanShapeSegmentsRt::identity(),
                PanShapeSegmentsRt::identity(),
            ); generators::MAX_OSCILLATORS],
            pan_shape_generations: [u32::MAX; generators::MAX_OSCILLATORS],
            base_wave_curve,
            wave_curves: [WaveCurveTransition::new(base_wave_curve); LEGACY_OSCILLATOR_COUNT],
            va_tables: (0..generators::MAX_OSCILLATORS)
                .map(|_| VaTableRt::default())
                .collect(),
            va_table_transitions: (0..LEGACY_OSCILLATOR_COUNT)
                .map(|_| VaTableTransition::default())
                .collect(),
            va_table_generations: [0; generators::MAX_OSCILLATORS],
            generator_rt_generation: u32::MAX,
            generator_materialized: false,
            generator_oscillators: std::array::from_fn(|_| {
                let mut config = generators::OscillatorConfig::default();
                config.enabled = false;
                config
            }),
            effective_generator_oscillators: std::array::from_fn(|_| {
                let mut config = generators::OscillatorConfig::default();
                config.enabled = false;
                config
            }),
            generator_filters: [filters::FilterConfig::default(); generators::MAX_FILTERS],
            effective_generator_filters: [filters::FilterConfig::default();
                generators::MAX_FILTERS],
            generator_filter_coefficients: [filters::FilterConfig::default()
                .coefficients(44_100.0 * f32::from(DEFAULT_FACTOR));
                generators::MAX_FILTERS],
            generator_filter_targets: [filters::FilterConfig::default()
                .coefficients(44_100.0 * f32::from(DEFAULT_FACTOR));
                generators::MAX_FILTERS],
            generator_filter_modulated_configs: [filters::FilterConfig::default();
                generators::MAX_FILTERS],
            generator_filter_smoothing: [0; generators::MAX_FILTERS],
            generator_filter_smoothing_mask: 0,
            generator_filter_mask: 0,
            generator_filter_modulation_mask: 0,
            generator_filter_modulation_tick: 0,
            generator_filter_modulation_stride: 1,
            generator_filters_were_silent: true,
            generator_module_ids: [0; generators::MAX_OSCILLATORS],
            generator_filter_module_ids: [0; generators::MAX_FILTERS],
            generator_groups: [generators::GeneratorRtGroup::EMPTY; generators::MAX_OUTPUT_PAIRS],
            generator_group_masks: std::array::from_fn(|index| if index == 0 { 1 } else { 0 }),
            generator_group_ids: [0; generators::MAX_OUTPUT_PAIRS],
            generator_group_outputs: [generators::GroupOutput::default();
                generators::MAX_OUTPUT_PAIRS],
            effective_generator_group_outputs: [generators::GroupOutput::default();
                generators::MAX_OUTPUT_PAIRS],
            generator_oscillator_groups: [0; generators::MAX_OSCILLATORS],
            generator_group_count: 1,
            generator_has_filters: false,
            generator_active_mask: 1,
            modular_route_generation: u32::MAX,
            modular_route_targets: [None; MODULATION_ROUTE_COUNT],
            overflow_route_generation: u32::MAX,
            overflow_routes: [ExtraModulationRoute::EMPTY; EXTRA_MODULATION_ROUTE_COUNT],
            overflow_route_ramps: [RouteAmountRamp::default(); EXTRA_MODULATION_ROUTE_COUNT],
            mod_wheel_ramp: RouteAmountRamp::default(),
            host_automation_generation: u32::MAX,
            host_automation_targets: [None; HOST_AUTOMATION_SLOT_COUNT],
            host_automation_slots: [0; HOST_AUTOMATION_SLOT_COUNT],
            host_automation_len: 0,
            lfos: LfoBank::default(),
            lfo_curve_generations: [u32::MAX; LFO_COUNT],
            lfo_modulation_block: [modulators::lfo::ModulationFrame::default();
                BLOCK_INTERNAL_SAMPLES],
            #[cfg(test)]
            block_major_enabled: true,
            #[cfg(test)]
            block_major_chunks: 0,
            #[cfg(test)]
            internal_pool_enabled: true,
            #[cfg(test)]
            internal_pool_coarse_jobs: 0,
            #[cfg(test)]
            internal_pool_partial_serial_jobs: 0,
        };
        diagnostics::lifecycle("dsp-default-return");
        state
    }
}

impl KurvDspState {
    fn reset_lfo_curve_generations(&mut self) {
        self.lfo_curve_generations.fill(u32::MAX);
    }

    fn fill_wave_curve_fades(&mut self, len: usize) {
        let step = 1.0 / (self.host_sample_rate * 0.004).max(1.0);
        for (transition, output) in self.wave_curves.iter_mut().zip([
            &mut self.controls.osc1_curve_fade,
            &mut self.controls.osc2_curve_fade,
            &mut self.controls.osc3_curve_fade,
        ]) {
            for value in &mut output[..len] {
                *value = transition.advance(step);
            }
        }
    }

    const fn block_major_enabled(&self) -> bool {
        #[cfg(test)]
        {
            self.block_major_enabled
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    const fn internal_pool_enabled(&self) -> bool {
        #[cfg(test)]
        {
            self.internal_pool_enabled
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    fn refresh_filter_coefficients(&mut self) {
        for (index, config) in self.effective_generator_filters.into_iter().enumerate() {
            let coefficients = config.coefficients(self.dsp_sample_rate);
            self.generator_filter_coefficients[index] = coefficients;
            self.generator_filter_targets[index] = coefficients;
            self.generator_filter_modulated_configs[index] = config;
            self.generator_filter_smoothing[index] = 0;
        }
        self.generator_filter_smoothing_mask = 0;
        self.generator_filters_were_silent = true;
    }

    fn retarget_filter_coefficients(&mut self, index: usize, target: filters::FilterCoefficients) {
        self.generator_filter_targets[index] = target;
        let bit = 1 << index;
        if self.generator_filter_mask & bit == 0 {
            self.generator_filter_coefficients[index] = target;
            self.generator_filter_smoothing[index] = 0;
            self.generator_filter_smoothing_mask &= !bit;
            return;
        }
        self.generator_filter_smoothing[index] = (self.dsp_sample_rate * FILTER_SMOOTH_SECONDS)
            .round()
            .clamp(1.0, f32::from(u16::MAX))
            as u16;
        self.generator_filter_smoothing_mask |= bit;
    }

    fn set_filter_modulation_mask(&mut self, mask: u32) {
        let mask = mask & self.generator_filter_mask;
        if mask == self.generator_filter_modulation_mask {
            return;
        }
        let mut deactivated = self.generator_filter_modulation_mask & !mask;
        while deactivated != 0 {
            let index = deactivated.trailing_zeros() as usize;
            deactivated &= deactivated - 1;
            self.retarget_filter_coefficients(
                index,
                self.effective_generator_filters[index].coefficients(self.dsp_sample_rate),
            );
            self.generator_filter_modulated_configs[index] =
                self.effective_generator_filters[index];
        }
        self.generator_filter_modulation_mask = mask;
        self.generator_filter_modulation_tick = 0;
    }

    fn update_filter_modulation(&mut self, modulation: Option<&StructuralModulationFrame>) {
        let mut mask = self.generator_filter_modulation_mask;
        if mask == 0 {
            return;
        }
        let snap = self.generator_filters_were_silent;
        if !snap && self.generator_filter_modulation_tick != 0 {
            self.generator_filter_modulation_tick -= 1;
            return;
        }
        let stride = self.generator_filter_modulation_stride.max(1);
        self.generator_filter_modulation_tick = stride - 1;
        while mask != 0 {
            let index = mask.trailing_zeros() as usize;
            let bit = 1 << index;
            mask &= mask - 1;
            let delta = modulation
                .filter(|frame| frame.filter_mask & bit != 0)
                .map_or(StructuralFilterDelta::default(), |frame| {
                    frame.filters[index]
                });
            let base = self.effective_generator_filters[index];
            let config = filters::FilterConfig {
                mode: base.mode,
                cutoff_hz: base.cutoff_hz * fast_exp2(delta.cutoff_octaves),
                q: base.q * fast_exp2(delta.resonance_octaves),
            };
            if !snap && config == self.generator_filter_modulated_configs[index] {
                continue;
            }
            self.generator_filter_modulated_configs[index] = config;
            let target = config.coefficients(self.dsp_sample_rate);
            self.generator_filter_targets[index] = target;
            if snap || stride == 1 {
                self.generator_filter_coefficients[index] = target;
                self.generator_filter_smoothing[index] = 0;
                self.generator_filter_smoothing_mask &= !bit;
            } else {
                self.generator_filter_smoothing[index] = u16::from(stride);
                self.generator_filter_smoothing_mask |= bit;
            }
        }
        self.generator_filters_were_silent = false;
    }

    fn advance_filter_coefficients(&mut self) {
        let mut mask = self.generator_filter_smoothing_mask;
        while mask != 0 {
            let index = mask.trailing_zeros() as usize;
            let bit = 1 << index;
            mask &= !bit;
            let remaining = self.generator_filter_smoothing[index];
            if remaining == 0 {
                self.generator_filter_smoothing_mask &= !bit;
                continue;
            }
            self.generator_filter_coefficients[index] = self.generator_filter_coefficients[index]
                .interpolate(
                    self.generator_filter_targets[index],
                    f32::from(remaining).recip(),
                );
            self.generator_filter_smoothing[index] = remaining - 1;
            if remaining == 1 {
                self.generator_filter_smoothing_mask &= !bit;
            }
        }
    }

    fn settle_filter_coefficients_for_silence(&mut self) {
        let mut mask = self.generator_filter_smoothing_mask;
        while mask != 0 {
            let index = mask.trailing_zeros() as usize;
            let bit = 1 << index;
            mask &= !bit;
            self.generator_filter_coefficients[index] = self.generator_filter_targets[index];
            self.generator_filter_smoothing[index] = 0;
        }
        self.generator_filter_smoothing_mask = 0;
        self.generator_filter_modulation_tick = 0;
        self.generator_filters_were_silent = true;
    }

    fn set_oversampling(&mut self, factor: u8, antialiasing: Antialiasing) -> bool {
        if factor == self.oversampler.factor() || self.synth.is_active() || self.decimator_tail != 0
        {
            return false;
        }
        self.oversampler.reset(factor);
        self.oversampler.set_spline_correction_immediate(matches!(
            antialiasing.for_factor(factor),
            Antialiasing::SplineOptimized
        ));
        for oversampler in &mut *self.group_oversamplers {
            oversampler.reset(factor);
            oversampler.set_spline_correction_immediate(matches!(
                antialiasing.for_factor(factor),
                Antialiasing::SplineOptimized
            ));
        }
        self.dsp_sample_rate = self.host_sample_rate * f32::from(factor);
        self.refresh_filter_coefficients();
        self.synth.set_sample_rate(self.dsp_sample_rate);
        self.lfos.set_sample_rate(self.dsp_sample_rate);
        true
    }
}

fn old_plain_value(value: &serde_json::Value) -> Option<f64> {
    let tagged = value.as_object()?;
    tagged
        .get("f32")
        .or_else(|| tagged.get("i32"))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            tagged
                .get("bool")
                .and_then(serde_json::Value::as_bool)
                .map(f64::from)
        })
}

truce::plugin! {
    logic: Kurv,
    params: KurvParams,
}

truce::enable_rt_paranoid!();

#[cfg(test)]
mod tests {
    use super::*;

    const PROCESS_TEST_FRAMES: usize = 128;

    fn render_process_test(
        events: &[Event],
        block_major_enabled: bool,
        smooth_shape: bool,
        swarm_mode: Option<SwarmMode>,
        oversampling_factor: u8,
    ) -> (Vec<(f32, f32)>, usize) {
        let params = KurvParams::default();
        params.unison_voices.set_value(64);
        params
            .oversampling
            .set_value(i64::from(oversampling_factor));
        params.phase_random.set_value(0.0);
        if let Some(mode) = swarm_mode {
            params.unison_swarm.set_value(1.0);
            params
                .unison_swarm_mode
                .set_value(i64::from(mode == SwarmMode::Jitter));
        }
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            block_major_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(
            &mut state,
            &params,
            &AudioConfig::new(48_000.0, PROCESS_TEST_FRAMES),
        );
        if smooth_shape {
            params.shape.set_value(2.5);
        }

        let mut input_events = EventList::with_capacity(events.len());
        for event in events {
            input_events.push(*event);
        }
        let mut output_events = EventList::with_capacity(0);
        let transport = TransportInfo::default();
        let mut context = ProcessContext::new(
            &transport,
            48_000.0,
            PROCESS_TEST_FRAMES,
            &mut output_events,
        );
        let mut left = vec![0.0; PROCESS_TEST_FRAMES];
        let mut right = vec![0.0; PROCESS_TEST_FRAMES];
        {
            let inputs: [&[f32]; 0] = [];
            let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
            let mut buffer =
                AudioBuffer::from_slices_checked(&inputs, &mut outputs, PROCESS_TEST_FRAMES);
            #[cfg(feature = "rt-paranoid")]
            {
                let (_, violations) = truce::rt::audit(|| {
                    let _rt = truce::rt::RtSection::enter();
                    <Kurv as PluginLogic>::process(
                        &mut state,
                        &params,
                        &mut buffer,
                        &input_events,
                        &mut context,
                    )
                });
                assert_eq!(violations, 0, "audio callback realtime violations");
            }
            #[cfg(not(feature = "rt-paranoid"))]
            let _ = <Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            );
        }
        (
            left.into_iter().zip(right).collect(),
            state.block_major_chunks,
        )
    }

    fn assert_process_paths_equal(
        events: &[Event],
        smooth_shape: bool,
        swarm_mode: Option<SwarmMode>,
        oversampling_factor: u8,
    ) -> usize {
        let (reference, _) =
            render_process_test(events, false, smooth_shape, swarm_mode, oversampling_factor);
        let (candidate, chunks) =
            render_process_test(events, true, smooth_shape, swarm_mode, oversampling_factor);
        assert_eq!(candidate, reference);
        chunks
    }

    fn note_on(offset: u32) -> Event {
        Event::new(
            offset,
            EventBody::NoteOn {
                group: 0,
                channel: 1,
                note: 60,
                velocity: 127,
            },
        )
    }

    fn configure_audio_rate_routes(params: &KurvParams, targets: &[u8]) {
        params.lfo1_active.set_value(true);
        params.lfo1_rate.set_value(17.0);
        params.lfo1_bipolar.set_value(true);
        let sources = [
            &params.mod1_source,
            &params.mod2_source,
            &params.mod3_source,
            &params.mod4_source,
            &params.mod5_source,
            &params.mod6_source,
            &params.mod7_source,
            &params.mod8_source,
            &params.mod9_source,
            &params.mod10_source,
            &params.mod11_source,
            &params.mod12_source,
            &params.mod13_source,
            &params.mod14_source,
            &params.mod15_source,
            &params.mod16_source,
        ];
        let route_targets = [
            &params.mod1_target,
            &params.mod2_target,
            &params.mod3_target,
            &params.mod4_target,
            &params.mod5_target,
            &params.mod6_target,
            &params.mod7_target,
            &params.mod8_target,
            &params.mod9_target,
            &params.mod10_target,
            &params.mod11_target,
            &params.mod12_target,
            &params.mod13_target,
            &params.mod14_target,
            &params.mod15_target,
            &params.mod16_target,
        ];
        let target_exts = [
            &params.mod1_target_ext,
            &params.mod2_target_ext,
            &params.mod3_target_ext,
            &params.mod4_target_ext,
            &params.mod5_target_ext,
            &params.mod6_target_ext,
            &params.mod7_target_ext,
            &params.mod8_target_ext,
            &params.mod9_target_ext,
            &params.mod10_target_ext,
            &params.mod11_target_ext,
            &params.mod12_target_ext,
            &params.mod13_target_ext,
            &params.mod14_target_ext,
            &params.mod15_target_ext,
            &params.mod16_target_ext,
        ];
        let amounts = [
            &params.mod1_amount,
            &params.mod2_amount,
            &params.mod3_amount,
            &params.mod4_amount,
            &params.mod5_amount,
            &params.mod6_amount,
            &params.mod7_amount,
            &params.mod8_amount,
            &params.mod9_amount,
            &params.mod10_amount,
            &params.mod11_amount,
            &params.mod12_amount,
            &params.mod13_amount,
            &params.mod14_amount,
            &params.mod15_amount,
            &params.mod16_amount,
        ];
        for (index, target) in targets.iter().copied().enumerate() {
            sources[index].set_value(1);
            amounts[index].set_value(1.0);
            if target <= modulation_target::LEGACY_TARGET_COUNT {
                route_targets[index].set_value(i64::from(target));
                target_exts[index].set_value(0);
            } else {
                route_targets[index].set_value(0);
                target_exts[index]
                    .set_value(i64::from(target - modulation_target::LEGACY_TARGET_COUNT));
            }
        }
    }

    fn render_audio_rate_route_test(
        target: u8,
        block_major_enabled: bool,
        frames: usize,
    ) -> (Vec<(f32, f32)>, usize, std::time::Duration) {
        render_audio_rate_route_config_test(&[target], block_major_enabled, frames, 64, 8, 2, 1)
    }

    fn render_audio_rate_route_config_test(
        targets: &[u8],
        block_major_enabled: bool,
        frames: usize,
        unison_voices: i64,
        polyphony: i64,
        oversampling_factor: i64,
        oscillator_count: usize,
    ) -> (Vec<(f32, f32)>, usize, std::time::Duration) {
        let params = KurvParams::default();
        params.unison_voices.set_value(unison_voices);
        params.osc2_unison_voices.set_value(unison_voices);
        params.osc3_unison_voices.set_value(unison_voices);
        params.voice_mode.set_value(polyphony);
        params.osc2_enabled.set_value(oscillator_count >= 2);
        params.osc3_enabled.set_value(oscillator_count >= 3);
        params.oversampling.set_value(oversampling_factor);
        params.phase_random.set_value(0.0);
        params.unison_detune.set_value(48.0);
        params.unison_detune_amount.set_value(1.0);
        configure_audio_rate_routes(&params, targets);
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            block_major_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, frames));
        let mut input_events = EventList::with_capacity(polyphony as usize);
        for voice in 0..polyphony {
            input_events.push(Event::new(
                0,
                EventBody::NoteOn {
                    group: 0,
                    channel: 1,
                    note: 48 + voice as u8,
                    velocity: 127,
                },
            ));
        }
        let mut output_events = EventList::with_capacity(0);
        let transport = TransportInfo::default();
        let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        let start = std::time::Instant::now();
        {
            let inputs: [&[f32]; 0] = [];
            let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
            let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);
            let _ = <Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            );
        }
        (
            left.into_iter().zip(right).collect(),
            state.block_major_chunks,
            start.elapsed(),
        )
    }

    fn dense_note_events(tail_pitch_bend: Option<u32>) -> Vec<Event> {
        let mut events = (0..24)
            .map(|voice| {
                Event::new(
                    0,
                    EventBody::NoteOn {
                        group: 0,
                        channel: 1,
                        note: 48 + voice,
                        velocity: 127,
                    },
                )
            })
            .collect::<Vec<_>>();
        if let Some(offset) = tail_pitch_bend {
            events.push(Event::new(
                offset,
                EventBody::PitchBend {
                    group: 0,
                    channel: 1,
                    value: 12_288,
                },
            ));
        }
        events
    }

    fn render_dense_pool_process(
        oversampling_factor: u8,
        frames: usize,
        pool_enabled: bool,
        tail_pitch_bend: Option<u32>,
    ) -> (Vec<(f32, f32)>, usize, usize, [u64; 3]) {
        let params = KurvParams::default();
        params.unison_voices.set_value(64);
        params.voice_mode.set_value(24);
        params
            .oversampling
            .set_value(i64::from(oversampling_factor));
        params.phase_random.set_value(0.0);
        params.unison_swarm.set_value(1.0);
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            internal_pool_enabled: pool_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, frames));

        let events = dense_note_events(tail_pitch_bend);
        let mut rendered = Vec::with_capacity(frames * 2);
        for pass in 0..2 {
            let mut input_events =
                EventList::with_capacity(if pass == 0 { events.len() } else { 0 });
            if pass == 0 {
                for event in &events {
                    input_events.push(*event);
                }
            }
            let mut output_events = EventList::with_capacity(0);
            let transport = TransportInfo::default();
            let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            {
                let inputs: [&[f32]; 0] = [];
                let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
                let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);
                let _ = <Kurv as PluginLogic>::process(
                    &mut state,
                    &params,
                    &mut buffer,
                    &input_events,
                    &mut context,
                );
            }
            rendered.extend(left.into_iter().zip(right));
        }
        (
            rendered,
            state.internal_pool_coarse_jobs,
            state.internal_pool_partial_serial_jobs,
            state.internal_pool.worker_participation(),
        )
    }

    #[test]
    fn production_pool_matches_serial_for_24_by_64_jitter() {
        let mut total_participation = 0;
        for factor in 1..=4 {
            let (serial, serial_jobs, _, _) = render_dense_pool_process(factor, 512, false, None);
            let (pooled, pooled_jobs, _, participation) =
                render_dense_pool_process(factor, 512, true, None);
            assert_eq!(pooled, serial, "factor {factor} output and continuation");
            assert_eq!(serial_jobs, 0);
            assert!(
                pooled_jobs > 0,
                "factor {factor} never dispatched a coarse job"
            );
            total_participation += participation.into_iter().sum::<u64>();
        }
        assert!(total_participation > 0, "helpers never claimed a voice");
    }

    #[test]
    fn partial_event_tail_stays_serial() {
        let (serial, serial_jobs, _, _) = render_dense_pool_process(2, 128, false, Some(100));
        let (pooled, pooled_jobs, partial_serial_jobs, participation) =
            render_dense_pool_process(2, 128, true, Some(100));
        assert_eq!(pooled, serial);
        assert_eq!(serial_jobs, 0);
        assert_eq!(pooled_jobs, 0);
        assert!(partial_serial_jobs > 0);
        assert_eq!(participation, [0; 3]);
    }

    #[test]
    fn block_path_respects_pitch_and_mpe_event_boundaries() {
        let events = [
            note_on(0),
            Event::new(
                16,
                EventBody::PitchBend {
                    group: 0,
                    channel: 1,
                    value: 12_288,
                },
            ),
            Event::new(
                32,
                EventBody::PerNotePitchBend {
                    group: 0,
                    channel: 1,
                    note: 60,
                    value: 0x9000_0000,
                },
            ),
            Event::new(
                48,
                EventBody::ParamChange {
                    id: u32::from(P::PitchBend),
                    value: -0.25,
                },
            ),
            Event::new(
                71,
                EventBody::PitchBend2 {
                    group: 0,
                    channel: 1,
                    value: 0x7000_0000,
                },
            ),
        ];
        for factor in 1..=4 {
            assert!(assert_process_paths_equal(&events, false, None, factor) > 0);
        }
    }

    #[test]
    fn block_path_falls_back_for_smoothed_controls_and_release() {
        let held = [note_on(0)];
        for factor in 1..=4 {
            assert_eq!(assert_process_paths_equal(&held, true, None, factor), 0);
        }

        let released = [
            note_on(0),
            Event::new(
                32,
                EventBody::NoteOff {
                    group: 0,
                    channel: 1,
                    note: 60,
                    velocity: 0,
                },
            ),
        ];
        for factor in 1..=4 {
            assert_eq!(
                assert_process_paths_equal(&released, false, None, factor),
                1
            );
        }
    }

    #[test]
    fn audio_rate_modulation_route_matrix_stays_finite_and_sample_accurate() {
        let targets = [
            modulation_target::target_for_param(P::Shape).unwrap(),
            modulation_target::target_for_param(P::Osc1Transpose).unwrap(),
            modulation_target::target_for_param(P::UnisonDetune).unwrap(),
            modulation_target::target_for_param(P::UnisonCurve).unwrap(),
            modulation_target::target_for_param(P::UnisonHarmonicAlign).unwrap(),
            modulation_target::target_for_param(P::OutputDb).unwrap(),
        ];
        for target in targets {
            let (reference, _, _) = render_audio_rate_route_test(target, false, 512);
            let (candidate, _, _) = render_audio_rate_route_test(target, true, 512);
            let maximum_error = candidate
                .iter()
                .zip(&reference)
                .flat_map(
                    |((candidate_left, candidate_right), (reference_left, reference_right))| {
                        [
                            (candidate_left - reference_left).abs(),
                            (candidate_right - reference_right).abs(),
                        ]
                    },
                )
                .fold(0.0_f32, f32::max);
            eprintln!("target={target},max_abs_error={maximum_error:.9e}");
            assert!(maximum_error < 5.0e-2, "target {target}");
            assert!(
                reference
                    .iter()
                    .flat_map(|(left, right)| [left, right])
                    .all(|sample| sample.is_finite()),
                "target {target}"
            );
            assert!(
                reference.windows(2).any(|samples| samples[0] != samples[1]),
                "target {target} is static"
            );
        }
    }

    #[test]
    fn output_modulation_keeps_the_block_renderer_eligible() {
        let output = modulation_target::target_for_param(P::OutputDb).unwrap();
        let (reference, _, _) = render_audio_rate_route_test(output, false, 512);
        let (candidate, chunks, _) = render_audio_rate_route_test(output, true, 512);
        let maximum_error = candidate
            .iter()
            .zip(&reference)
            .flat_map(
                |((candidate_left, candidate_right), (reference_left, reference_right))| {
                    [
                        (candidate_left - reference_left).abs(),
                        (candidate_right - reference_right).abs(),
                    ]
                },
            )
            .fold(0.0_f32, f32::max);
        eprintln!("output_modulation_max_abs_error={maximum_error:.9e}");
        assert!(maximum_error < 1.0e-3);
        assert!(
            chunks > 0,
            "output modulation fell back to per-sample rendering"
        );
    }

    #[test]
    fn audio_rate_modulation_route_matrix_reports_controlled_costs() {
        let targets = [
            (
                "shape",
                modulation_target::target_for_param(P::Shape).unwrap(),
            ),
            (
                "osc-pitch",
                modulation_target::target_for_param(P::Osc1Transpose).unwrap(),
            ),
            (
                "unison-range",
                modulation_target::target_for_param(P::UnisonDetune).unwrap(),
            ),
            (
                "unison-distribution",
                modulation_target::target_for_param(P::UnisonCurve).unwrap(),
            ),
            (
                "unison-align",
                modulation_target::target_for_param(P::UnisonHarmonicAlign).unwrap(),
            ),
            (
                "output",
                modulation_target::target_for_param(P::OutputDb).unwrap(),
            ),
        ];
        for (name, target) in targets {
            let (_, serial_chunks, serial_time) =
                render_audio_rate_route_test(target, false, 4_096);
            let (_, block_chunks, block_time) = render_audio_rate_route_test(target, true, 4_096);
            eprintln!(
                "modulation_route={name},serial_ns_per_frame={:.3},block_ns_per_frame={:.3},serial_block_chunks={serial_chunks},block_block_chunks={block_chunks}",
                serial_time.as_nanos() as f64 / 4_096.0,
                block_time.as_nanos() as f64 / 4_096.0,
            );
        }
    }

    #[test]
    fn audio_rate_modulation_target_catalog_matches_the_scalar_reference() {
        for target in 1..=modulation_target::TARGET_COUNT {
            let descriptor = modulation_target::descriptor(target).unwrap();
            let (reference, _, serial_time) =
                render_audio_rate_route_config_test(&[target], false, 512, 64, 8, 2, 3);
            let (candidate, chunks, block_time) =
                render_audio_rate_route_config_test(&[target], true, 512, 64, 8, 2, 3);
            let maximum_error = candidate
                .iter()
                .zip(&reference)
                .flat_map(
                    |((candidate_left, candidate_right), (reference_left, reference_right))| {
                        [
                            (candidate_left - reference_left).abs(),
                            (candidate_right - reference_right).abs(),
                        ]
                    },
                )
                .fold(0.0_f32, f32::max);
            assert!(
                candidate
                    .iter()
                    .flat_map(|(left, right)| [left, right])
                    .all(|sample| sample.is_finite()),
                "target {target} ({}) produced a non-finite sample",
                descriptor.label
            );
            assert!(
                maximum_error < 5.0e-2,
                "target {target} ({}) diverged by {maximum_error:.9e}",
                descriptor.label
            );
            eprintln!(
                "catalog_target={target},label={},max_abs_error={maximum_error:.9e},serial_ns_per_frame={:.3},block_ns_per_frame={:.3},block_chunks={chunks}",
                descriptor.label,
                serial_time.as_nanos() as f64 / 512.0,
                block_time.as_nanos() as f64 / 512.0,
            );
        }
    }

    #[test]
    fn audio_rate_modulation_block_paths_survive_voice_and_oversampling_stress() {
        let target = |param| modulation_target::target_for_param(param).unwrap();
        let cases: [&[u8]; 5] = [
            &[target(P::Shape), target(P::OutputDb)],
            &[
                target(P::Osc1Transpose),
                target(P::UnisonDetune),
                target(P::UnisonCurve),
                target(P::UnisonHarmonicAlign),
            ],
            &[target(P::PulseWidth), target(P::Attack)],
            &[target(P::UnisonStereo), target(P::UnisonWeight)],
            &[
                target(P::Osc1Transpose),
                target(P::PulseWidth),
                target(P::OutputDb),
            ],
        ];
        let configurations = [(1, 1, 1, 1), (16, 8, 2, 1), (64, 8, 2, 3), (64, 24, 4, 3)];
        for (case_index, targets) in cases.into_iter().enumerate() {
            for &(voices, polyphony, oversampling, oscillators) in &configurations {
                let (reference, _, _) = render_audio_rate_route_config_test(
                    targets,
                    false,
                    1_024,
                    voices,
                    polyphony,
                    oversampling,
                    oscillators,
                );
                let (candidate, chunks, _) = render_audio_rate_route_config_test(
                    targets,
                    true,
                    1_024,
                    voices,
                    polyphony,
                    oversampling,
                    oscillators,
                );
                let maximum_error = candidate
                    .iter()
                    .zip(&reference)
                    .flat_map(
                        |((candidate_left, candidate_right), (reference_left, reference_right))| {
                            [
                                (candidate_left - reference_left).abs(),
                                (candidate_right - reference_right).abs(),
                            ]
                        },
                    )
                    .fold(0.0_f32, f32::max);
                assert!(
                    candidate
                        .iter()
                        .flat_map(|(left, right)| [left, right])
                        .all(|sample| sample.is_finite()),
                    "case {case_index} produced a non-finite sample"
                );
                assert!(
                    maximum_error < 5.0e-2,
                    "case {case_index} voices={voices} polyphony={polyphony} oversampling={oversampling} oscillators={oscillators} diverged by {maximum_error:.9e}"
                );
                assert!(
                    chunks > 0,
                    "case {case_index} voices={voices} polyphony={polyphony} oversampling={oversampling} oscillators={oscillators} did not use a block path"
                );
            }
        }
    }

    #[test]
    fn adaptive_wander_blocks_match_the_pair_path() {
        for factor in 1..=4 {
            assert!(
                assert_process_paths_equal(&[note_on(0)], false, Some(SwarmMode::Wander), factor,)
                    > 0
            );
        }
    }

    #[test]
    fn voice_renders_silence_when_idle() {
        let mut voice = VaVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings::new(2.0, 220.0, 0.5, 1.0, 0.0, 0.0);

        let (l, r) = voice.render(settings, 48_000.0, false);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn voice_renders_nonzero_in_drone_mode() {
        let mut voice = VaVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings::new(2.0, 220.0, 0.5, 1.0, 0.0, 0.0);

        let mut peak = 0.0_f32;
        for _ in 0..512 {
            let (l, r) = voice.render(settings, 48_000.0, true);
            peak = peak.max(l.abs()).max(r.abs());
        }

        assert!(peak.is_finite());
        assert!(peak > 0.01);
    }

    #[test]
    fn phase_randomization_changes_a_new_notes_start_phase() {
        let settings = VoiceSettings::new(0.0, 220.0, 0.5, 1.0, 0.0, 0.0);
        let mut fixed = VaVoice::default();
        fixed.set_sample_rate(48_000.0);
        fixed.configure_unison(UnisonSettings::new(1, 0.0, 0.0, 0.0, 0.0));
        fixed.start(57, 1.0, 0, None, 1);

        let mut randomized = VaVoice::default();
        randomized.set_sample_rate(48_000.0);
        randomized.configure_unison(UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0));
        randomized.start(57, 1.0, 0, None, 1);

        let (fixed_sample, _) = fixed.render(settings, 48_000.0, false);
        let (random_sample, _) = randomized.render(settings, 48_000.0, false);
        assert!((fixed_sample - random_sample).abs() > 1.0e-5);
    }
}
