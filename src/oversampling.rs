//! Fixed-allocation dynamic synthesis oversampling.

use truce_simd::simd::f32x8;

pub const DEFAULT_FACTOR: u8 = 2;
pub const LATENCY_SAMPLES: u32 = 33;
pub const TAIL_SAMPLES: u8 = 67;

const HOST_LATENCY: usize = LATENCY_SAMPLES as usize;
const MAX_TAPS: usize = 193;
const BUFFER: usize = MAX_TAPS * 2;
const POST_FILTER_DELAY: usize = 7;
const PASSBAND_EQ_SIDE: f32 = -0.017_88;
const PASSBAND_EQ_CENTER: f32 = 1.0 - 2.0 * PASSBAND_EQ_SIDE;
const SPLINE_EQ_OUTER: f32 = 0.017_700_59;
const SPLINE_EQ_SIDE: f32 = -0.099_797_11;
const SPLINE_EQ_CENTER: f32 = 1.164_193_04;

pub struct StereoOversampler {
    x2: StereoDecimator<97, 12>,
    x3: StereoDecimator<145, 18>,
    x4: StereoDecimator<193, 24>,
    direct_delay: StereoDelay,
    direct_output: (f32, f32),
    factor: u8,
    spline_correction_mix: f32,
}

impl Default for StereoOversampler {
    fn default() -> Self {
        Self {
            x2: StereoDecimator::new(2),
            x3: StereoDecimator::new(3),
            x4: StereoDecimator::new(4),
            direct_delay: StereoDelay::default(),
            direct_output: (0.0, 0.0),
            factor: DEFAULT_FACTOR,
            spline_correction_mix: 0.0,
        }
    }
}

impl StereoOversampler {
    pub const fn factor(&self) -> u8 {
        self.factor
    }

    pub fn reset(&mut self, factor: u8) {
        self.x2.reset();
        self.x3.reset();
        self.x4.reset();
        self.direct_delay.reset();
        self.direct_output = (0.0, 0.0);
        self.factor = factor.clamp(1, 4);
        self.spline_correction_mix = 0.0;
    }

    pub const fn push(&mut self, left: f32, right: f32) {
        match self.factor {
            1 => self.direct_output = self.direct_delay.process(left, right),
            2 => self.x2.push(left, right),
            3 => self.x3.push(left, right),
            _ => self.x4.push(left, right),
        }
    }

    pub fn process_direct(&mut self, left: f32, right: f32) -> (f32, f32) {
        debug_assert_eq!(self.factor, 1);
        self.direct_output = self.direct_delay.process(left, right);
        self.direct_output
    }

    pub fn set_spline_correction(&mut self, enabled: bool) {
        self.spline_correction_mix = f32::from(u8::from(enabled));
    }

    pub fn set_spline_correction_immediate(&mut self, enabled: bool) {
        self.set_spline_correction(enabled);
    }

    pub fn output(&mut self) -> (f32, f32) {
        match self.factor {
            1 => self.direct_output,
            2 => self.x2.output_with_spline_mix(self.spline_correction_mix),
            3 => self.x3.output(),
            _ => self.x4.output(),
        }
    }
}

struct StereoDelay {
    left: [f32; HOST_LATENCY],
    right: [f32; HOST_LATENCY],
    write: usize,
}

impl Default for StereoDelay {
    fn default() -> Self {
        Self {
            left: [0.0; HOST_LATENCY],
            right: [0.0; HOST_LATENCY],
            write: 0,
        }
    }
}

impl StereoDelay {
    fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write = 0;
    }

    const fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let output = (self.left[self.write], self.right[self.write]);
        self.left[self.write] = left;
        self.right[self.write] = right;
        self.write += 1;
        if self.write == HOST_LATENCY {
            self.write = 0;
        }
        output
    }
}

struct StereoDecimator<const TAPS: usize, const BLOCKS: usize> {
    coefficient_blocks: [f32x8; BLOCKS],
    tail_coefficient: f32,
    left: [f32; BUFFER],
    right: [f32; BUFFER],
    write: usize,
    equalizer_left: [f32; 4],
    equalizer_right: [f32; 4],
    delay_left: [f32; POST_FILTER_DELAY],
    delay_right: [f32; POST_FILTER_DELAY],
    delay_write: usize,
}

