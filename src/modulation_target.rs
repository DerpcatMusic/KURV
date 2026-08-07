use crate::P;

pub const LEGACY_TARGET_COUNT: u8 = 21;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscTarget {
    Pitch,
    Shape,
    PulseWidth,
    Warp,
    CustomShape,
    Level,
    Pan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnisonTarget {
    DetuneAmount,
    DetuneRange,
    Stereo,
    PhaseRandom,
    Curve,
    JitterAmount,
    JitterRate,
    StereoX,
    StereoY,
    Weight,
    PanCenter,
    PanLeft,
    PanRight,
    PanCenterX,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalTarget {
    Output,
    Attack,
    Decay,
    Sustain,
    Release,
    AttackCurve,
    DecayCurve,
    ReleaseCurve,
    AttackCurveTime,
    DecayCurveTime,
    ReleaseCurveTime,
    Velocity,
    Pressure,
    Timbre,
    Glide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Oscillator {
        oscillator: u8,
        control: OscTarget,
    },
    Unison {
        oscillator: u8,
        control: UnisonTarget,
    },
    Global(GlobalTarget),
}

#[derive(Clone, Copy, Debug)]
pub struct TargetDescriptor {
    pub param: P,
    pub label: &'static str,
    pub kind: TargetKind,
    pub scale: f32,
    pub normalized_span: f32,
}

const fn oscillator(
    param: P,
    label: &'static str,
    oscillator: u8,
    control: OscTarget,
    scale: f32,
    normalized_span: f32,
) -> TargetDescriptor {
    TargetDescriptor {
        param,
        label,
        kind: TargetKind::Oscillator {
            oscillator,
            control,
        },
        scale,
        normalized_span,
    }
}

const fn unison(
    param: P,
    label: &'static str,
    oscillator: u8,
    control: UnisonTarget,
    scale: f32,
    normalized_span: f32,
) -> TargetDescriptor {
    TargetDescriptor {
        param,
        label,
        kind: TargetKind::Unison {
            oscillator,
            control,
        },
        scale,
        normalized_span,
    }
}

const fn global(
    param: P,
    label: &'static str,
    control: GlobalTarget,
    scale: f32,
    normalized_span: f32,
) -> TargetDescriptor {
    TargetDescriptor {
        param,
        label,
        kind: TargetKind::Global(control),
        scale,
        normalized_span,
    }
}

// Entries 1..=21 are the original public route IDs. Never reorder them.
pub const TARGETS: [TargetDescriptor; 81] = [
    oscillator(
        P::Osc1Transpose,
        "OSC 1 TRANSPOSE",
        0,
        OscTarget::Pitch,
        48.0,
        0.5,
    ),
    oscillator(P::Shape, "OSC 1 SHAPE", 0, OscTarget::Shape, 3.0, 1.0),
    oscillator(
        P::PulseWidth,
        "OSC 1 PULSE",
        0,
        OscTarget::PulseWidth,
        0.47,
        0.5,
    ),
    oscillator(
        P::Osc1WarpAmount,
        "OSC 1 WARP",
        0,
        OscTarget::Warp,
        1.0,
        1.0,
    ),
    oscillator(P::Osc1Level, "OSC 1 LEVEL", 0, OscTarget::Level, 1.0, 1.0),
    oscillator(P::Osc1Pan, "OSC 1 PAN", 0, OscTarget::Pan, 1.0, 0.5),
    oscillator(
        P::Osc2Transpose,
        "OSC 2 TRANSPOSE",
        1,
        OscTarget::Pitch,
        48.0,
        0.5,
    ),
    oscillator(P::Osc2Shape, "OSC 2 SHAPE", 1, OscTarget::Shape, 3.0, 1.0),
    oscillator(
        P::Osc2PulseWidth,
        "OSC 2 PULSE",
        1,
        OscTarget::PulseWidth,
        0.47,
        0.5,
    ),
    oscillator(
        P::Osc2WarpAmount,
        "OSC 2 WARP",
        1,
        OscTarget::Warp,
        1.0,
        1.0,
    ),
    oscillator(P::Osc2Level, "OSC 2 LEVEL", 1, OscTarget::Level, 1.0, 1.0),
    oscillator(P::Osc2Pan, "OSC 2 PAN", 1, OscTarget::Pan, 1.0, 0.5),
    oscillator(
        P::Osc3Transpose,
        "OSC 3 TRANSPOSE",
        2,
        OscTarget::Pitch,
        48.0,
        0.5,
    ),
    oscillator(P::Osc3Shape, "OSC 3 SHAPE", 2, OscTarget::Shape, 3.0, 1.0),
    oscillator(
        P::Osc3PulseWidth,
        "OSC 3 PULSE",
        2,
        OscTarget::PulseWidth,
        0.47,
        0.5,
    ),
    oscillator(
        P::Osc3WarpAmount,
        "OSC 3 WARP",
        2,
        OscTarget::Warp,
        1.0,
        1.0,
    ),
    oscillator(P::Osc3Level, "OSC 3 LEVEL", 2, OscTarget::Level, 1.0, 1.0),
    oscillator(P::Osc3Pan, "OSC 3 PAN", 2, OscTarget::Pan, 1.0, 0.5),
    unison(
        P::UnisonDetuneAmount,
        "OSC 1 DETUNE AMOUNT",
        0,
        UnisonTarget::DetuneAmount,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonDetuneAmount,
        "OSC 2 DETUNE AMOUNT",
        1,
        UnisonTarget::DetuneAmount,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonDetuneAmount,
        "OSC 3 DETUNE AMOUNT",
        2,
        UnisonTarget::DetuneAmount,
        1.0,
        1.0,
    ),
    oscillator(P::Osc1Cents, "OSC 1 FINE", 0, OscTarget::Pitch, 1.0, 0.5),
    oscillator(
        P::Osc1CustomShape,
        "OSC 1 CURVE MIX",
        0,
        OscTarget::CustomShape,
        1.0,
        1.0,
    ),
    unison(
        P::UnisonDetune,
        "OSC 1 RANGE",
        0,
        UnisonTarget::DetuneRange,
        4_800.0,
        1.0,
    ),
    unison(
        P::UnisonStereo,
        "OSC 1 WIDTH",
        0,
        UnisonTarget::Stereo,
        1.0,
        1.0,
    ),
    unison(
        P::PhaseRandom,
        "OSC 1 RANDOM PHASE",
        0,
        UnisonTarget::PhaseRandom,
        1.0,
        1.0,
    ),
    unison(
        P::UnisonCurve,
        "OSC 1 DISTRIBUTION",
        0,
        UnisonTarget::Curve,
        2.0,
        1.0,
    ),
    unison(
        P::UnisonSwarm,
        "OSC 1 JITTER",
        0,
        UnisonTarget::JitterAmount,
        1.0,
        1.0,
    ),
    unison(
        P::UnisonSwarmRate,
        "OSC 1 JITTER RATE",
        0,
        UnisonTarget::JitterRate,
        1.0,
        1.0,
    ),
    unison(
        P::StereoX,
        "OSC 1 STEREO X",
        0,
        UnisonTarget::StereoX,
        1.0,
        1.0,
    ),
    unison(
        P::StereoAlternate,
        "OSC 1 STEREO Y",
        0,
        UnisonTarget::StereoY,
        1.0,
        1.0,
    ),
    unison(
        P::UnisonWeight,
        "OSC 1 WEIGHT",
        0,
        UnisonTarget::Weight,
        2.0,
        1.0,
    ),
    unison(
        P::PanShapeCenter,
        "OSC 1 CENTER",
        0,
        UnisonTarget::PanCenter,
        1.0,
        1.0,
    ),
    unison(
        P::PanShapeLeft,
        "OSC 1 LEFT SIDE",
        0,
        UnisonTarget::PanLeft,
        1.0,
        1.0,
    ),
    unison(
        P::PanShapeRight,
        "OSC 1 RIGHT SIDE",
        0,
        UnisonTarget::PanRight,
        1.0,
        1.0,
    ),
    unison(
        P::PanShapeCenterX,
        "OSC 1 CENTER X",
        0,
        UnisonTarget::PanCenterX,
        0.9,
        1.0,
    ),
    oscillator(P::Osc2Cents, "OSC 2 FINE", 1, OscTarget::Pitch, 1.0, 0.5),
    oscillator(
        P::Osc2CustomShape,
        "OSC 2 CURVE MIX",
        1,
        OscTarget::CustomShape,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonDetune,
        "OSC 2 RANGE",
        1,
        UnisonTarget::DetuneRange,
        4_800.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonStereo,
        "OSC 2 WIDTH",
        1,
        UnisonTarget::Stereo,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2PhaseRandom,
        "OSC 2 RANDOM PHASE",
        1,
        UnisonTarget::PhaseRandom,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonCurve,
        "OSC 2 DISTRIBUTION",
        1,
        UnisonTarget::Curve,
        2.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonJitter,
        "OSC 2 JITTER",
        1,
        UnisonTarget::JitterAmount,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonJitterRate,
        "OSC 2 JITTER RATE",
        1,
        UnisonTarget::JitterRate,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2StereoX,
        "OSC 2 STEREO X",
        1,
        UnisonTarget::StereoX,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2StereoAlternate,
        "OSC 2 STEREO Y",
        1,
        UnisonTarget::StereoY,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2UnisonWeight,
        "OSC 2 WEIGHT",
        1,
        UnisonTarget::Weight,
        2.0,
        1.0,
    ),
    unison(
        P::Osc2PanShapeCenter,
        "OSC 2 CENTER",
        1,
        UnisonTarget::PanCenter,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2PanShapeLeft,
        "OSC 2 LEFT SIDE",
        1,
        UnisonTarget::PanLeft,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2PanShapeRight,
        "OSC 2 RIGHT SIDE",
        1,
        UnisonTarget::PanRight,
        1.0,
        1.0,
    ),
    unison(
        P::Osc2PanShapeCenterX,
        "OSC 2 CENTER X",
        1,
        UnisonTarget::PanCenterX,
        0.9,
        1.0,
    ),
    oscillator(P::Osc3Cents, "OSC 3 FINE", 2, OscTarget::Pitch, 1.0, 0.5),
    oscillator(
        P::Osc3CustomShape,
        "OSC 3 CURVE MIX",
        2,
        OscTarget::CustomShape,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonDetune,
        "OSC 3 RANGE",
        2,
        UnisonTarget::DetuneRange,
        4_800.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonStereo,
        "OSC 3 WIDTH",
        2,
        UnisonTarget::Stereo,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3PhaseRandom,
        "OSC 3 RANDOM PHASE",
        2,
        UnisonTarget::PhaseRandom,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonCurve,
        "OSC 3 DISTRIBUTION",
        2,
        UnisonTarget::Curve,
        2.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonJitter,
        "OSC 3 JITTER",
        2,
        UnisonTarget::JitterAmount,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonJitterRate,
        "OSC 3 JITTER RATE",
        2,
        UnisonTarget::JitterRate,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3StereoX,
        "OSC 3 STEREO X",
        2,
        UnisonTarget::StereoX,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3StereoAlternate,
        "OSC 3 STEREO Y",
        2,
        UnisonTarget::StereoY,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3UnisonWeight,
        "OSC 3 WEIGHT",
        2,
        UnisonTarget::Weight,
        2.0,
        1.0,
    ),
    unison(
        P::Osc3PanShapeCenter,
        "OSC 3 CENTER",
        2,
        UnisonTarget::PanCenter,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3PanShapeLeft,
        "OSC 3 LEFT SIDE",
        2,
        UnisonTarget::PanLeft,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3PanShapeRight,
        "OSC 3 RIGHT SIDE",
        2,
        UnisonTarget::PanRight,
        1.0,
        1.0,
    ),
    unison(
        P::Osc3PanShapeCenterX,
        "OSC 3 CENTER X",
        2,
        UnisonTarget::PanCenterX,
        0.9,
        1.0,
    ),
    global(P::OutputDb, "OUTPUT", GlobalTarget::Output, 54.0, 1.0),
    global(P::Attack, "ATTACK", GlobalTarget::Attack, 8.0, 1.0),
    global(P::Decay, "DECAY", GlobalTarget::Decay, 8.0, 1.0),
    global(P::Sustain, "SUSTAIN", GlobalTarget::Sustain, 1.0, 1.0),
    global(P::Release, "RELEASE", GlobalTarget::Release, 12.0, 1.0),
    global(
        P::AttackCurve,
        "ATTACK CURVE",
        GlobalTarget::AttackCurve,
        2.0,
        1.0,
    ),
    global(
        P::DecayCurve,
        "DECAY CURVE",
        GlobalTarget::DecayCurve,
        2.0,
        1.0,
    ),
    global(
        P::ReleaseCurve,
        "RELEASE CURVE",
        GlobalTarget::ReleaseCurve,
        2.0,
        1.0,
    ),
    global(
        P::AttackCurveTime,
        "ATTACK CURVE TIME",
        GlobalTarget::AttackCurveTime,
        0.9,
        1.0,
    ),
    global(
        P::DecayCurveTime,
        "DECAY CURVE TIME",
        GlobalTarget::DecayCurveTime,
        0.9,
        1.0,
    ),
    global(
        P::ReleaseCurveTime,
        "RELEASE CURVE TIME",
        GlobalTarget::ReleaseCurveTime,
        0.9,
        1.0,
    ),
    global(
        P::VelocityAmount,
        "VELOCITY",
        GlobalTarget::Velocity,
        1.0,
        1.0,
    ),
    global(
        P::PressureAmount,
        "PRESSURE",
        GlobalTarget::Pressure,
        1.0,
        1.0,
    ),
    global(P::TimbreAmount, "TIMBRE", GlobalTarget::Timbre, 1.0, 1.0),
    global(P::GlideTime, "GLIDE", GlobalTarget::Glide, 5.0, 1.0),
];

pub const TARGET_COUNT: u8 = TARGETS.len() as u8;
pub const EXTENDED_TARGET_COUNT: u8 = TARGET_COUNT - LEGACY_TARGET_COUNT;

pub fn descriptor(target: u8) -> Option<&'static TargetDescriptor> {
    TARGETS.get(usize::from(target.checked_sub(1)?))
}

pub fn target_for_param(param: P) -> Option<u8> {
    TARGETS
        .iter()
        .position(|target| target.param == param)
        .map(|index| index as u8 + 1)
}

pub fn target_oscillator(target: u8) -> Option<usize> {
    match descriptor(target)?.kind {
        TargetKind::Oscillator { oscillator, .. } | TargetKind::Unison { oscillator, .. } => {
            Some(usize::from(oscillator))
        }
        TargetKind::Global(_) => None,
    }
}
