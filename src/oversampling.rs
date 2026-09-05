//! Fixed-allocation dynamic synthesis oversampling.

use truce_simd::simd::f32x8;

pub const DEFAULT_FACTOR: u8 = 2;
pub const LATENCY_SAMPLES: u32 = 33;
pub const TAIL_SAMPLES: u8 = 67;

const HOST_LATENCY: usize = LATENCY_SAMPLES as usize;
const MAX_TAPS: usize = 193;
const BUFFER: usize = MAX_TAPS * 2;
const POST_FILTER_DELAY: usize = 6;
/// Symmetric seven-tap host-rate correction, centred three samples back.
///
/// `H(w) = center + 2 * side * cos(w) + 2 * outer * cos(2w) + 2 * fringe *
/// cos(3w)`, and the taps always sum to unity so the correction cannot move
/// the level. The third tap pair is not free -- it costs one host-rate
/// multiply-add per channel and one sample of the padding delay below -- but a
/// five-tap fit bottoms out at 0.32 dB of residual at 2x, and this reaches
/// 0.17 dB.
#[derive(Clone, Copy)]
struct PassbandEqualizer {
    fringe: f32,
    outer: f32,
    side: f32,
    center: f32,
}

impl PassbandEqualizer {
    /// The correction each oversampling factor needs.
    ///
    /// This is *not* correcting the decimator. The equiripple decimators are
    /// already flat to 0.052 dB across 0-20.5 kHz, which
    /// `equiripple_filters_meet_response_and_latency_contract` asserts
    /// directly. What actually needs correcting is the spline BLEP/BLAMP
    /// residual in the oscillators: it is a fixed-width smoothing kernel, so
    /// the lower the oversampling factor, the more of the audible band it
    /// eats. Measured against an ideal saw
    /// (`every_factor_matches_the_ideal_saw_spectrum`), the uncorrected loss
    /// at 20.5 kHz is 3.05 dB at 2x, 1.22 dB at 3x and 0.71 dB at 4x.
    ///
    /// Each set is the relative-error least-squares inverse of the measured
    /// composite for that factor, fitted over 100 Hz - 20.5 kHz under a
    /// unity-DC constraint. That leaves every factor flat to under 0.18 dB
    /// and -- far more importantly -- leaves them agreeing with each other to
    /// within 0.1 dB, so changing the oversampling quality no longer changes
    /// the tone.
    ///
    /// 1x has no entry here because it does not run a decimator at all; its
    /// correction is `DirectEqualizer`, which needs far more taps because its
    /// droop is far deeper. See that type for the measured trade it makes.
    const fn for_factor(factor: u8) -> Self {
        match factor {
            2 => Self {
                fringe: -0.006_614_62,
                outer: 0.025_648_33,
                side: -0.104_598_19,
                center: 1.171_128_97,
            },
            3 => Self {
                fringe: -0.002_059_59,
                outer: 0.008_046_80,
                side: -0.036_808_59,
                center: 1.061_642_77,
            },
            _ => Self {
                fringe: -0.001_074_17,
                outer: 0.004_224_95,
                side: -0.020_060_77,
                center: 1.033_819_99,
            },
        }
    }

    /// Filter one host-rate sample against the six that preceded it.
    ///
    /// `history[0]` is the previous sample, so the centre tap lands on
    /// `history[2]`, three samples back from `sample`.
    fn apply(self, sample: f32, history: &[f32; 6]) -> f32 {
        self.fringe.mul_add(
            sample + history[5],
            self.outer.mul_add(
                history[0] + history[4],
                self.side
                    .mul_add(history[1] + history[3], self.center * history[2]),
            ),
        )
    }
}

pub struct StereoOversampler {
    x2: StereoDecimator<97, 12>,
    x3: StereoDecimator<145, 18>,
    x4: StereoDecimator<193, 24>,
    direct_delay: StereoDelay,
    direct_equalizer: DirectEqualizer,
    direct_output: (f32, f32),
    factor: u8,
}