impl<const TAPS: usize, const BLOCKS: usize> StereoDecimator<TAPS, BLOCKS> {
    fn new(factor: u8) -> Self {
        let taps = factor_taps(factor);
        debug_assert_eq!(TAPS, taps);
        debug_assert_eq!(BLOCKS, (TAPS - 1) / 8);
        let coefficients = equiripple_filter(factor);
        Self {
            coefficient_blocks: std::array::from_fn(|block| {
                f32x8::from(std::array::from_fn(|lane| coefficients[block * 8 + lane]))
            }),
            tail_coefficient: coefficients[TAPS - 1],
            left: [0.0; BUFFER],
            right: [0.0; BUFFER],
            write: 0,
            equalizer_left: [0.0; 4],
            equalizer_right: [0.0; 4],
            delay_left: [0.0; POST_FILTER_DELAY],
            delay_right: [0.0; POST_FILTER_DELAY],
            delay_write: 0,
        }
    }

    fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write = 0;
        self.equalizer_left.fill(0.0);
        self.equalizer_right.fill(0.0);
        self.delay_left.fill(0.0);
        self.delay_right.fill(0.0);
        self.delay_write = 0;
    }

    const fn push(&mut self, left: f32, right: f32) {
        self.left[self.write] = left;
        self.left[self.write + TAPS] = left;
        self.right[self.write] = right;
        self.right[self.write + TAPS] = right;
        self.write += 1;
        if self.write == TAPS {
            self.write = 0;
        }
    }

    fn output(&mut self) -> (f32, f32) {
        self.output_with_spline_mix(0.0)
    }

    fn output_with_spline_mix(&mut self, spline_mix: f32) -> (f32, f32) {
        let mut left_a = f32x8::ZERO;
        let mut left_b = f32x8::ZERO;
        let mut right_a = f32x8::ZERO;
        let mut right_b = f32x8::ZERO;
        let mut block = 0;
        while block + 2 <= BLOCKS {
            let index = block * 8;
            let coefficients_a = self.coefficient_blocks[block];
            let coefficients_b = self.coefficient_blocks[block + 1];
            let left_samples_a =
                f32x8::from(*self.left[self.write + index..].first_chunk().unwrap());
            let left_samples_b =
                f32x8::from(*self.left[self.write + index + 8..].first_chunk().unwrap());
            let right_samples_a =
                f32x8::from(*self.right[self.write + index..].first_chunk().unwrap());
            let right_samples_b =
                f32x8::from(*self.right[self.write + index + 8..].first_chunk().unwrap());
            left_a = left_samples_a.mul_add(coefficients_a, left_a);
            left_b = left_samples_b.mul_add(coefficients_b, left_b);
            right_a = right_samples_a.mul_add(coefficients_a, right_a);
            right_b = right_samples_b.mul_add(coefficients_b, right_b);
            block += 2;
        }
        let mut left = (left_a + left_b).reduce_add();
        let mut right = (right_a + right_b).reduce_add();
        let index = TAPS - 1;
        left = self.left[self.write + index].mul_add(self.tail_coefficient, left);
        right = self.right[self.write + index].mul_add(self.tail_coefficient, right);
        let (equalized_left, equalized_right) = if spline_mix == 0.0 {
            (
                PASSBAND_EQ_SIDE.mul_add(
                    self.equalizer_left[0] + self.equalizer_left[2],
                    PASSBAND_EQ_CENTER * self.equalizer_left[1],
                ),
                PASSBAND_EQ_SIDE.mul_add(
                    self.equalizer_right[0] + self.equalizer_right[2],
                    PASSBAND_EQ_CENTER * self.equalizer_right[1],
                ),
            )
        } else {
            let (outer, side, center) = if spline_mix == 1.0 {
                (SPLINE_EQ_OUTER, SPLINE_EQ_SIDE, SPLINE_EQ_CENTER)
            } else {
                (
                    SPLINE_EQ_OUTER * spline_mix,
                    (SPLINE_EQ_SIDE - PASSBAND_EQ_SIDE).mul_add(spline_mix, PASSBAND_EQ_SIDE),
                    (SPLINE_EQ_CENTER - PASSBAND_EQ_CENTER).mul_add(spline_mix, PASSBAND_EQ_CENTER),
                )
            };
            (
                outer.mul_add(
                    left + self.equalizer_left[3],
                    side.mul_add(
                        self.equalizer_left[0] + self.equalizer_left[2],
                        center * self.equalizer_left[1],
                    ),
                ),
                outer.mul_add(
                    right + self.equalizer_right[3],
                    side.mul_add(
                        self.equalizer_right[0] + self.equalizer_right[2],
                        center * self.equalizer_right[1],
                    ),
                ),
            )
        };
        self.equalizer_left = [
            left,
            self.equalizer_left[0],
            self.equalizer_left[1],
            self.equalizer_left[2],
        ];
        self.equalizer_right = [
            right,
            self.equalizer_right[0],
            self.equalizer_right[1],
            self.equalizer_right[2],
        ];
        let output = (
            self.delay_left[self.delay_write],
            self.delay_right[self.delay_write],
        );
        self.delay_left[self.delay_write] = equalized_left;
        self.delay_right[self.delay_write] = equalized_right;
        self.delay_write += 1;
        if self.delay_write == POST_FILTER_DELAY {
            self.delay_write = 0;
        }
        output
    }
}

