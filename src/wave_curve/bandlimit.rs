//! Fixed offline harmonic-mip compiler for periodic custom waveforms.
//!
//! Compilation belongs on the editor/state thread. Evaluation is bounded,
//! lock-free, and allocation-free, but publication to the audio thread is an
//! integration concern deliberately kept outside this isolated module.

use truce_simd::simd::{f32x4, f32x8};

use crate::dsp::{Complex, fft};

pub const TABLE_SIZE: usize = 2048;
pub const HARMONIC_CAPS: [usize; 20] = [
    1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1023,
];
pub const MIP_COUNT: usize = HARMONIC_CAPS.len();
pub const STORAGE_BYTES: usize = MIP_COUNT * TABLE_SIZE * std::mem::size_of::<f32>();
pub const COMPILE_SCRATCH_BYTES: usize = 2 * TABLE_SIZE * 2 * std::mem::size_of::<f64>();

const TABLE_MASK: usize = TABLE_SIZE - 1;
const _: () = assert!(TABLE_SIZE.is_power_of_two());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    NonFiniteSample { index: usize },
    NonFiniteTransform { level: usize, index: usize },
}

/// Contiguous, 32-byte-aligned mip tables; the Nyquist bin is always omitted.
#[derive(Clone)]
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

    /// Allocates an initialized zero table without constructing the large bank
    /// in the caller's stack frame.
    pub fn boxed_zero() -> Box<Self> {
        let mut table = Box::<Self>::new_uninit();
        // SAFETY: the type contains only f32 arrays and zero is valid for f32.
        unsafe {
            std::ptr::write_bytes(table.as_mut_ptr(), 0, 1);
            table.assume_init()
        }
    }

    /// Samples one period at `phase = n / TABLE_SIZE`, transforms it once, then
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
    pub fn compile(sampler: impl FnMut(f32) -> f32) -> Result<Self, CompileError> {
        Self::compile_boxed(sampler).map(|compiled| *compiled)
    }

    /// Heap-targeted compiler for large immutable artifacts. This avoids
    /// placing the 160 KiB table bank in an offline worker's stack frame.
    pub fn compile_boxed(mut sampler: impl FnMut(f32) -> f32) -> Result<Box<Self>, CompileError> {
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

        let mut compiled = Self::boxed_zero();
        for (level, harmonic_cap) in HARMONIC_CAPS.into_iter().enumerate() {
            let mut filtered = spectrum;
            for (bin, value) in filtered.iter_mut().enumerate().skip(1) {
                if bin.min(TABLE_SIZE - bin) > harmonic_cap {
                    *value = Complex::ZERO;
                }
            }
            filtered[0].im = 0.0;
            fft(&mut filtered, true);
            for (index, (sample, value)) in
                compiled.tables[level].iter_mut().zip(filtered).enumerate()
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

    /// Evaluates four periodic phases using one conservatively selected mip.
    ///
    /// `max_abs_phase_step` must cover the maximum absolute effective phase
    /// step across all four lanes so they cannot diverge onto different mips.
    #[must_use]
    #[inline]
    pub fn eval4(&self, phase: f32x4, max_abs_phase_step: f32) -> f32x4 {
        let Some(mip) = Self::mip_for_phase_step(max_abs_phase_step) else {
            return f32x4::ZERO;
        };
        #[cfg(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        ))]
        {
            let [a, b, c, d]: [f32; 4] = phase.into();
            let sample: [f32; 8] = self
                .eval8_mip(f32x8::from([a, b, c, d, 0.0, 0.0, 0.0, 0.0]), mip)
                .into();
            return f32x4::from([sample[0], sample[1], sample[2], sample[3]]);
        }
        #[cfg(not(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )))]
        {
            let phase: [f32; 4] = phase.into();
            f32x4::from(phase.map(|phase| self.eval_mip(phase, mip)))
        }
    }

    /// Evaluates eight periodic phases using one conservatively selected mip.
    ///
    /// `max_abs_phase_step` must cover the maximum absolute effective phase
    /// step across all eight lanes so they cannot diverge onto different mips.
    #[must_use]
    #[inline]
    pub fn eval8(&self, phase: f32x8, max_abs_phase_step: f32) -> f32x8 {
        let Some(mip) = Self::mip_for_phase_step(max_abs_phase_step) else {
            return f32x8::ZERO;
        };
        self.eval8_mip(phase, mip)
    }

    /// Evaluates eight periodic phases from one caller-selected mip.
    #[must_use]
    #[inline]
    pub fn eval8_mip(&self, phase: f32x8, mip: usize) -> f32x8 {
        if mip >= MIP_COUNT {
            return f32x8::ZERO;
        }
        #[cfg(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        ))]
        {
            return Self::eval8_mip_avx2(phase, &self.tables[mip]);
        }
        #[cfg(not(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )))]
        {
            let phase: [f32; 8] = phase.into();
            f32x8::from(phase.map(|phase| self.eval_mip(phase, mip)))
        }
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

    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    ))]
    #[inline]
    fn eval8_mip_avx2(phase: f32x8, table: &[f32; TABLE_SIZE]) -> f32x8 {
        use std::arch::x86_64::{
            _CMP_LE_OQ, _mm256_add_epi32, _mm256_add_ps, _mm256_and_ps, _mm256_and_si256,
            _mm256_andnot_ps, _mm256_cmp_ps, _mm256_cvtepi32_ps, _mm256_cvttps_epi32,
            _mm256_floor_ps, _mm256_fmadd_ps, _mm256_i32gather_ps, _mm256_loadu_ps,
            _mm256_max_epi32, _mm256_min_epi32, _mm256_mul_ps, _mm256_set1_epi32, _mm256_set1_ps,
            _mm256_storeu_ps, _mm256_sub_ps,
        };

        let phase: [f32; 8] = phase.into();
        let mut output = [0.0; 8];
        // SAFETY: `phase`, `output`, and the fixed-size `table` are initialized.
        // The base index is clamped to the table range, and each periodic point index is
        // masked back to that range before its gather. Scale 4 therefore reads
        // one in-bounds `f32` per lane. This function is compiled only when AVX2
        // and FMA are enabled; the finite mask restores scalar invalid-input zeroes.
        unsafe {
            let phase = _mm256_loadu_ps(phase.as_ptr());
            let abs_phase = _mm256_andnot_ps(_mm256_set1_ps(-0.0), phase);
            let finite = _mm256_cmp_ps(abs_phase, _mm256_set1_ps(f32::MAX), _CMP_LE_OQ);
            let position = _mm256_mul_ps(
                _mm256_sub_ps(phase, _mm256_floor_ps(phase)),
                _mm256_set1_ps(TABLE_SIZE as f32),
            );
            let index = _mm256_min_epi32(
                _mm256_max_epi32(_mm256_cvttps_epi32(position), _mm256_set1_epi32(0)),
                _mm256_set1_epi32((TABLE_SIZE - 1) as i32),
            );
            let t = _mm256_sub_ps(position, _mm256_cvtepi32_ps(index));
            let mask = _mm256_set1_epi32(TABLE_MASK as i32);
            let p0_index = _mm256_and_si256(_mm256_add_epi32(index, _mm256_set1_epi32(-1)), mask);
            let p1_index = _mm256_and_si256(index, mask);
            let p2_index = _mm256_and_si256(_mm256_add_epi32(index, _mm256_set1_epi32(1)), mask);
            let p3_index = _mm256_and_si256(_mm256_add_epi32(index, _mm256_set1_epi32(2)), mask);
            let p0 = _mm256_i32gather_ps::<4>(table.as_ptr(), p0_index);
            let p1 = _mm256_i32gather_ps::<4>(table.as_ptr(), p1_index);
            let p2 = _mm256_i32gather_ps::<4>(table.as_ptr(), p2_index);
            let p3 = _mm256_i32gather_ps::<4>(table.as_ptr(), p3_index);
            let half = _mm256_set1_ps(0.5);
            let a = _mm256_mul_ps(
                _mm256_add_ps(
                    _mm256_sub_ps(p3, p0),
                    _mm256_mul_ps(_mm256_set1_ps(3.0), _mm256_sub_ps(p1, p2)),
                ),
                half,
            );
            let b = _mm256_mul_ps(
                _mm256_sub_ps(
                    _mm256_add_ps(
                        _mm256_mul_ps(_mm256_set1_ps(2.0), p0),
                        _mm256_mul_ps(_mm256_set1_ps(4.0), p2),
                    ),
                    _mm256_add_ps(_mm256_mul_ps(_mm256_set1_ps(5.0), p1), p3),
                ),
                half,
            );
            let c = _mm256_mul_ps(_mm256_sub_ps(p2, p0), half);
            let sample = _mm256_fmadd_ps(_mm256_fmadd_ps(_mm256_fmadd_ps(a, t, b), t, c), t, p1);
            _mm256_storeu_ps(output.as_mut_ptr(), _mm256_and_ps(sample, finite));
        }
        f32x8::from(output)
    }

    /// Returns one compiled mip table.
    #[must_use]
    pub fn table(&self, mip: usize) -> Option<&[f32; TABLE_SIZE]> {
        self.tables.get(mip)
    }

    pub(crate) fn table_mut(&mut self, mip: usize) -> Option<&mut [f32; TABLE_SIZE]> {
        self.tables.get_mut(mip)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_low_notes_can_use_far_more_than_two_hundred_harmonics() {
        let c1_step = 32.703_197 / 48_000.0;
        let mip = BandlimitedWaveCurve::mip_for_phase_step(c1_step).expect("C1 must be playable");
        let cap = BandlimitedWaveCurve::harmonic_cap(mip).expect("mip must expose its cap");
        assert!(cap >= 512, "C1 cap was only {cap}");
        assert!(cap as f32 * c1_step < 0.5);
    }

    #[test]
    fn every_selected_mip_stays_below_nyquist() {
        for step in [1.0 / 96_000.0, 32.7 / 48_000.0, 440.0 / 48_000.0, 0.24] {
            let mip =
                BandlimitedWaveCurve::mip_for_phase_step(step).expect("step must be playable");
            let cap = BandlimitedWaveCurve::harmonic_cap(mip).expect("valid mip");
            assert!(cap as f32 * step < 0.5);
            if let Some(next) = BandlimitedWaveCurve::harmonic_cap(mip + 1) {
                assert!(next as f32 * step >= 0.5);
            }
        }
    }

    #[test]
    fn compiled_sine_is_periodic_and_finite() {
        let table = BandlimitedWaveCurve::compile(|phase| (std::f32::consts::TAU * phase).sin())
            .expect("finite sine must compile");
        for phase in [-3.25, -0.25, 0.0, 0.25, 1.0, 4.75] {
            let sample = table.eval(phase, 440.0 / 48_000.0);
            let wrapped = table.eval(phase + 1.0, 440.0 / 48_000.0);
            assert!(sample.is_finite());
            assert!(
                (sample - wrapped).abs() <= 1.0e-5,
                "{phase}: {sample} != {wrapped}"
            );
        }
    }

    #[test]
    fn non_finite_source_is_rejected() {
        assert!(matches!(
            BandlimitedWaveCurve::compile(|phase| if phase == 0.0 { f32::NAN } else { 0.0 }),
            Err(CompileError::NonFiniteSample { index: 0 })
        ));
    }
}