impl Default for StereoOversampler {
    fn default() -> Self {
        Self {
            x2: StereoDecimator::new(2),
            x3: StereoDecimator::new(3),
            x4: StereoDecimator::new(4),
            direct_delay: StereoDelay::default(),
            direct_equalizer: DirectEqualizer::default(),
            direct_output: (0.0, 0.0),
            factor: DEFAULT_FACTOR,
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
        self.direct_equalizer.reset();
        self.direct_output = (0.0, 0.0);
        self.factor = factor.clamp(1, 4);
    }

    pub fn push(&mut self, left: f32, right: f32) {
        match self.factor {
            1 => self.direct_output = self.process_direct_inner(left, right),
            2 => self.x2.push(left, right),
            3 => self.x3.push(left, right),
            _ => self.x4.push(left, right),
        }
    }

    pub fn process_direct(&mut self, left: f32, right: f32) -> (f32, f32) {
        debug_assert_eq!(self.factor, 1);
        self.direct_output = self.process_direct_inner(left, right);
        self.direct_output
    }

    fn process_direct_inner(&mut self, left: f32, right: f32) -> (f32, f32) {
        let (left, right) = self.direct_equalizer.process(left, right);
        self.direct_delay.process(left, right)
    }

    pub fn output(&mut self) -> (f32, f32) {
        match self.factor {
            1 => self.direct_output,
            2 => self.x2.output(),
            3 => self.x3.output(),
            _ => self.x4.output(),
        }
    }
}

/// Half-width of the 1x correction: 17 taps, centred eight samples back.
const DIRECT_EQ_HALF: usize = 8;

/// Host-rate correction for the 1x path.
///
/// The 1x path runs the spline BLEP kernel at the host rate, so the kernel's
/// fixed sample-width smoothing eats the whole top of the audible band: the
/// uncorrected loss is 2.85 dB at 10 kHz, 7.66 dB at 16 kHz and 14.51 dB at
/// 20.5 kHz, against 0.17 dB or better for 2x-4x. That is why 1x used to sound
/// audibly darker than every other setting.
///
/// These taps are the least-squares inverse of that measured curve over
/// 100 Hz - 20.5 kHz under a unity-DC constraint, which brings 1x to 0.16 dB
/// of the ideal saw spectrum -- the same budget the oversampled factors meet,
/// so all four now sound alike.
///
/// # The cost, measured
///
/// Flattening a 14.5 dB droop needs a filter that peaks at +18 dB near
/// Nyquist, and the 20.5-24 kHz transition is only 1 kHz wide, so the peak
/// cannot be constrained without giving back several dB of passband --
/// penalised fits were tried and cost 4 dB or more. Since the boost lands
/// where 1x keeps most of its alias energy, the alias-to-signal ratio inside
/// the audible band drops from 54.1 dB to 45.4 dB (2333 Hz saw, 4x measures
/// 82.0 dB). Peak level is unchanged in practice: 0.85 before, 1.11 after,
/// against 1.07 for the same 4x saw.
///
/// This is a real trade and it is the reason 1x was left uncorrected before.
/// It is made deliberately: matching tone across the quality settings is worth
/// more than 9 dB of alias headroom on the setting that exists to save CPU,
/// and a listener switching quality now hears the same instrument.
#[derive(Clone, Copy)]
struct DirectEqualizer {
    left: [f32; DIRECT_EQ_HALF * 2],
    right: [f32; DIRECT_EQ_HALF * 2],
}

/// `[center, side_1, ..., side_8]`; the taps sum to unity at DC.
const DIRECT_EQ_TAPS: [f32; DIRECT_EQ_HALF + 1] = [
    2.578_669_4,
    -1.202_128_2,
    0.650_770_6,
    -0.378_523_9,
    0.220_009_8,
    -0.122_496_0,
    0.062_147_6,
    -0.027_035_1,
    0.007_920_4,
];

impl Default for DirectEqualizer {
    fn default() -> Self {
        Self {
            left: [0.0; DIRECT_EQ_HALF * 2],
            right: [0.0; DIRECT_EQ_HALF * 2],
        }
    }
}

impl DirectEqualizer {
    fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
    }

    /// `history[0]` is the previous sample, so the centre tap lands on
    /// `history[DIRECT_EQ_HALF - 1]`, eight samples back from `sample`.
    fn filter(sample: f32, history: &[f32; DIRECT_EQ_HALF * 2]) -> f32 {
        let mut sum = DIRECT_EQ_TAPS[0] * history[DIRECT_EQ_HALF - 1];
        for offset in 1..=DIRECT_EQ_HALF {
            let newer = if offset == DIRECT_EQ_HALF {
                sample
            } else {
                history[DIRECT_EQ_HALF - 1 - offset]
            };
            sum = DIRECT_EQ_TAPS[offset].mul_add(newer + history[DIRECT_EQ_HALF - 1 + offset], sum);
        }
        sum
    }

    fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let output = (
            Self::filter(left, &self.left),
            Self::filter(right, &self.right),
        );
        self.left.rotate_right(1);
        self.left[0] = left;
        self.right.rotate_right(1);
        self.right[0] = right;
        output
    }
}

/// Padding delay for the 1x path.
///
/// `DirectEqualizer` is a linear-phase FIR centred `DIRECT_EQ_HALF` samples
/// back, so it already accounts for that much of the reported latency and this
/// only has to make up the rest.
const DIRECT_DELAY: usize = HOST_LATENCY - DIRECT_EQ_HALF;

struct StereoDelay {
    left: [f32; DIRECT_DELAY],
    right: [f32; DIRECT_DELAY],
    write: usize,
}

impl Default for StereoDelay {
    fn default() -> Self {
        Self {
            left: [0.0; DIRECT_DELAY],
            right: [0.0; DIRECT_DELAY],
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
        if self.write == DIRECT_DELAY {
            self.write = 0;
        }
        output
    }
}

struct StereoDecimator<const TAPS: usize, const BLOCKS: usize> {
    coefficient_blocks: [f32x8; BLOCKS],
    tail_coefficient: f32,
    equalizer: PassbandEqualizer,
    left: [f32; BUFFER],
    right: [f32; BUFFER],
    write: usize,
    equalizer_left: [f32; 6],
    equalizer_right: [f32; 6],
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
            equalizer: PassbandEqualizer::for_factor(factor),
            left: [0.0; BUFFER],
            right: [0.0; BUFFER],
            write: 0,
            equalizer_left: [0.0; 6],
            equalizer_right: [0.0; 6],
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
        const SILENCE: [f32; 8] = [0.0; 8];