const fn factor_taps(factor: u8) -> usize {
    match factor {
        2 => 97,
        3 => 145,
        _ => 193,
    }
}

fn equiripple_filter(factor: u8) -> [f32; MAX_TAPS] {
    match factor {
        2 => symmetric_filter(&EQUIRIPPLE_2X_HALF, factor_taps(2)),
        3 => symmetric_filter(&EQUIRIPPLE_3X_HALF, factor_taps(3)),
        _ => symmetric_filter(&EQUIRIPPLE_4X_HALF, factor_taps(4)),
    }
}

fn symmetric_filter<const N: usize>(half: &[f32; N], taps: usize) -> [f32; MAX_TAPS] {
    let mut coefficients = [0.0; MAX_TAPS];
    for (index, coefficient) in half.iter().copied().enumerate() {
        coefficients[index] = coefficient;
        coefficients[taps - 1 - index] = coefficient;
    }
    coefficients
}

// Parks-McClellan linear-phase kernels, specified at a 48 kHz HOST rate:
// 0-20.5 kHz passband, 24 kHz stopband. These fixed normalized coefficients
// scale those edges with host rate (18.835/22.05 kHz at 44.1 kHz host),
// and 100:1 stopband weighting at each internal sample rate. Only one symmetric
// half is stored; the complete filter is assembled during plugin initialization.
#[rustfmt::skip]
#[allow(clippy::excessive_precision, reason = "offline-designed coefficients are rounded by the f32 array type")]
const EQUIRIPPLE_2X_HALF: [f32; 49] = [
    2.141_617_1e-4, 9.156_493_6e-4, 1.523_236e-3, 1.091_261_9e-3, -3.496_221_2e-4,
    -1.099_877_8e-3, -4.312_857e-5, 1.254_656_4e-3, 5.076_255_4e-4, -1.368_944_4e-3,
    -1.138_129_5e-3, 1.301_600_3e-3, 1.898_813_8e-3, -9.451_314e-4, -2.700_823_6e-3,
    2.239_714e-4, 3.414_615_3e-3, 8.976_306_4e-4, -3.880_063_4e-3, -2.403_074_6e-3,
    3.920_137e-3, 4.212_975_5e-3, -3.359_417_7e-3, -6.179_969_8e-3, 2.044_47e-3,
    8.088_958e-3, 1.361_686_8e-4, -9.660_876e-3, -3.230_577_6e-3, 1.056_423_3e-2,
    7.208_604_4e-3, -1.042_866_1e-2, -1.195_791_3e-2, 8.846_187e-3, 1.728_017_6e-2,
    -5.363_448e-3, -2.290_603_5e-2, -5.810_917_6e-4, 2.851_115e-2, 9.831_473e-3,
    -3.374_359_4e-2, -2.403_516_3e-2, 3.825_466e-2, 4.745_053_5e-2, -4.173_074_3e-2,
    -9.600_058e-2, 4.392_319_5e-2, 3.148_779_6e-1, 4.553_276e-1,
];

