use truce_simd::simd::{f32x4, f32x8};

use super::{
    cosine_phase4, cosine_phase8, sine_cosine_phase4, sine_cosine_phase8, sine_phase4, sine_phase8,
    wrap_phase4, wrap_phase8,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhaseWarpMode {
    #[default]
    None,
    Pwm,
    PhaseBend,
    Harmonic,
}

impl PhaseWarpMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Pwm,
            2 => Self::PhaseBend,
            3 => Self::Harmonic,
            _ => Self::None,
        }
    }
}

pub(super) fn warp_phase_scalar(
    phase: f32,
    phase_step: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> (f32, f32) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            const PWM_NORMALIZATION: f32 = 0.058_174_6;
            let angle = std::f32::consts::TAU * phase;
            let displacement = (angle.cos() - (2.0 * angle).cos()) * PWM_NORMALIZATION;
            let derivative = (-std::f32::consts::TAU * angle.sin()
                + 2.0 * std::f32::consts::TAU * (2.0 * angle).sin())
                * PWM_NORMALIZATION;
            (
                phase - depth * displacement,
                phase_step * (1.0 - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            let angle = 2.0 * std::f32::consts::TAU * phase;
            let displacement = angle.sin() / (2.0 * std::f32::consts::TAU);
            let derivative = angle.cos();
            (
                phase - depth * displacement,
                phase_step * (1.0 - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            let angle = std::f32::consts::TAU * phase;
            (
                phase - depth * angle.sin() / std::f32::consts::TAU,
                phase_step * (1.0 - depth * angle.cos()),
            )
        }
    }
}

#[inline]
fn inverse_warp_phase_scalar(
    target: f32,
    phase_step: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32 {
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return target;
    }
    let safe_step = phase_step.max(f32::EPSILON);
    let mut phase = target;
    for _ in 0..3 {
        let (mapped, warped_step) = warp_phase_scalar(phase, phase_step, mode, amount);
        let derivative = (warped_step / safe_step).max(0.05);
        phase = (phase - (mapped - target) / derivative).clamp(0.0, 1.0);
    }
    phase
}

#[inline]
pub(super) fn warped_pulse_edge_scalar(
    phase_step: f32,
    pulse_width: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> Option<f32> {
    (mode != PhaseWarpMode::None && amount > f32::EPSILON).then(|| {
        let width = phase_step
            .max(pulse_width.clamp(0.03, 0.97))
            .min(1.0 - phase_step);
        inverse_warp_phase_scalar(width, phase_step, mode, amount)
    })
}

#[inline]
pub(super) fn warp_phase_position_scalar(
    phase: f32,
    phase_step: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            const NORMALIZATION: f32 = 0.058_174_6;
            let angle = std::f32::consts::TAU * phase;
            phase - depth * (angle.cos() - (2.0 * angle).cos()) * NORMALIZATION
        }
        PhaseWarpMode::PhaseBend => {
            phase
                - depth * (2.0 * std::f32::consts::TAU * phase).sin()
                    / (2.0 * std::f32::consts::TAU)
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * (std::f32::consts::TAU * phase).sin() / std::f32::consts::TAU
        }
    }
}

#[inline]
pub(super) fn warp_phase4(
    phase: f32x4,
    phase_step: f32x4,
    mode: PhaseWarpMode,
    amount: f32,
) -> (f32x4, f32x4) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let normalization = f32x4::splat(0.058_174_6);
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            let (sine, cosine) = sine_cosine_phase4(phase);
            let (second_sine, second_cosine) = sine_cosine_phase4(second_phase);
            let displacement = (cosine - second_cosine) * normalization;
            let derivative = (second_sine * f32x4::splat(2.0) - sine)
                * f32x4::splat(std::f32::consts::TAU)
                * normalization;
            (
                phase - depth * displacement,
                phase_step * (f32x4::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            let displacement =
                sine_phase4(second_phase) * f32x4::splat((2.0 * std::f32::consts::TAU).recip());
            let derivative = cosine_phase4(second_phase);
            (
                phase - depth * displacement,
                phase_step * (f32x4::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let sine = sine_phase4(phase);
            let cosine = cosine_phase4(phase);
            (
                phase - depth * sine * f32x4::splat(std::f32::consts::TAU.recip()),
                phase_step * (f32x4::ONE - depth * cosine),
            )
        }
    }
}

#[inline]
fn inverse_warp_phase4(
    target: f32x4,
    phase_step: f32x4,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x4 {
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return target;
    }
    let safe_step = phase_step.fast_max(f32x4::splat(f32::EPSILON));
    let mut phase = target;
    for _ in 0..3 {
        let (mapped, warped_step) = warp_phase4(phase, phase_step, mode, amount);
        let derivative = (warped_step / safe_step).fast_max(f32x4::splat(0.05));
        phase = (phase - (mapped - target) / derivative)
            .fast_max(f32x4::ZERO)
            .fast_min(f32x4::ONE);
    }
    phase
}

#[inline]
pub(super) fn warped_pulse_edge4(
    phase_step: f32x4,
    pulse_width: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> Option<f32x4> {
    (mode != PhaseWarpMode::None && amount > f32::EPSILON).then(|| {
        let one = f32x4::ONE;
        let width = phase_step
            .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
            .fast_min(one - phase_step);
        inverse_warp_phase4(width, phase_step, mode, amount)
    })
}

#[inline]
pub(super) fn warp_phase_position4(
    phase: f32x4,
    phase_step: f32x4,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x4 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = f32x4::splat(amount * 0.95).fast_min(
        (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
            .fast_max(f32x4::ZERO),
    );
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            phase
                - depth
                    * (cosine_phase4(phase) - cosine_phase4(second_phase))
                    * f32x4::splat(0.058_174_6)
        }
        PhaseWarpMode::PhaseBend => {
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            phase
                - depth
                    * sine_phase4(second_phase)
                    * f32x4::splat((2.0 * std::f32::consts::TAU).recip())
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * sine_phase4(phase) * f32x4::splat(std::f32::consts::TAU.recip())
        }
    }
}

#[inline]
pub(super) fn warp_phase8(
    phase: f32x8,
    phase_step: f32x8,
    mode: PhaseWarpMode,
    amount: f32,
) -> (f32x8, f32x8) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let normalization = f32x8::splat(0.058_174_6);
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            let (sine, cosine) = sine_cosine_phase8(phase);
            let (second_sine, second_cosine) = sine_cosine_phase8(second_phase);
            let displacement = (cosine - second_cosine) * normalization;
            let derivative = (second_sine * f32x8::splat(2.0) - sine)
                * f32x8::splat(std::f32::consts::TAU)
                * normalization;
            (
                phase - depth * displacement,
                phase_step * (f32x8::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            let displacement =
                sine_phase8(second_phase) * f32x8::splat((2.0 * std::f32::consts::TAU).recip());
            let derivative = cosine_phase8(second_phase);
            (
                phase - depth * displacement,
                phase_step * (f32x8::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let sine = sine_phase8(phase);
            let cosine = cosine_phase8(phase);
            (
                phase - depth * sine * f32x8::splat(std::f32::consts::TAU.recip()),
                phase_step * (f32x8::ONE - depth * cosine),
            )
        }
    }
}

#[inline]
fn inverse_warp_phase8(
    target: f32x8,
    phase_step: f32x8,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x8 {
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return target;
    }
    let safe_step = phase_step.fast_max(f32x8::splat(f32::EPSILON));
    let mut phase = target;
    for _ in 0..3 {
        let (mapped, warped_step) = warp_phase8(phase, phase_step, mode, amount);
        let derivative = (warped_step / safe_step).fast_max(f32x8::splat(0.05));
        phase = (phase - (mapped - target) / derivative)
            .fast_max(f32x8::ZERO)
            .fast_min(f32x8::ONE);
    }
    phase
}

#[inline]
pub(super) fn warped_pulse_edge8(
    phase_step: f32x8,
    pulse_width: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> Option<f32x8> {
    (mode != PhaseWarpMode::None && amount > f32::EPSILON).then(|| {
        let one = f32x8::ONE;
        let width = phase_step
            .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
            .fast_min(one - phase_step);
        inverse_warp_phase8(width, phase_step, mode, amount)
    })
}

#[inline]
pub(super) fn warp_phase_position8(
    phase: f32x8,
    phase_step: f32x8,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x8 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = f32x8::splat(amount * 0.95).fast_min(
        (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
            .fast_max(f32x8::ZERO),
    );
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            phase
                - depth
                    * (cosine_phase8(phase) - cosine_phase8(second_phase))
                    * f32x8::splat(0.058_174_6)
        }
        PhaseWarpMode::PhaseBend => {
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            phase
                - depth
                    * sine_phase8(second_phase)
                    * f32x8::splat((2.0 * std::f32::consts::TAU).recip())
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * sine_phase8(phase) * f32x8::splat(std::f32::consts::TAU.recip())
        }
    }
}