        let mut left_a = f32x8::ZERO;
        let mut left_b = f32x8::ZERO;
        let mut right_a = f32x8::ZERO;
        let mut right_b = f32x8::ZERO;
        let mut block = 0;
        while block + 2 <= BLOCKS {
            let index = block * 8;
            let coefficients_a = self.coefficient_blocks[block];
            let coefficients_b = self.coefficient_blocks[block + 1];
            // `write < TAPS` and `index + 16 <= 2 * TAPS` both hold by
            // construction, so every chunk is present. The fallback is here
            // because this runs inside the host's audio callback, where a
            // panic unwinds across the plugin ABI and takes the host with it;
            // a stretch of silence is a far better failure than that. The
            // debug assertion is what actually catches the mistake.
            debug_assert!(self.write + index + 16 <= 2 * TAPS);
            let left_samples_a = f32x8::from(
                *self.left[self.write + index..]
                    .first_chunk()
                    .unwrap_or(&SILENCE),
            );
            let left_samples_b = f32x8::from(
                *self.left[self.write + index + 8..]
                    .first_chunk()
                    .unwrap_or(&SILENCE),
            );
            let right_samples_a = f32x8::from(
                *self.right[self.write + index..]
                    .first_chunk()
                    .unwrap_or(&SILENCE),
            );
            let right_samples_b = f32x8::from(
                *self.right[self.write + index + 8..]
                    .first_chunk()
                    .unwrap_or(&SILENCE),
            );
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
        let equalized_left = self.equalizer.apply(left, &self.equalizer_left);
        let equalized_right = self.equalizer.apply(right, &self.equalizer_right);
        self.equalizer_left = [
            left,
            self.equalizer_left[0],
            self.equalizer_left[1],
            self.equalizer_left[2],
            self.equalizer_left[3],
            self.equalizer_left[4],
        ];
        self.equalizer_right = [
            right,
            self.equalizer_right[0],
            self.equalizer_right[1],
            self.equalizer_right[2],
            self.equalizer_right[3],
            self.equalizer_right[4],
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

// Parks-McClellan linear-phase kernels: 0-20.5 kHz passband, 24 kHz stopband,
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
        LATENCY_SAMPLES, POST_FILTER_DELAY, PassbandEqualizer, equiripple_filter, factor_taps,
    };

    #[test]
    fn equiripple_filters_meet_response_and_latency_contract() {
        for factor in 2..=4 {
            let taps = factor_taps(factor);
            let coefficients = equiripple_filter(factor);
            let group_delay = (taps - 1) / (2 * usize::from(factor));
            assert_eq!(
                group_delay + 3 + POST_FILTER_DELAY,
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

            // The correction must not move the level. Everything else about
            // it is calibrated against the oscillators rather than the
            // decimator, and is asserted by
            // `every_factor_matches_the_ideal_saw_spectrum`.
            let equalizer = PassbandEqualizer::for_factor(factor);
            let dc = equalizer.center + 2.0 * (equalizer.side + equalizer.outer + equalizer.fringe);
            assert!(
                (dc - 1.0).abs() < 1.0e-6,
                "factor {factor}: equalizer DC gain is {dc}"
            );
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

#[cfg(test)]
mod passband_equalizer_tests {
    use super::StereoOversampler;
    use crate::oscillators::Antialiasing;

    /// Harmonic magnitudes of a bandlimited saw taken at the host rate.
    ///
    /// This measures the whole chain the way a listener hears it: the spline
    /// BLEP kernel running at the oversampled rate, the equiripple decimator,
    /// and the post-decimation equalizer. Comparing against the ideal `2/(pi k)`
    /// saw spectrum is what makes the equalizer falsifiable — the taps exist to
    /// undo the BLEP kernel's high-frequency droop, and nothing about the
    /// decimator in isolation can tell you whether they succeed.
    fn saw_harmonics(factor: u8, f0: f64, harmonics: usize) -> Vec<f64> {
        let host_rate = 48_000.0_f64;
        let antialiasing = Antialiasing::Spline.for_factor(factor);
        let step = f0 / (host_rate * f64::from(factor));
        let host_frames = 48_000_usize;
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        let mut phase = 0.0_f64;
        let mut host = Vec::with_capacity(host_frames);
        for _ in 0..host_frames {
            for _ in 0..factor {
                let sample = crate::oscillators::antialias_probe_saw(phase, step, antialiasing);
                phase += step;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                oversampler.push(sample as f32, sample as f32);
            }
            host.push(f64::from(oversampler.output().0));
        }

        // Drop the latency and filter warm-up, then analyse a whole number of
        // periods so the DFT bins land exactly on the harmonics.
        let skip = 2_000;
        let period = (host_rate / f0).round() as usize;
        assert_eq!(
            host_rate / f0,
            period as f64,
            "probe requires an integer period"
        );
        let usable = (host.len() - skip) / period * period;
        let analysed = &host[skip..skip + usable];
        let length = analysed.len() as f64;
        (1..=harmonics)
            .map(|harmonic| {
                let omega = std::f64::consts::TAU * (harmonic as f64) * f0 / host_rate;
                let (mut real, mut imaginary) = (0.0, 0.0);
                for (index, sample) in analysed.iter().enumerate() {
                    let phase = omega * index as f64;
                    real += sample * phase.cos();
                    imaginary -= sample * phase.sin();
                }
                2.0 * real.hypot(imaginary) / length
            })
            .collect()
    }

    #[test]
    fn every_factor_matches_the_ideal_saw_spectrum() {
        let f0 = 100.0;
        let harmonics = 205; // 20.5 kHz, the top of the corrected band.
        let ideal: Vec<f64> = (1..=harmonics)
            .map(|harmonic| 2.0 / (std::f64::consts::PI * harmonic as f64))
            .collect();

        let mut responses = Vec::new();
        for factor in 1..=4 {
            let measured = saw_harmonics(factor, f0, harmonics);
            let mut worst = (0.0_f64, 0.0_f64);
            let response: Vec<f64> = measured
                .iter()
                .zip(&ideal)
                .enumerate()
                .map(|(index, (measured, ideal))| {
                    let decibels = 20.0 * (measured / ideal).log10();
                    if decibels.abs() > worst.0.abs() {
                        worst = (decibels, (index + 1) as f64 * f0);
                    }
                    decibels
                })
                .collect();
            assert!(
                worst.0.abs() < 0.20,
                "factor {factor}: {:.3} dB from ideal at {:.0} Hz",
                worst.0,
                worst.1
            );
            responses.push(response);
        }

        // Changing the oversampling factor must not change the tone. This is
        // the property a user actually notices, and it is stricter than each
        // factor's own budget because the errors could otherwise sit at
        // opposite ends of it. 1x is included: it is corrected by
        // `DirectEqualizer` precisely so that it belongs here.
        for (index, factor) in (2..=4_u8).enumerate() {
            for (harmonic, (reference, other)) in
                responses[0].iter().zip(&responses[index + 1]).enumerate()
            {
                let difference = reference - other;
                assert!(
                    difference.abs() < 0.15,
                    "factor {factor} differs from 1x by {difference:.3} dB at {:.0} Hz",
                    (harmonic + 1) as f64 * f0
                );
            }
        }
    }
}
