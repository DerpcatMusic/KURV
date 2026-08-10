//! Fixed offline harmonic-mip compiler for periodic custom waveforms.
//!
//! Compilation belongs on the editor/state thread. Evaluation is bounded,
//! lock-free, and allocation-free, but publication to the audio thread is an
//! integration concern deliberately kept outside this isolated module.

pub const TABLE_SIZE: usize = 512;
pub const HARMONIC_CAPS: [usize; 24] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 24, 32, 48, 64, 96, 128, 192,
    255,
];
pub const MIP_COUNT: usize = HARMONIC_CAPS.len();
pub const STORAGE_BYTES: usize = MIP_COUNT * TABLE_SIZE * std::mem::size_of::<f32>();
pub const COMPILE_SCRATCH_BYTES: usize =
    2 * TABLE_SIZE * 2 * std::mem::size_of::<f64>();

const TABLE_MASK: usize = TABLE_SIZE - 1;
const _: () = assert!(TABLE_SIZE.is_power_of_two());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    NonFiniteSample { index: usize },
    NonFiniteTransform { level: usize, index: usize },
}

/// Contiguous, 32-byte-aligned mip tables; bin 256 is always omitted.
#[repr(align(32))]
pub struct BandlimitedWaveCurve {
    tables: [[f32; TABLE_SIZE]; MIP_COUNT],
}

const _: [(); STORAGE_BYTES] = [(); std::mem::size_of::<BandlimitedWaveCurve>()];

impl BandlimitedWaveCurve {
    pub const fn zero() -> Self {
        Self {
            tables: [[0.0; TABLE_SIZE]; MIP_COUNT],
        }
    }

    /// Samples one period at `phase = n / 512`, transforms it once, then
    /// synthesizes every harmonic cap with an inverse transform.
    ///
    /// Each mip retains bins whose signed harmonic number is at most its cap.
    /// Work and storage are fixed, with no heap allocation.
    ///
    /// # Errors
    ///
    /// Returns [`CompileError::NonFiniteSample`] if the sampler produces NaN
    /// or infinity, and [`CompileError::NonFiniteTransform`] if an output cannot
    /// be represented as `f32`. The callback must be periodic on `[0, 1)`.
    pub fn compile(mut sampler: impl FnMut(f32) -> f32) -> Result<Self, CompileError> {
        let mut spectrum = [Complex::ZERO; TABLE_SIZE];
        for (index, bin) in spectrum.iter_mut().enumerate() {
            let sample = sampler(index as f32 / TABLE_SIZE as f32);
            if !sample.is_finite() {
                return Err(CompileError::NonFiniteSample { index });
            }
            bin.re = f64::from(sample);
        }
        fft(&mut spectrum, false);
        spectrum[0].im = 0.0;
        spectrum[TABLE_SIZE / 2] = Complex::ZERO;

        let mut compiled = Self::zero();
        for (level, harmonic_cap) in HARMONIC_CAPS.into_iter().enumerate() {
            let mut filtered = spectrum;
            for (bin, value) in filtered.iter_mut().enumerate().skip(1) {
                if bin.min(TABLE_SIZE - bin) > harmonic_cap {
                    *value = Complex::ZERO;
                }
            }
            filtered[0].im = 0.0;
            fft(&mut filtered, true);
            for (index, (sample, value)) in compiled.tables[level]
                .iter_mut()
                .zip(filtered)
                .enumerate()
            {
                if !value.re.is_finite()
                    || !value.im.is_finite()
                    || value.re.abs() > f64::from(f32::MAX)
                {
                    return Err(CompileError::NonFiniteTransform { level, index });
                }
                *sample = value.re as f32;
            }
        }
        Ok(compiled)
    }

    /// Chooses the richest mip whose highest harmonic is strictly below Nyquist.
    ///
    /// A warp-aware caller must include the maximum warp derivative in this
    /// normalized cycles-per-sample bound. Invalid or out-of-band steps return `None`.
    #[must_use]
    #[inline]
    pub fn mip_for_phase_step(max_abs_phase_step: f32) -> Option<usize> {
        let step = max_abs_phase_step.abs();
        if !step.is_finite() {
            return None;
        }
        HARMONIC_CAPS
            .iter()
            .rposition(|&cap| cap as f32 * step < 0.5)
    }

    /// Evaluates a periodic phase; invalid inputs or no legal harmonic return zero.
    #[must_use]
    #[inline]
    pub fn eval(&self, phase: f32, max_abs_phase_step: f32) -> f32 {
        let Some(mip) = Self::mip_for_phase_step(max_abs_phase_step) else {
            return 0.0;
        };
        self.eval_mip(phase, mip)
    }

    /// Evaluates one selected mip with periodic four-point Catmull-Rom interpolation.
    #[must_use]
    #[inline]
    pub fn eval_mip(&self, phase: f32, mip: usize) -> f32 {
        if !phase.is_finite() {
            return 0.0;
        }
        let Some(table) = self.tables.get(mip) else {
            return 0.0;
        };
        let position = (phase - phase.floor()) * TABLE_SIZE as f32;
        let index = (position as usize).min(TABLE_SIZE - 1);
        let t = position - index as f32;
        let p0 = table[(index + TABLE_SIZE - 1) & TABLE_MASK];
        let p1 = table[index];
        let p2 = table[(index + 1) & TABLE_MASK];
        let p3 = table[(index + 2) & TABLE_MASK];
        let a = (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * 0.5;
        let b = (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * 0.5;
        let c = (p2 - p0) * 0.5;
        a.mul_add(t, b).mul_add(t, c).mul_add(t, p1)
    }

    /// Returns one compiled mip table.
    #[must_use]
    pub fn table(&self, mip: usize) -> Option<&[f32; TABLE_SIZE]> {
        self.tables.get(mip)
    }

    #[must_use]
    pub const fn harmonic_cap(mip: usize) -> Option<usize> {
        if mip < MIP_COUNT {
            Some(HARMONIC_CAPS[mip])
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };

    #[inline]
    fn multiply(self, other: Self) -> Self {
        Self {
            re: self.re.mul_add(other.re, -self.im * other.im),
            im: self.re.mul_add(other.im, self.im * other.re),
        }
    }
}

fn fft(values: &mut [Complex; TABLE_SIZE], inverse: bool) {
    let mut reversed = 0;
    for index in 1..TABLE_SIZE {
        let mut bit = TABLE_SIZE >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if index < reversed {
            values.swap(index, reversed);
        }
    }

    let direction = if inverse { 1.0 } else { -1.0 };
    let mut width = 2;
    while width <= TABLE_SIZE {
        let angle = direction * std::f64::consts::TAU / width as f64;
        let (sin, cos) = angle.sin_cos();
        let root = Complex { re: cos, im: sin };
        let half = width / 2;
        for start in (0..TABLE_SIZE).step_by(width) {
            let mut twiddle = Complex { re: 1.0, im: 0.0 };
            for offset in 0..half {
                let even = values[start + offset];
                let odd = values[start + offset + half].multiply(twiddle);
                values[start + offset] = Complex {
                    re: even.re + odd.re,
                    im: even.im + odd.im,
                };
                values[start + offset + half] = Complex {
                    re: even.re - odd.re,
                    im: even.im - odd.im,
                };
                twiddle = twiddle.multiply(root);
            }
        }
        width <<= 1;
    }

    if inverse {
        let scale = 1.0 / TABLE_SIZE as f64;
        for value in values {
            value.re *= scale;
            value.im *= scale;
        }
    }
}