#[rustfmt::skip]
#[allow(clippy::excessive_precision, reason = "offline-designed coefficients are rounded by the f32 array type")]
const EQUIRIPPLE_3X_HALF: [f32; 73] = [
    1.327_123e-4, 4.203_883_8e-4, 7.619_503e-4, 1.018_868_9e-3, 9.261_082_5e-4,
    4.556_913_8e-4, -2.236_297_7e-4, -6.871_015_6e-4, -6.232_127_6e-4, -3.251_552_7e-5,
    6.430_632_5e-4, 8.532_524_6e-4, 3.426_706_2e-4, -5.466_315e-4, -1.097_768e-3,
    -7.635_160_3e-4, 3.020_591e-4, 1.274_855e-3, 1.270_714_1e-3, 1.359_208_9e-4,
    -1.300_996_8e-3, -1.804_288_2e-3, -7.788_124_4e-4, 1.097_479_4e-3, 2.278_126_8e-3,
    1.604_278_7e-3, -5.982_118e-4, -2.585_733_8e-3, -2.552_257_6e-3, -2.389_278_5e-4,
    2.609_452_7e-3, 3.523_775e-3, 1.420_913e-3, -2.232_552e-3, -4.383_592_4e-3,
    -2.910_213_8e-3, 1.352_988_4e-3, 4.966_647_4e-3, 4.618_32e-3, 1.027_118_6e-4,
    -5.087_524e-3, -6.402_574e-3, -2.166_319_4e-3, 4.552_221_9e-3, 8.067_003e-3,
    4.818_184_3e-3, -3.169_135_3e-3, -9.365_581e-3, -7.982_535e-3, 7.546_161_4e-4,
    1.000_356_6e-2, 1.152_802_6e-2, 2.873_147_6e-3, -9.629_648e-3, -1.527_463_45e-2,
    -7.926_184e-3, 7.800_301_5e-3, 1.900_662_7e-2, 1.473_957_5e-2, -3.866_383e-3,
    -2.249_036_2e-2, -2.403_074_5e-2, -3.382_118_7e-3, 2.549_352_1e-2, 3.774_102e-2,
    1.702_501_1e-2, -2.780_739_5e-2, -6.288_886e-2, -4.927_228_8e-2, 2.926_65e-2,
    1.499_692_4e-1, 2.594_88e-1, 3.035_675_6e-1,
];

#[rustfmt::skip]
#[allow(clippy::excessive_precision, reason = "offline-designed coefficients are rounded by the f32 array type")]
const EQUIRIPPLE_4X_HALF: [f32; 97] = [
    1.055_234_66e-4, 2.391_010_1e-4, 4.279_956_8e-4, 6.160_261_4e-4, 7.321_259e-4,
    7.079_148e-4, 5.113_493e-4, 1.730_119_9e-4, -2.107_602_2e-4, -5.059_833_7e-4,
    -5.940_479_6e-4, -4.286_454_7e-4, -6.914_377e-5, 3.309_252e-4, 5.830_326e-4,
    5.490_941e-4, 2.149_451_2e-4, -2.868_980_6e-4, -7.244_274e-4, -8.695_307e-4,
    -6.134_123e-4, -3.668_890_8e-5, 6.074_631e-4, 1.000_419_1e-3, 9.122_824_3e-4,
    3.302_664_8e-4, -5.081_072e-4, -1.206_060_8e-3, -1.391_481_6e-3, -9.104_315e-4,
    6.828_453e-5, 1.102_045_2e-3, 1.669_773_3e-3, 1.430_175_4e-3, 4.169_758e-4,
    -9.366_126e-4, -1.976_035_3e-3, -2.136_946e-3, -1.244_331_5e-3, 3.561_491_4e-4,
    1.920_381e-3, 2.645_583_4e-3, 2.075_825_4e-3, 3.714_373_2e-4, -1.710_294_2e-3,
    -3.142_881e-3, -3.129_706e-3, -1.535_207_6e-3, 9.801_528e-4, 3.219_905_3e-3,
    4.012_065e-3, 2.811_904e-3, 4.184_084_7e-5, -3.031_448e-3, -4.864_407e-3,
    -4.400_433_5e-3, -1.657_605_3e-3, 2.177_619_1e-3, 5.245_904_4e-3, 5.902_384_8e-3,
    3.579_188_8e-3, -8.203_415_3e-4, -5.242_064e-3, -7.420_761_5e-3, -6.018_72e-3,
    -1.401_087_9e-3, 4.382_789_6e-3, 8.479_988e-3, 8.612_505e-3, 4.300_968e-3,
    -2.704_612_2e-3, -9.086_246e-3, -1.148_741_7e-2, -8.224_652e-3, -3.330_442_5e-4,
    8.654_757e-3, 1.422_271_6e-2, 1.305_920_6e-2, 4.894_307_3e-3, -6.976_251e-3,
    -1.689_948_5e-2, -1.941_619_4e-2, -1.205_867_06e-2, 2.952_33e-3, 1.908_870_8e-2,
    2.807_417_3e-2, 2.370_103_1e-2, 5.284_835_5e-3, -2.088_779_3e-2, -4.304_076e-2,
    -4.803_66e-2, -2.667_985_5e-2, 2.191_956e-2, 8.881_913e-2, 1.574_094_7e-1,
    2.086_712_7e-1, 2.276_439_2e-1,
];

