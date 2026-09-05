pub const SAMPLE_MAX_FRAMES: usize = 131_072;
pub const GRAIN_MAX_SOURCE_FRAMES: usize = 524_288;
/// Maximum admitted absolute PCM sample for every runtime artifact compiler.
pub const MAX_SOURCE_ABS_SAMPLE: f32 = 16.0;
const SOURCE_INTEGRAL_BLOCK_FRAMES: usize = 64;
const OFFLINE_DECIMATOR_LOBES: usize = 8;
// A dyadic family is worker-built until a scalar terminal level. The derived
// PCM lengths form a strict geometric sum smaller than the authoritative base.
const MIP_BLEND_START_RESIDUAL: f32 = 1.5;
const MIP_HALF_TAPS: isize = 16;
// Offline-designed 33-tap Blackman-windowed sinc, cutoff 0.225 cycles/input.
// Every worker output is an integer-centred dot product; no per-output trig.
const MIP_HALF_COEFFICIENTS: [f64; 33] = [
    1.622_936_989_835_286_25e-19,
    5.278_834_186_545_985_38e-05,
    2.691_041_028_696_049_69e-04,
    -3.877_581_761_855_243_78e-04,
    -1.676_411_447_421_974_52e-03,
    5.052_280_764_045_849_51e-04,
    5.478_197_622_359_497_03e-03,
    1.375_234_507_365_200_55e-03,
    -1.286_701_978_955_544_40e-02,
    -9.159_189_462_686_349_18e-03,
    2.381_247_625_264_390_84e-02,
    3.003_547_526_470_456_97e-02,
    -3.618_526_031_935_460_91e-02,
    -8.190_972_391_307_486_94e-02,
    4.616_078_689_776_389_33e-02,
    3.094_791_891_974_973_95e-01,
    4.500_337_656_896_091_22e-01,
    3.094_791_891_974_973_95e-01,
    4.616_078_689_776_389_33e-02,
    -8.190_972_391_307_486_94e-02,
    -3.618_526_031_935_460_91e-02,
    3.003_547_526_470_456_97e-02,
    2.381_247_625_264_390_84e-02,
    -9.159_189_462_686_349_18e-03,
    -1.286_701_978_955_544_40e-02,
    1.375_234_507_365_200_55e-03,
    5.478_197_622_359_497_03e-03,
    5.052_280_764_045_849_51e-04,
    -1.676_411_447_421_974_52e-03,
    -3.877_581_761_855_243_78e-04,
    2.691_041_028_696_049_69e-04,
    5.278_834_186_545_985_38e-05,
    1.622_936_989_835_286_25e-19,
];
/// Internal realtime capacity, never a user-facing grain-count control.
pub const GRAIN_TELEMETRY: usize = 8;
pub const GRAIN_LAYERS: usize = GRAIN_TELEMETRY;
pub const RICH_ZONE_COUNT: usize = 22;
/// Bounded source-ordered Rich timeline. Full-resolution 4096-sample waveform
/// frames retain the existing spectral guard while covering 32 source regions.
pub const RICH_FRAME_COUNT: usize = 32;
pub const RICH_FRAME_SAMPLES: usize = 4_096;
pub const RICH_ZONE_SAMPLES: usize = RICH_FRAME_COUNT * RICH_FRAME_SAMPLES;
/// Wire shape retained for packs written before the Rich timeline migration.
pub const LEGACY_RICH_FRAME_COUNT: usize = 8;
pub const LEGACY_RICH_FRAME_SAMPLES: usize = 4_096;
pub const LEGACY_RICH_ZONE_SAMPLES: usize = LEGACY_RICH_FRAME_COUNT * LEGACY_RICH_FRAME_SAMPLES;
pub const RICH_ASSET_SAMPLE_RATE: f32 = 48_000.0;
pub const RICH_GUARD_HZ: f32 = 20_000.0;
pub const RICH_STORAGE_BYTES: usize =
    RICH_ZONE_COUNT * RICH_ZONE_SAMPLES * std::mem::size_of::<f32>();
#[expect(
    dead_code,
    reason = "published recurrence contract used by validation tooling"
)]
pub const RICH_RECURRENCE_FRAMES: u64 = RICH_ZONE_SAMPLES as u64;
#[expect(
    dead_code,
    reason = "published recurrence contract used by validation tooling"
)]
pub const GRAIN_RECURRENCE_EVENTS: u64 = u64::MAX;
const _: () = assert!(RICH_STORAGE_BYTES <= 16 * 1024 * 1024);
const _: () = assert!(RICH_FRAME_SAMPLES * RICH_FRAME_COUNT == RICH_ZONE_SAMPLES);

