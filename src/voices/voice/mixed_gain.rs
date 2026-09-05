//! Shared mixed PM/gain post-processing; depth values stay at audio rate.
pub(super) fn apply<const SAMPLES: usize>(
    output: &mut [(f32, f32); SAMPLES],
    source: &[f32; SAMPLES],
    base_gains: (f32, f32),
    control: crate::OscillatorControl,
    amount: f32,
    gain_amounts: &[f32; SAMPLES],
    dynamic_gain: bool,
) {
    let base_gain_position = || {
        let left_power = base_gains.0 * base_gains.0;
        let right_power = base_gains.1 * base_gains.1;
        (
            (left_power + right_power).sqrt() * std::f32::consts::FRAC_1_SQRT_2,
            (right_power - left_power) / (right_power + left_power).max(f32::EPSILON),
        )
    };
    match control {
        crate::OscillatorControl::Level => {
            let (base_level, base_pan) = base_gain_position();
            let base_pan = base_pan.clamp(-1.0, 1.0);
            let left_pan_gain = (1.0 - base_pan).sqrt();
            let right_pan_gain = (1.0 + base_pan).sqrt();
            for frame in 0..SAMPLES {
                let delta = source[frame] * gain_amounts[frame];
                let (left_gain, right_gain) = if delta == 0.0 {
                    (base_gains.0, base_gains.1)
                } else {
                    let level = (base_level + delta).clamp(0.0, 1.0);
                    (level * left_pan_gain, level * right_pan_gain)
                };
                output[frame].0 *= left_gain;
                output[frame].1 *= right_gain;
            }
        }
        crate::OscillatorControl::Pan => {
            let (base_level, base_pan) = base_gain_position();
            let level = base_level.clamp(0.0, 1.0);
            for frame in 0..SAMPLES {
                let delta = source[frame] * gain_amounts[frame];
                let (left_gain, right_gain) = if delta == 0.0 {
                    (base_gains.0, base_gains.1)
                } else {
                    let pan = (base_pan + delta).clamp(-1.0, 1.0);
                    (level * (1.0 - pan).sqrt(), level * (1.0 + pan).sqrt())
                };
                output[frame].0 *= left_gain;
                output[frame].1 *= right_gain;
            }
        }
        crate::OscillatorControl::RingModAmount => {
            let wet = amount.abs();
            let dry = 1.0 - wet;
            let signed_wet = amount.signum() * wet;
            for frame in 0..SAMPLES {
                let ring_gain = if dynamic_gain {
                    let amount = gain_amounts[frame];
                    let wet = amount.abs();
                    (1.0 - wet) + source[frame] * amount.signum() * wet
                } else {
                    dry + source[frame] * signed_wet
                };
                output[frame].0 *= base_gains.0 * ring_gain;
                output[frame].1 *= base_gains.1 * ring_gain;
            }
        }
        _ => unreachable!("mixed route must contain a gain control"),
    }
}