#[cfg(test)]
mod tests {
    use super::{
        LATENCY_SAMPLES, PASSBAND_EQ_CENTER, PASSBAND_EQ_SIDE, POST_FILTER_DELAY,
        equiripple_filter, factor_taps,
    };

    #[test]
    fn equiripple_filters_meet_response_and_latency_contract() {
        for factor in 2..=4 {
            let taps = factor_taps(factor);
            let coefficients = equiripple_filter(factor);
            let group_delay = (taps - 1) / (2 * usize::from(factor));
            assert_eq!(
                group_delay + 2 + POST_FILTER_DELAY,
                LATENCY_SAMPLES as usize
            );
            for index in 0..taps {
                assert_eq!(coefficients[index], coefficients[taps - 1 - index]);
            }

            let sample_rate = 48_000.0 * f64::from(factor);
            let mut passband_error = 0.0_f64;
            for bin in 0..=1024 {
                let frequency = 20_500.0 * f64::from(bin) / 1024.0;
                let magnitude = magnitude(&coefficients[..taps], frequency, sample_rate);
                passband_error = passband_error.max((20.0 * magnitude.log10()).abs());
            }
            assert!(passband_error < 0.052);

            let mut stopband = 0.0_f64;
            for bin in 0..=4096 {
                let frequency = 24_000.0 + (sample_rate * 0.5 - 24_000.0) * f64::from(bin) / 4096.0;
                stopband = stopband.max(magnitude(&coefficients[..taps], frequency, sample_rate));
            }
            assert!(20.0 * stopband.log10() < -84.0);

            assert!((PASSBAND_EQ_CENTER + 2.0 * PASSBAND_EQ_SIDE - 1.0).abs() < f32::EPSILON);
        }
    }

    /// Characterizes the production push schedule, including decimation phase.
    /// The integer host latency is a nominal group delay, not sample alignment
    /// with the first internal sample of each host frame.
    #[test]
    fn streaming_phase_includes_decimation_offset() {
        for factor in 1..=4 {
            for spline in [false, true] {
                let mut oversampler = super::StereoOversampler::default();
                oversampler.reset(factor);
                oversampler.set_spline_correction_immediate(spline);
                let angular = std::f64::consts::TAU / 64.0;
                let (mut sine, mut cosine) = (0.0, 0.0);
                for frame in 0..1024 {
                    for sub in 0..factor {
                        let time = f64::from(frame) + f64::from(sub) / f64::from(factor);
                        let input = (angular * time).sin() as f32;
                        oversampler.push(input, -input);
                    }
                    let (left, right) = oversampler.output();
                    assert!((left + right).abs() < 1.0e-6);
                    if frame >= 256 {
                        sine += f64::from(left) * (angular * f64::from(frame)).sin();
                        cosine += f64::from(left) * (angular * f64::from(frame)).cos();
                    }
                }
                let measured = (-cosine.atan2(sine) / angular).rem_euclid(64.0);
                let expected =
                    f64::from(LATENCY_SAMPLES) - f64::from(factor - 1) / f64::from(factor);
                assert!(
                    (measured - expected).abs() < 1.0e-4,
                    "factor={factor}, spline={spline}, lag={measured}, expected={expected}"
                );
            }
        }
    }

    fn magnitude(coefficients: &[f32], frequency: f64, sample_rate: f64) -> f64 {
        let angular = std::f64::consts::TAU * frequency / sample_rate;
        let (mut real, mut imaginary) = (0.0, 0.0);
        for (index, coefficient) in coefficients.iter().copied().enumerate() {
            let phase = angular * index as f64;
            real += f64::from(coefficient) * phase.cos();
            imaginary -= f64::from(coefficient) * phase.sin();
        }
        real.hypot(imaginary)
    }
}