pub(super) const MIDI_ZERO_HZ: f32 = 8.175_799;
pub(super) const MIN_SAMPLE_LOOP_FRAMES: usize = 256;
// The admitted 384 kHz / 20 Hz endpoint spans 19,200 source frames.
// Keeping the complete period is required for integer-cycle loop quantization.
pub(super) const MAX_SAMPLE_PERIOD_FRAMES: f32 = 19_200.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactBuildError {
    Empty,
    NonFinite,
    RootRequired,
    Allocation,
    Cancelled,
}

/// Deterministic worker-side projection whose coordinates remain in its
/// declared effective sample rate. The audio thread never constructs this.
pub(crate) struct BoundedSourceProjection {
    pub(crate) samples: Vec<f32>,
    pub(crate) sample_rate: f32,
    pub(crate) stride: usize,
}

pub(crate) fn bandlimit_source_by_stride_with_cancel(
    source: &[f32],
    source_sample_rate: f32,
    stride: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<BoundedSourceProjection, ArtifactBuildError> {
    validate_source(source)?;
    if !source_sample_rate.is_finite() || source_sample_rate <= 0.0 || stride == 0 {
        return Err(ArtifactBuildError::NonFinite);
    }
    Ok(BoundedSourceProjection {
        samples: bandlimited_decimate_with_cancel(source, stride, should_cancel)?,
        sample_rate: source_sample_rate / stride as f32,
        stride,
    })
}

pub(super) fn bandlimited_decimate_with_cancel(
    source: &[f32],
    stride: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    if should_cancel() {
        return Err(ArtifactBuildError::Cancelled);
    }
    if stride <= 1 {
        return Ok(source.to_vec());
    }
    let half = OFFLINE_DECIMATOR_LOBES * stride;
    let cutoff = 0.45_f64 / stride as f64;
    let mut kernel = Vec::with_capacity(half * 2 + 1);
    for tap in 0..=half * 2 {
        let offset = tap as isize - half as isize;
        let x = offset as f64;
        let ideal = if offset == 0 {
            2.0 * cutoff
        } else {
            (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
        };
        let phase = x / half as f64;
        let window = 0.42
            + 0.5 * (std::f64::consts::PI * phase).cos()
            + 0.08 * (std::f64::consts::TAU * phase).cos();
        kernel.push(ideal * window);
    }
    let kernel_sum = kernel.iter().sum::<f64>();
    for coefficient in &mut kernel {
        *coefficient /= kernel_sum;
    }

    let output_frames = source.len().div_ceil(stride);
    let mut output = Vec::with_capacity(output_frames);
    for output_index in 0..output_frames {
        if output_index & 255 == 0 && should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = output_index * stride;
        let mut sample = 0.0_f64;
        for (tap, coefficient) in kernel.iter().copied().enumerate() {
            let offset = tap as isize - half as isize;
            let index = reflected_index(center as isize + offset, source.len());
            sample += f64::from(source[index]) * coefficient;
        }
        output.push(sample as f32);
    }
    Ok(output)
}

pub(super) fn reflected_index(index: isize, length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let maximum = length as isize - 1;
    let period = maximum * 2;
    let folded = index.rem_euclid(period);
    if folded <= maximum {
        folded as usize
    } else {
        (period - folded) as usize
    }
}

#[derive(Clone)]
pub(super) struct PeriodicMipLevel {
    pub(super) samples: Box<[f32]>,
    pub(super) integral: Box<[f64]>,
}

#[derive(Clone)]
pub(super) struct ReflectedMipLevel {
    pub(super) samples: Box<[f32]>,
}

#[derive(Clone)]
pub(super) struct OneShotMipLevel {
    pub(super) samples: Box<[f32]>,
    pub(super) integral_blocks: Box<[f64]>,
}

#[derive(Clone, Copy)]
pub(super) enum MipBoundary {
    Periodic,
    Reflected,
    Zero,
}

pub(super) fn validate_source(source: &[f32]) -> Result<(), ArtifactBuildError> {
    if source.is_empty() {
        return Err(ArtifactBuildError::Empty);
    }
    if source
        .iter()
        .any(|sample| !sample.is_finite() || sample.abs() > MAX_SOURCE_ABS_SAMPLE)
    {
        return Err(ArtifactBuildError::NonFinite);
    }
    Ok(())
}

pub(super) fn best_periodic_start(
    source: &[f32],
    nominal: usize,
    start_limit: usize,
    loop_frames: usize,
    period: f32,
) -> usize {
    let radius = ((period * 2.0).round() as usize).clamp(8, 2_048);
    let lo = nominal.saturating_sub(radius);
    let hi = nominal.saturating_add(radius).min(start_limit);
    (lo..=hi)
        .min_by(|&a, &b| {
            seam_cost(source, a, loop_frames).total_cmp(&seam_cost(source, b, loop_frames))
        })
        .unwrap_or(nominal.min(start_limit))
}

pub(super) fn seam_cost(source: &[f32], start: usize, length: usize) -> f32 {
    let end = (start + length).min(source.len() - 1);
    let start_next = (start + 1).min(source.len() - 1);
    let end_prev = end.saturating_sub(1);
    (source[start] - source[end]).abs()
        + 0.5 * ((source[start_next] - source[start]) - (source[end] - source[end_prev])).abs()
}

pub(in crate::oscillators) fn remove_dc_and_peak_normalize(samples: &mut [f32]) {
    let mean =
        samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len().max(1) as f64;
    let peak = samples
        .iter()
        .map(|sample| (f64::from(*sample) - mean).abs())
        .fold(0.0_f64, f64::max);
    let gain = if peak > 1.0e-12 { peak.recip() } else { 1.0 };
    for sample in samples.iter_mut() {
        *sample = ((f64::from(*sample) - mean) * gain).clamp(-1.0, 1.0) as f32;
    }
    debug_assert!(
        samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
    );
}

pub(in crate::oscillators) fn remove_dc_and_stereo_peak_normalize(
    mid: &mut [f32],
    side: &mut [f32],
) {
    debug_assert_eq!(mid.len(), side.len());
    let mean = |samples: &[f32]| {
        samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len().max(1) as f64
    };
    let mid_mean = mean(mid);
    let side_mean = mean(side);
    let peak = mid
        .iter()
        .zip(side.iter())
        .map(|(mid, side)| {
            let mid = f64::from(*mid) - mid_mean;
            let side = f64::from(*side) - side_mean;
            (mid + side).abs().max((mid - side).abs())
        })
        .fold(0.0_f64, f64::max);
    let gain = if peak > 1.0e-12 { peak.recip() } else { 1.0 };
    for (mid, side) in mid.iter_mut().zip(side.iter_mut()) {
        *mid = ((f64::from(*mid) - mid_mean) * gain) as f32;
        *side = ((f64::from(*side) - side_mean) * gain) as f32;
    }
}

pub(super) fn build_periodic_mips(base: &[f32]) -> Box<[PeriodicMipLevel]> {
    build_periodic_mips_with_cancel(base, &|| false)
        .expect("non-cancelling periodic mip generation")
}

pub(super) fn build_periodic_mips_with_cancel(
    base: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[PeriodicMipLevel]>, ArtifactBuildError> {
    let mut levels = Vec::new();
    let mut current = base.to_vec();
    while current.len() > 2 {
        let next = prefilter_half_with_cancel(&current, MipBoundary::Periodic, should_cancel)?;
        let integral = periodic_integral_prefix_with_cancel(&next, should_cancel)?;
        levels.push(PeriodicMipLevel {
            samples: next.clone().into_boxed_slice(),
            integral,
        });
        current = next;
    }
    Ok(levels.into_boxed_slice())
}

pub(super) fn build_reflected_mips(base: &[f32]) -> Box<[ReflectedMipLevel]> {
    build_reflected_mips_with_cancel(base, &|| false)
        .expect("non-cancelling reflected mip generation")
}

pub(super) fn build_reflected_mips_with_cancel(
    base: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[ReflectedMipLevel]>, ArtifactBuildError> {
    let mut levels = Vec::new();
    let mut current = base.to_vec();
    while current.len() > 2 {
        let next = prefilter_half_with_cancel(&current, MipBoundary::Reflected, should_cancel)?;
        levels.push(ReflectedMipLevel {
            samples: next.clone().into_boxed_slice(),
        });
        current = next;
    }
    Ok(levels.into_boxed_slice())
}

pub(super) fn build_one_shot_mips_with_cancel(
    base: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[OneShotMipLevel]>, ArtifactBuildError> {
    let mut levels = Vec::new();
    let mut current = base.to_vec();
    while current.len() > 2 {
        let next = prefilter_half_with_cancel(&current, MipBoundary::Zero, should_cancel)?;
        let integral_blocks = one_shot_integral_blocks_with_cancel(&next, should_cancel)?;
        levels.push(OneShotMipLevel {
            samples: next.clone().into_boxed_slice(),
            integral_blocks,
        });
        current = next;
    }
    Ok(levels.into_boxed_slice())
}

/// Worker-side true low-pass halving. Every output level is at most half the
/// previous length, so all derived PCM combined is smaller than the base.
pub(super) fn prefilter_half_with_cancel(
    source: &[f32],
    boundary: MipBoundary,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    if should_cancel() {
        return Err(ArtifactBuildError::Cancelled);
    }
    let output_len = (source.len() / 2).max(1);
    let filtered_at = |center: isize| {
        let mut sum = 0.0_f64;
        for (tap, coefficient) in MIP_HALF_COEFFICIENTS.iter().copied().enumerate() {
            let index = center + tap as isize - MIP_HALF_TAPS;
            let sample = match boundary {
                MipBoundary::Periodic => source[index.rem_euclid(source.len() as isize) as usize],
                MipBoundary::Reflected => source[reflected_index(index, source.len())],
                MipBoundary::Zero => {
                    if (0..source.len() as isize).contains(&index) {
                        source[index as usize]
                    } else {
                        0.0
                    }
                }
            };
            sum = coefficient.mul_add(f64::from(sample), sum);
        }
        sum
    };
    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        if output_index & 255 == 0 && should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        // The immutable level is uniformly spaced across the authoritative
        // periodic duration, reflected endpoint span, or one-shot duration.
        // Interpolating two integer-centred fixed-kernel results avoids both
        // per-output transcendental work and odd-length coordinate drift.
        let (numerator, denominator) = match boundary {
            MipBoundary::Reflected if output_len > 1 => {
                (output_index * (source.len() - 1), output_len - 1)
            }
            _ => (output_index * source.len(), output_len),
        };
        let first = numerator / denominator;
        let fraction = (numerator % denominator) as f64 / denominator as f64;
        let first_sample = filtered_at(first as isize);
        let sample = if fraction == 0.0 {
            first_sample
        } else {
            let second_sample = filtered_at(first as isize + 1);
            (second_sample - first_sample).mul_add(fraction, first_sample)
        };
        output.push(sample as f32);
    }
    Ok(output)
}

pub(super) fn periodic_integral_prefix(samples: &[f32]) -> Box<[f64]> {
    let mut prefix = Vec::with_capacity(samples.len() + 1);
    prefix.push(0.0);
    let mut integral = 0.0_f64;
    for index in 0..samples.len() {
        let next = (index + 1) % samples.len().max(1);
        integral += 0.5
            * (f64::from(samples[index])
                + f64::from(samples.get(next).copied().unwrap_or(samples[index])));
        prefix.push(integral);
    }
    prefix.into_boxed_slice()
}

#[cfg(test)]
pub(super) fn reflected_integral_prefix(samples: &[f32]) -> Box<[f64]> {
    let mut prefix = Vec::with_capacity(samples.len().max(1));
    prefix.push(0.0);
    let mut integral = 0.0_f64;
    for pair in samples.windows(2) {
        integral += 0.5 * (f64::from(pair[0]) + f64::from(pair[1]));
        prefix.push(integral);
    }
    prefix.into_boxed_slice()
}

pub(super) fn periodic_integral_prefix_with_cancel(
    samples: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[f64]>, ArtifactBuildError> {
    let mut prefix = Vec::with_capacity(samples.len() + 1);
    prefix.push(0.0);
    let mut integral = 0.0_f64;
    for index in 0..samples.len() {
        if index & 255 == 0 && should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let next = (index + 1) % samples.len().max(1);
        integral += 0.5 * (f64::from(samples[index]) + f64::from(samples[next]));
        prefix.push(integral);
    }
    Ok(prefix.into_boxed_slice())
}

#[cfg(test)]
pub(super) fn reflected_integral_prefix_with_cancel(
    samples: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[f64]>, ArtifactBuildError> {
    let mut prefix = Vec::with_capacity(samples.len().max(1));
    prefix.push(0.0);
    let mut integral = 0.0_f64;
    for (index, pair) in samples.windows(2).enumerate() {
        if index & 255 == 0 && should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        integral += 0.5 * (f64::from(pair[0]) + f64::from(pair[1]));
        prefix.push(integral);
    }
    Ok(prefix.into_boxed_slice())
}

pub(super) fn one_shot_integral_blocks_with_cancel(
    samples: &[f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Box<[f64]>, ArtifactBuildError> {
    let block_count = samples.len().div_ceil(SOURCE_INTEGRAL_BLOCK_FRAMES);
    let mut blocks = Vec::with_capacity(block_count + 1);
    blocks.push(0.0);
    let mut integral = 0.0_f64;
    for index in 0..samples.len() {
        if index & 255 == 0 && should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let next = samples.get(index + 1).copied().unwrap_or(samples[index]);
        integral += 0.5 * (f64::from(samples[index]) + f64::from(next));
        if (index + 1) % SOURCE_INTEGRAL_BLOCK_FRAMES == 0 {
            blocks.push(integral);
        }
    }
    if samples.len() % SOURCE_INTEGRAL_BLOCK_FRAMES != 0 {
        blocks.push(integral);
    }
    Ok(blocks.into_boxed_slice())
}

#[inline]
pub(super) fn segment_integral(first: f32, second: f32, fraction: f64) -> f64 {
    f64::from(first) * fraction + 0.5 * f64::from(second - first) * fraction * fraction
}

#[inline]
pub(super) fn periodic_antiderivative(samples: &[f32], prefix: &[f64], position: f64) -> f64 {
    let length = samples.len();
    if length == 0 {
        return 0.0;
    }
    if length == 1 {
        return position * f64::from(samples[0]);
    }
    let length_f64 = length as f64;
    let cycles = (position / length_f64).floor();
    let remainder = position - cycles * length_f64;
    let index = (remainder.floor() as usize).min(length - 1);
    let fraction = remainder - index as f64;
    cycles * prefix[length]
        + prefix[index]
        + segment_integral(samples[index], samples[(index + 1) % length], fraction)
}

#[inline]
#[cfg(test)]
pub(super) fn reflected_base_antiderivative(samples: &[f32], prefix: &[f64], position: f64) -> f64 {
    let maximum = samples.len().saturating_sub(1);
    if maximum == 0 {
        return position * f64::from(samples.first().copied().unwrap_or(0.0));
    }
    let position = position.clamp(0.0, maximum as f64);
    let index = (position.floor() as usize).min(maximum);
    if index == maximum {
        return prefix[maximum];
    }
    prefix[index] + segment_integral(samples[index], samples[index + 1], position - index as f64)
}

#[inline]
#[cfg(test)]
pub(super) fn reflected_antiderivative(samples: &[f32], prefix: &[f64], position: f64) -> f64 {
    let maximum = samples.len().saturating_sub(1) as f64;
    if maximum <= 0.0 {
        return position * f64::from(samples.first().copied().unwrap_or(0.0));
    }
    let period = maximum * 2.0;
    let cycles = (position / period).floor();
    let remainder = position - cycles * period;
    let forward_total = prefix[samples.len() - 1];
    let partial = if remainder <= maximum {
        reflected_base_antiderivative(samples, prefix, remainder)
    } else {
        2.0 * forward_total - reflected_base_antiderivative(samples, prefix, period - remainder)
    };
    cycles * (2.0 * forward_total) + partial
}

#[inline]
pub(super) fn one_shot_antiderivative(samples: &[f32], blocks: &[f64], position: f64) -> f64 {
    if position <= 0.0 || samples.is_empty() {
        return 0.0;
    }
    if position >= samples.len() as f64 {
        return blocks.last().copied().unwrap_or(0.0);
    }
    let whole = position.floor() as usize;
    let block = whole / SOURCE_INTEGRAL_BLOCK_FRAMES;
    let start = block * SOURCE_INTEGRAL_BLOCK_FRAMES;
    let mut integral = blocks[block];
    for index in start..whole {
        let next = samples.get(index + 1).copied().unwrap_or(samples[index]);
        integral += 0.5 * (f64::from(samples[index]) + f64::from(next));
    }
    let next = samples.get(whole + 1).copied().unwrap_or(samples[whole]);
    integral + segment_integral(samples[whole], next, position - whole as f64)
}

const RT_AA_TAPS: usize = 16;
const RT_INTEGRATED_AA_TAPS: usize = 25;
// Offline-designed Kaiser(beta=8) windowed-sinc low-pass. The interval sample
// rate is `2 / source_step`, so the 0.22 cycles/interval cutoff retains 88% of
// the destination Nyquist band while giving the fixed 25-tap kernel transition
// room. Exact adjacent segment integrals provide the anti-aliased tap values.
const RT_INTEGRATED_AA_COEFFICIENTS: [f64; RT_INTEGRATED_AA_TAPS] = [
    -4.780_226_922_748_594_11e-05,
    1.869_583_142_847_227_39e-04,
    1.155_867_651_607_545_57e-03,
    -3.667_660_956_098_452_14e-04,
    -6.036_389_254_995_292_75e-03,
    -2.805_824_024_883_537_67e-03,
    1.771_166_903_651_855_83e-02,
    1.899_492_957_814_575_76e-02,
    -3.554_353_105_700_309_30e-02,
    -7.066_128_310_725_201_81e-02,
    5.277_050_739_270_069_96e-02,
    3.046_412_718_019_662_62e-01,
    4.400_007_840_674_953_21e-01,
    3.046_412_718_019_662_62e-01,
    5.277_050_739_270_069_96e-02,
    -7.066_128_310_725_201_81e-02,
    -3.554_353_105_700_309_30e-02,
    1.899_492_957_814_575_76e-02,
    1.771_166_903_651_855_83e-02,
    -2.805_824_024_883_537_67e-03,
    -6.036_389_254_995_292_75e-03,
    -3.667_660_956_098_452_14e-04,
    1.155_867_651_607_545_57e-03,
    1.869_583_142_847_227_39e-04,
    -4.780_226_922_748_594_11e-05,
];

#[inline]
pub(super) fn reflected_position(position: f32, maximum: f32) -> f32 {
    if maximum <= 0.0 {
        return 0.0;
    }
    if (0.0..=maximum).contains(&position) {
        return position;
    }
    let period = maximum * 2.0;
    let folded = position.rem_euclid(period);
    if folded <= maximum {
        folded
    } else {
        period - folded
    }
}

#[inline]
pub(super) fn mip_floor_level(rate: f32, maximum_level: usize) -> usize {
    if rate < 2.0 {
        return 0;
    }
    let exponent = ((rate.to_bits() >> 23) & 0xff) as usize;
    exponent.saturating_sub(127).min(maximum_level)
}

#[inline]
pub(super) fn periodic_mip_sample(
    base: &[f32],
    base_integral: &[f64],
    mips: &[PeriodicMipLevel],
    phase: f32,
    source_step: f32,
) -> f32 {
    let rate = source_step.abs();
    if !rate.is_finite() {
        return 0.0;
    }
    let level = mip_floor_level(rate, mips.len());
    let evaluate = |level: usize| {
        if level == 0 {
            periodic_integrated_sample(base, base_integral, phase, rate)
        } else {
            let mip = &mips[level - 1];
            let residual = rate * mip.samples.len() as f32 / base.len().max(1) as f32;
            periodic_integrated_sample(&mip.samples, &mip.integral, phase, residual)
        }
    };
    let fine = evaluate(level);
    if level >= mips.len() {
        return fine;
    }
    let fine_len = if level == 0 {
        base.len()
    } else {
        mips[level - 1].samples.len()
    };
    let residual = rate * fine_len as f32 / base.len().max(1) as f32;
    let blend =
        ((residual - MIP_BLEND_START_RESIDUAL) / (2.0 - MIP_BLEND_START_RESIDUAL)).clamp(0.0, 1.0);
    if blend <= 0.0 {
        fine
    } else {
        let coarse = evaluate(level + 1);
        (coarse - fine).mul_add(blend, fine)
    }
}

#[inline]
pub(super) fn reflected_mip_sample(
    base: &[f32],
    mips: &[ReflectedMipLevel],
    position: f32,
    source_step: f32,
) -> f32 {
    let rate = source_step.abs();
    if !rate.is_finite() {
        return 0.0;
    }
    let level = mip_floor_level(rate, mips.len());
    let base_span = base.len().saturating_sub(1).max(1) as f32;
    let evaluate = |level: usize| {
        if level == 0 {
            reflected_cubic(base, position)
        } else {
            let mip = &mips[level - 1];
            let scale = mip.samples.len().saturating_sub(1) as f32 / base_span;
            reflected_cubic(&mip.samples, position * scale)
        }
    };
    let fine = evaluate(level);
    if level >= mips.len() {
        return fine;
    }
    let fine_scale = if level == 0 {
        1.0
    } else {
        mips[level - 1].samples.len().saturating_sub(1) as f32 / base_span
    };
    let residual = rate * fine_scale;
    let blend =
        ((residual - MIP_BLEND_START_RESIDUAL) / (2.0 - MIP_BLEND_START_RESIDUAL)).clamp(0.0, 1.0);
    if blend <= 0.0 {
        fine
    } else {
        let coarse = evaluate(level + 1);
        (coarse - fine).mul_add(blend, fine)
    }
}

#[inline]
pub(super) fn reflected_cubic(samples: &[f32], position: f32) -> f32 {
    if samples.len() < 4 || !position.is_finite() {
        return reflected_linear(samples, position);
    }
    let position = reflected_position(position, samples.len().saturating_sub(1) as f32);
    let floor = position.floor();
    let index = floor as isize;
    let fraction = position - floor;
    let (y0, y1, y2, y3) = if index >= 1 && index as usize + 2 < samples.len() {
        let index = index as usize;
        (
            samples[index - 1],
            samples[index],
            samples[index + 1],
            samples[index + 2],
        )
    } else {
        (
            samples[reflected_index(index - 1, samples.len())],
            samples[reflected_index(index, samples.len())],
            samples[reflected_index(index + 1, samples.len())],
            samples[reflected_index(index + 2, samples.len())],
        )
    };
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * fraction + c2) * fraction + c1).mul_add(fraction, c0)
}

#[inline]
pub(super) fn one_shot_mip_sample(
    base: &[f32],
    base_integral: &[f64],
    mips: &[OneShotMipLevel],
    position: f64,
    source_step: f32,
) -> f32 {
    if !position.is_finite() || position < 0.0 || position >= base.len() as f64 {
        return 0.0;
    }
    let rate = source_step.abs();
    if !rate.is_finite() {
        return 0.0;
    }
    let level = mip_floor_level(rate, mips.len());
    let base_len = base.len().max(1) as f64;
    let evaluate = |level: usize| {
        if level == 0 {
            integrated_one_shot_sample(base, base_integral, position, rate)
        } else {
            let mip = &mips[level - 1];
            let scale = mip.samples.len() as f64 / base_len;
            integrated_one_shot_sample(
                &mip.samples,
                &mip.integral_blocks,
                position * scale,
                rate * scale as f32,
            )
        }
    };
    let fine = evaluate(level);
    if level >= mips.len() {
        return fine;
    }
    let fine_scale = if level == 0 {
        1.0
    } else {
        mips[level - 1].samples.len() as f32 / base.len().max(1) as f32
    };
    let residual = rate * fine_scale;
    let blend =
        ((residual - MIP_BLEND_START_RESIDUAL) / (2.0 - MIP_BLEND_START_RESIDUAL)).clamp(0.0, 1.0);
    if blend <= 0.0 {
        fine
    } else {
        let coarse = evaluate(level + 1);
        (coarse - fine).mul_add(blend, fine)
    }
}

#[inline]
pub(super) fn integrated_one_shot_sample(
    samples: &[f32],
    integral_blocks: &[f64],
    position: f64,
    rate: f32,
) -> f32 {
    if rate <= 0.9 {
        if position < 0.0 || position >= samples.len() as f64 {
            return 0.0;
        }
        let first = position.floor() as usize;
        let second = (first + 1).min(samples.len() - 1);
        let mix = (position - first as f64) as f32;
        return (samples[second] - samples[first]).mul_add(mix, samples[first]);
    }
    let interval_width = f64::from(rate) * 0.5;
    let inverse_width = interval_width.recip();
    let mut boundary = position - interval_width * (RT_INTEGRATED_AA_TAPS as f64 * 0.5);
    let mut before = one_shot_antiderivative(samples, integral_blocks, boundary);
    let mut output = 0.0_f64;
    for coefficient in RT_INTEGRATED_AA_COEFFICIENTS {
        boundary += interval_width;
        let after = one_shot_antiderivative(samples, integral_blocks, boundary);
        output = coefficient.mul_add((after - before) * inverse_width, output);
        before = after;
    }
    let filtered = output as f32;
    if rate < 1.0 {
        if position < 0.0 || position >= samples.len() as f64 {
            return 0.0;
        }
        let first = position.floor() as usize;
        let second = (first + 1).min(samples.len() - 1);
        let mix = (position - first as f64) as f32;
        let direct = (samples[second] - samples[first]).mul_add(mix, samples[first]);
        (filtered - direct).mul_add((rate - 0.9) * 10.0, direct)
    } else {
        filtered
    }
}

#[inline]
#[cfg(test)]
pub(super) fn grain_antialiased_sample(
    samples: &[f32],
    integral: &[f64],
    position: f32,
    source_step: f32,
) -> f32 {
    let rate = source_step.abs();
    if rate <= 0.9 {
        return reflected_linear(samples, position);
    }
    if !rate.is_finite() {
        return 0.0;
    }
    let interval_width = f64::from(rate) * 0.5;
    let inverse_width = interval_width.recip();
    let center = f64::from(position);
    let mut boundary = center - interval_width * (RT_INTEGRATED_AA_TAPS as f64 * 0.5);
    let mut before = reflected_antiderivative(samples, integral, boundary);
    let mut output = 0.0_f64;
    for coefficient in RT_INTEGRATED_AA_COEFFICIENTS {
        boundary += interval_width;
        let after = reflected_antiderivative(samples, integral, boundary);
        let average = (after - before) * inverse_width;
        output = coefficient.mul_add(average, output);
        before = after;
    }
    let filtered = output as f32;
    if rate < 1.0 {
        let direct = reflected_linear(samples, position);
        (filtered - direct).mul_add((rate - 0.9) * 10.0, direct)
    } else {
        filtered
    }
}

#[inline]
pub(super) fn reflected_linear(samples: &[f32], position: f32) -> f32 {
    let position = reflected_position(position, samples.len().saturating_sub(1) as f32);
    bounded_linear(samples, position)
}

#[inline]
pub(super) fn bounded_linear(samples: &[f32], position: f32) -> f32 {
    let position = position.clamp(0.0, samples.len().saturating_sub(1) as f32);
    let first = position.floor() as usize;
    let second = (first + 1).min(samples.len() - 1);
    let mix = position - first as f32;
    (samples[second] - samples[first]).mul_add(mix, samples[first])
}

#[inline]
pub(super) fn periodic_linear(samples: &[f32], position: f32) -> f32 {
    let length = samples.len();
    if length < 2 {
        return samples.first().copied().unwrap_or(0.0);
    }
    let position = position.rem_euclid(length as f32);
    let first = position.floor() as usize;
    let second = (first + 1) % length;
    let mix = position - first as f32;
    (samples[second] - samples[first]).mul_add(mix, samples[first])
}

#[inline]
pub(super) fn periodic_integrated_sample(
    samples: &[f32],
    integral: &[f64],
    phase: f32,
    source_step: f32,
) -> f32 {
    let rate = source_step.abs();
    if rate <= 0.9 {
        return periodic_cubic(samples, phase);
    }
    if !rate.is_finite() {
        return 0.0;
    }
    let interval_width = f64::from(rate) * 0.5;
    let inverse_width = interval_width.recip();
    let center = f64::from(phase.rem_euclid(1.0)) * samples.len() as f64;
    let mut boundary = center - interval_width * (RT_INTEGRATED_AA_TAPS as f64 * 0.5);
    let mut before = periodic_antiderivative(samples, integral, boundary);
    let mut output = 0.0_f64;
    for coefficient in RT_INTEGRATED_AA_COEFFICIENTS {
        boundary += interval_width;
        let after = periodic_antiderivative(samples, integral, boundary);
        let average = (after - before) * inverse_width;
        output = coefficient.mul_add(average, output);
        before = after;
    }
    let filtered = output as f32;
    if rate < 1.0 {
        let direct = periodic_cubic(samples, phase);
        (filtered - direct).mul_add((rate - 0.9) * 10.0, direct)
    } else {
        filtered
    }
}

#[inline]
pub(super) fn periodic_antialiased_sample(samples: &[f32], phase: f32, source_step: f32) -> f32 {
    let rate = source_step.abs();
    if rate <= 1.0 {
        return periodic_cubic(samples, phase);
    }
    if !rate.is_finite() {
        return 0.0;
    }
    let center = phase.rem_euclid(1.0) * samples.len() as f32;
    let spacing = rate / RT_AA_TAPS as f32;
    let first = center - 0.5 * rate + 0.5 * spacing;
    let mut sum = 0.0_f32;
    for tap in 0..RT_AA_TAPS {
        sum += periodic_linear(samples, first + tap as f32 * spacing);
    }
    sum / RT_AA_TAPS as f32
}

#[inline]
pub(super) fn periodic_cubic(samples: &[f32], phase: f32) -> f32 {
    let length = samples.len();
    if length < 4 {
        return samples.first().copied().unwrap_or(0.0);
    }
    let position = phase.rem_euclid(1.0) * length as f32;
    let index = position.floor() as usize % length;
    let fraction = position - position.floor();
    let y0 = samples[(index + length - 1) % length];
    let y1 = samples[index];
    let y2 = samples[(index + 1) % length];
    let y3 = samples[(index + 2) % length];
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * fraction + c2) * fraction + c1).mul_add(fraction, c0)
}
