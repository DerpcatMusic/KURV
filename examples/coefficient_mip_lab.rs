#![allow(dead_code)]

#[path = "../src/dsp.rs"]
mod dsp;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::hint::black_box;
use std::time::Instant;

use dsp::{Complex, fft};
use truce_simd::simd::f32x8;
use wave_curve::{WaveCurveData, WaveCurveRt, WaveKnot};

const GRID: usize = 65_536;
const SEGMENTS: usize = 16;
const CAPS: [usize; 5] = [2, 3, 6, 13, 27];

#[derive(Clone, Copy)]
struct UnclampedCurve {
    coefficients: [f32; 64],
}

impl UnclampedCurve {
    fn from_clamped(curve: WaveCurveRt) -> Self {
        Self {
            coefficients: curve.coefficients(),
        }
    }

    #[inline]
    fn eval(self, phase: f32) -> f32 {
        let position = phase * SEGMENTS as f32;
        let segment = (position as usize).min(SEGMENTS - 1);
        let t = position - segment as f32;
        self.coefficients[coefficient_index(segment, 0)]
            .mul_add(t, self.coefficients[coefficient_index(segment, 1)])
            .mul_add(t, self.coefficients[coefficient_index(segment, 2)])
            .mul_add(t, self.coefficients[coefficient_index(segment, 3)])
    }

    #[inline]
    fn eval8(self, phase: f32x8) -> f32x8 {
        #[cfg(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        ))]
        unsafe {
            use core::arch::x86_64::{
                _mm256_and_si256, _mm256_blendv_ps, _mm256_castsi256_ps, _mm256_cmpgt_epi32,
                _mm256_cvtepi32_ps, _mm256_cvttps_epi32, _mm256_fmadd_ps, _mm256_loadu_ps,
                _mm256_max_epi32, _mm256_min_epi32, _mm256_mul_ps, _mm256_permutevar8x32_ps,
                _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_ps, _mm256_sub_ps,
            };
            let input: [f32; 8] = phase.into();
            let mut output = [0.0; 8];
            let phase = _mm256_loadu_ps(input.as_ptr());
            let position = _mm256_mul_ps(phase, _mm256_set1_ps(SEGMENTS as f32));
            let segment = _mm256_min_epi32(
                _mm256_max_epi32(_mm256_cvttps_epi32(position), _mm256_set1_epi32(0)),
                _mm256_set1_epi32((SEGMENTS - 1) as i32),
            );
            let t = _mm256_sub_ps(position, _mm256_cvtepi32_ps(segment));
            let bank_index = _mm256_and_si256(segment, _mm256_set1_epi32(7));
            let upper = _mm256_castsi256_ps(_mm256_cmpgt_epi32(segment, _mm256_set1_epi32(7)));
            let select = |plane: usize| {
                let values = self.coefficients.as_ptr().add(plane * SEGMENTS);
                let lower = _mm256_permutevar8x32_ps(_mm256_loadu_ps(values), bank_index);
                let upper_bank =
                    _mm256_permutevar8x32_ps(_mm256_loadu_ps(values.add(8)), bank_index);
                _mm256_blendv_ps(lower, upper_bank, upper)
            };
            let sample = _mm256_fmadd_ps(
                _mm256_fmadd_ps(_mm256_fmadd_ps(select(0), t, select(1)), t, select(2)),
                t,
                select(3),
            );
            _mm256_storeu_ps(output.as_mut_ptr(), sample);
            return f32x8::from(output);
        }
        #[cfg(not(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )))]
        {
            let phases: [f32; 8] = phase.into();
            f32x8::from(phases.map(|phase| self.eval(phase)))
        }
    }

    fn interpolate(a: Self, b: Self, mix: f32) -> Self {
        Self {
            coefficients: std::array::from_fn(|index| {
                (b.coefficients[index] - a.coefficients[index]).mul_add(mix, a.coefficients[index])
            }),
        }
    }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("report") => report(),
        Some("transition") => transition_report(),
        Some("bench") => bench(),
        _ => {
            eprintln!("coefficient_mip_lab <report|transition|bench>");
            std::process::exit(2);
        }
    }
}

fn transition_report() {
    let drawn = drawn_curve();
    for shape in Shape::ALL {
        let spectrum = source_spectrum(shape, drawn);
        let frames =
            [2, 3, 6].map(|cap| UnclampedCurve::from_clamped(compile_projection(&spectrum, cap)));
        let raw = raw_curve(shape, drawn);
        for (name, samples) in [("slow", 65_536), ("fast", 512)] {
            let metrics = pitch_sweep(raw, &spectrum, frames, samples);
            println!(
                "pitch_sweep,shape={},speed={name},rms_error={:.9},peak_error={:.9},peak_step={:.9},peak_excess_step={:.9},finite={}",
                shape.name(),
                metrics.0,
                metrics.1,
                metrics.2,
                metrics.3,
                metrics.4
            );
        }
        let abrupt = abrupt_transition(raw, &spectrum, frames);
        println!(
            "pitch_jump,shape={},direct_peak_error={:.9},fade32_peak_error={:.9},direct_peak_excess_step={:.9},fade32_peak_excess_step={:.9},finite={}",
            shape.name(),
            abrupt.0,
            abrupt.1,
            abrupt.2,
            abrupt.3,
            abrupt.4
        );
        let selector = selector_jump(raw, frames);
        println!(
            "selector_jump,shape={},direct_same_phase_peak={:.9},fade32_step_peak={:.9},fade1024_step_peak={:.9}",
            shape.name(),
            selector.0,
            selector.1,
            selector.2
        );
    }
    println!(
        "narrow_storage,frames=3,bytes_per_curve={},bytes_per_16_curve_table={},atomic_f32_words_per_table={},eligibility=custom_mix_one_and_no_phase_warp,fallback=current_1x",
        3 * std::mem::size_of::<UnclampedCurve>(),
        16 * 3 * std::mem::size_of::<UnclampedCurve>(),
        16 * 3 * 64
    );
}

#[derive(Clone, Copy)]
enum Shape {
    Saw,
    Square,
    Pulse,
    Triangle,
    Drawn,
}

impl Shape {
    const ALL: [Self; 5] = [
        Self::Saw,
        Self::Square,
        Self::Pulse,
        Self::Triangle,
        Self::Drawn,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Saw => "saw",
            Self::Square => "square",
            Self::Pulse => "pulse37",
            Self::Triangle => "triangle",
            Self::Drawn => "drawn",
        }
    }

    fn source(self, phase: f32, drawn: WaveCurveRt) -> f32 {
        match self {
            Self::Saw => phase.mul_add(2.0, -1.0),
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Pulse => {
                if phase < 0.37 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Self::Drawn => drawn.eval(phase),
        }
    }
}

fn report() {
    let drawn = drawn_curve();
    for shape in Shape::ALL {
        let spectrum = source_spectrum(shape, drawn);
        let frames = CAPS.map(|cap| compile_projection(&spectrum, cap));
        let unclamped = frames.map(UnclampedCurve::from_clamped);
        for (cap, frame) in CAPS.into_iter().zip(frames) {
            let (rms, peak) = projection_error(frame, &spectrum, cap);
            println!(
                "projection,shape={},cap={cap},rms={rms:.9},peak={peak:.9}",
                shape.name()
            );
        }
        for (cap, frame) in CAPS.into_iter().zip(unclamped) {
            let metrics = unclamped_metrics(frame, &spectrum, cap);
            println!(
                "unclamped,shape={},cap={cap},rms={:.9},peak={:.9},wanted_error_db={:.3},unwanted_db={:.3},minimum={:.9},maximum={:.9},signal_rms={:.9},finite={}",
                shape.name(),
                metrics.0,
                metrics.1,
                metrics.2,
                metrics.3,
                metrics.4,
                metrics.5,
                metrics.6,
                metrics.7
            );
        }
        for band in 0..CAPS.len() - 1 {
            let (sweep_peak, transition_peak, coefficient_peak) = sweep_error(
                frames[band],
                frames[band + 1],
                &spectrum,
                CAPS[band],
                CAPS[band + 1],
            );
            println!(
                "sweep,shape={},caps={}-{},sample_step_peak={sweep_peak:.9},same_phase_transition_peak={transition_peak:.9},coefficient_error_peak={coefficient_peak:.9}",
                shape.name(),
                CAPS[band],
                CAPS[band + 1]
            );
            let transition = unclamped_transition(unclamped[band], unclamped[band + 1]);
            println!(
                "unclamped_sweep,shape={},caps={}-{},same_phase_transition_peak={transition:.9}",
                shape.name(),
                CAPS[band],
                CAPS[band + 1]
            );
        }
    }
    println!(
        "storage,frames={},bytes_per_frame={},bytes_per_curve={}",
        CAPS.len(),
        std::mem::size_of::<WaveCurveRt>(),
        CAPS.len() * std::mem::size_of::<WaveCurveRt>()
    );
}

fn bench() {
    let spectrum = source_spectrum(Shape::Drawn, drawn_curve());
    let frames = CAPS.map(|cap| compile_projection(&spectrum, cap));
    let unclamped = frames.map(UnclampedCurve::from_clamped);
    let mut times = Vec::new();
    let mut checksum = 0.0;
    for _ in 0..7 {
        let started = Instant::now();
        for index in 0..5_000_000 {
            checksum += black_box(frames[2].eval(black_box((index & 65_535) as f32 / 65_536.0)));
        }
        times.push(started.elapsed().as_nanos() as f64 / 5_000_000.0);
    }
    times.sort_by(f64::total_cmp);
    let mut x8_times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for index in 0..1_000_000 {
            let base = (index & 65_535) as f32 / 65_536.0;
            checksum += black_box(frames[2].eval8(f32x8::from(std::array::from_fn(|lane| {
                (base + lane as f32 * 0.071).fract()
            }))))
            .reduce_add();
        }
        x8_times.push(started.elapsed().as_nanos() as f64 / 8_000_000.0);
    }
    x8_times.sort_by(f64::total_cmp);
    let mut unclamped_times = Vec::new();
    let mut unclamped8_times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for index in 0..5_000_000 {
            checksum += black_box(unclamped[2].eval(black_box((index & 65_535) as f32 / 65_536.0)));
        }
        unclamped_times.push(started.elapsed().as_nanos() as f64 / 5_000_000.0);
        let started = Instant::now();
        for index in 0..1_000_000 {
            let base = (index & 65_535) as f32 / 65_536.0;
            checksum += black_box(unclamped[2].eval8(f32x8::from(std::array::from_fn(|lane| {
                (base + lane as f32 * 0.071).fract()
            }))))
            .reduce_add();
        }
        unclamped8_times.push(started.elapsed().as_nanos() as f64 / 8_000_000.0);
    }
    unclamped_times.sort_by(f64::total_cmp);
    unclamped8_times.sort_by(f64::total_cmp);
    let started = Instant::now();
    for index in 0..500_000 {
        checksum += black_box(interpolate(
            frames[1],
            frames[2],
            black_box((index & 255) as f32 / 255.0),
        ))
        .eval(black_box((index & 65_535) as f32 / 65_536.0));
    }
    let interpolation = started.elapsed().as_nanos() as f64 / 500_000.0;
    let raw = UnclampedCurve::from_clamped(drawn_curve());
    let mut narrow_times = Vec::new();
    let mut narrow8_times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for block in 0..78_125 {
            let selected = selected_curve(raw, unclamped, black_box(0.5 / 4.5));
            for lane in 0..64 {
                let index = block * 64 + lane;
                checksum += black_box(selected.eval((index & 65_535) as f32 / 65_536.0));
            }
        }
        narrow_times.push(started.elapsed().as_nanos() as f64 / 5_000_000.0);
        let started = Instant::now();
        for block in 0..125_000 {
            let selected = selected_curve(raw, unclamped, black_box(0.5 / 4.5));
            for group in 0..8 {
                let base = (block * 64 + group * 8) as f32 / 65_536.0;
                checksum += black_box(selected.eval8(f32x8::from(std::array::from_fn(|lane| {
                    (base + lane as f32 / 65_536.0).fract()
                }))))
                .reduce_add();
            }
        }
        narrow8_times.push(started.elapsed().as_nanos() as f64 / 8_000_000.0);
    }
    narrow_times.sort_by(f64::total_cmp);
    narrow8_times.sort_by(f64::total_cmp);
    println!(
        "bench,clamped_eval_ns={:.3},clamped_eval8_ns_per_sample={:.3},unclamped_eval_ns={:.3},unclamped_eval8_ns_per_sample={:.3},narrow_block64_scalar_ns={:.3},narrow_block64_eval8_ns_per_sample={:.3},coefficient_interpolate_plus_eval_ns={interpolation:.3},checksum={checksum:.9}",
        times[times.len() / 2],
        x8_times[x8_times.len() / 2],
        unclamped_times[unclamped_times.len() / 2],
        unclamped8_times[unclamped8_times.len() / 2],
        narrow_times[narrow_times.len() / 2],
        narrow8_times[narrow8_times.len() / 2]
    );
}

fn selected_curve(
    raw: UnclampedCurve,
    frames: [UnclampedCurve; 5],
    phase_step: f32,
) -> UnclampedCurve {
    let cap = 0.5 / phase_step;
    if cap <= 3.0 {
        UnclampedCurve::interpolate(frames[0], frames[1], (cap - 2.0).clamp(0.0, 1.0))
    } else if cap < 6.0 {
        UnclampedCurve::interpolate(frames[1], frames[2], ((cap - 3.0) / 3.0).clamp(0.0, 1.0))
    } else if cap < 8.0 {
        UnclampedCurve::interpolate(frames[2], raw, (cap - 6.0) * 0.5)
    } else {
        raw
    }
}

fn source_spectrum(shape: Shape, drawn: WaveCurveRt) -> Vec<Complex> {
    let mut samples = (0..GRID)
        .map(|index| {
            Complex::new(
                f64::from(shape.source(index as f32 / GRID as f32, drawn)),
                0.0,
            )
        })
        .collect::<Vec<_>>();
    fft(&mut samples, false);
    for value in &mut samples {
        *value /= GRID as f64;
    }
    samples
}

fn projected(spectrum: &[Complex], cap: usize, phase: f64) -> f32 {
    let mut value = spectrum[0].re;
    for harmonic in 1..=cap {
        value += 2.0
            * (spectrum[harmonic]
                * Complex::from_polar(1.0, std::f64::consts::TAU * harmonic as f64 * phase))
            .re;
    }
    value as f32
}

fn compile_projection(spectrum: &[Complex], cap: usize) -> WaveCurveRt {
    let mut coefficients = [0.0; 64];
    for segment in 0..SEGMENTS {
        let phase = segment as f64 / SEGMENTS as f64;
        let step = 1.0 / SEGMENTS as f64;
        let y0 = projected(spectrum, cap, phase);
        let y1 = projected(spectrum, cap, phase + step / 3.0);
        let y2 = projected(spectrum, cap, phase + step * 2.0 / 3.0);
        let y3 = projected(spectrum, cap, phase + step);
        let p = y1 - y0;
        let q = y2 - y0;
        let r = y3 - y0;
        let values = [
            4.5_f32.mul_add(r, 13.5 * (p - q)),
            (-4.5_f32).mul_add(r, (-22.5_f32).mul_add(p, 18.0 * q)),
            9.0_f32.mul_add(p, (-4.5_f32).mul_add(q, r)),
            y0,
        ];
        for (plane, value) in values.into_iter().enumerate() {
            coefficients[coefficient_index(segment, plane)] = value;
        }
    }
    WaveCurveRt::from_coefficients(coefficients)
}

fn raw_curve(shape: Shape, drawn: WaveCurveRt) -> UnclampedCurve {
    if matches!(shape, Shape::Drawn) {
        return UnclampedCurve::from_clamped(drawn);
    }
    let mut coefficients = [0.0; 64];
    for segment in 0..SEGMENTS {
        let start = segment as f32 / SEGMENTS as f32;
        let (slope, value) = match shape {
            Shape::Saw => (2.0 / SEGMENTS as f32, start.mul_add(2.0, -1.0)),
            Shape::Square => (0.0, if segment < 8 { 1.0 } else { -1.0 }),
            Shape::Pulse => (0.0, if segment < 6 { 1.0 } else { -1.0 }),
            Shape::Triangle if segment < 8 => (4.0 / SEGMENTS as f32, start.mul_add(4.0, -1.0)),
            Shape::Triangle => (-4.0 / SEGMENTS as f32, 3.0 - 4.0 * start),
            Shape::Drawn => unreachable!(),
        };
        coefficients[coefficient_index(segment, 2)] = slope;
        coefficients[coefficient_index(segment, 3)] = value;
    }
    UnclampedCurve { coefficients }
}

fn projection_error(curve: WaveCurveRt, spectrum: &[Complex], cap: usize) -> (f64, f32) {
    let mut square = 0.0;
    let mut peak = 0.0_f32;
    for index in 0..GRID {
        let phase = index as f32 / GRID as f32;
        let error = curve.eval(phase) - projected(spectrum, cap, f64::from(phase));
        square += f64::from(error) * f64::from(error);
        peak = peak.max(error.abs());
    }
    ((square / GRID as f64).sqrt(), peak)
}

fn unclamped_metrics(
    curve: UnclampedCurve,
    spectrum: &[Complex],
    cap: usize,
) -> (f64, f32, f64, f64, f32, f32, f64, bool) {
    let mut square = 0.0;
    let mut signal_square = 0.0;
    let mut peak = 0.0_f32;
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    let mut samples = Vec::with_capacity(GRID);
    for index in 0..GRID {
        let phase = index as f32 / GRID as f32;
        let sample = curve.eval(phase);
        let error = sample - projected(spectrum, cap, f64::from(phase));
        square += f64::from(error) * f64::from(error);
        signal_square += f64::from(sample) * f64::from(sample);
        peak = peak.max(error.abs());
        minimum = minimum.min(sample);
        maximum = maximum.max(sample);
        samples.push(Complex::new(f64::from(sample), 0.0));
    }
    fft(&mut samples, false);
    for value in &mut samples {
        *value /= GRID as f64;
    }
    let wanted_energy = (1..=cap)
        .map(|harmonic| 2.0 * spectrum[harmonic].norm_sqr())
        .sum::<f64>();
    let wanted_error = (1..=cap)
        .map(|harmonic| 2.0 * (samples[harmonic] - spectrum[harmonic]).norm_sqr())
        .sum::<f64>();
    let unwanted = (cap + 1..GRID / 2)
        .map(|harmonic| 2.0 * samples[harmonic].norm_sqr())
        .sum::<f64>();
    (
        (square / GRID as f64).sqrt(),
        peak,
        10.0 * (wanted_error / wanted_energy.max(1.0e-30)).log10(),
        10.0 * (unwanted / wanted_energy.max(1.0e-30)).log10(),
        minimum,
        maximum,
        (signal_square / GRID as f64).sqrt(),
        minimum.is_finite() && maximum.is_finite(),
    )
}

fn unclamped_transition(low: UnclampedCurve, high: UnclampedCurve) -> f32 {
    let before = UnclampedCurve::interpolate(low, high, 0.5 - 1.0 / 16_384.0);
    let after = UnclampedCurve::interpolate(low, high, 0.5 + 1.0 / 16_384.0);
    (0..GRID).fold(0.0_f32, |peak, index| {
        let phase = index as f32 / GRID as f32;
        peak.max((after.eval(phase) - before.eval(phase)).abs())
    })
}

fn narrow_sample(
    raw: UnclampedCurve,
    frames: [UnclampedCurve; 3],
    phase: f32,
    phase_step: f32,
) -> f32 {
    let cap = (0.5 / phase_step).max(0.0);
    let projected = if cap <= 3.0 {
        UnclampedCurve::interpolate(frames[0], frames[1], (cap - 2.0).clamp(0.0, 1.0)).eval(phase)
    } else {
        UnclampedCurve::interpolate(frames[1], frames[2], ((cap - 3.0) / 3.0).clamp(0.0, 1.0))
            .eval(phase)
    };
    if cap < 6.0 {
        projected
    } else if cap < 8.0 {
        let mix = (cap - 6.0) * 0.5;
        (raw.eval(phase) - projected).mul_add(mix, projected)
    } else {
        raw.eval(phase)
    }
}

fn pitch_sweep(
    raw: UnclampedCurve,
    spectrum: &[Complex],
    frames: [UnclampedCurve; 3],
    samples: usize,
) -> (f64, f32, f32, f32, bool) {
    let mut phase = 0.173_f32;
    let mut previous = 0.0_f32;
    let mut previous_ideal = 0.0_f32;
    let mut square = 0.0;
    let mut peak_error = 0.0_f32;
    let mut peak_step = 0.0_f32;
    let mut peak_excess_step = 0.0_f32;
    let mut finite = true;
    for index in 0..samples {
        let sweep = index as f32 / (samples - 1) as f32;
        let cap = 10.0_f32 * (0.2_f32).powf(sweep);
        let phase_step = 0.5 / cap;
        let sample = narrow_sample(raw, frames, phase, phase_step);
        let ideal = projected(spectrum, cap.floor() as usize, f64::from(phase));
        let error = sample - ideal;
        square += f64::from(error) * f64::from(error);
        peak_error = peak_error.max(error.abs());
        if index != 0 {
            let step = sample - previous;
            let ideal_step = ideal - previous_ideal;
            peak_step = peak_step.max(step.abs());
            peak_excess_step = peak_excess_step.max((step - ideal_step).abs());
        }
        finite &= sample.is_finite();
        previous = sample;
        previous_ideal = ideal;
        phase = (phase + phase_step).fract();
    }
    (
        (square / samples as f64).sqrt(),
        peak_error,
        peak_step,
        peak_excess_step,
        finite,
    )
}

fn abrupt_transition(
    raw: UnclampedCurve,
    spectrum: &[Complex],
    frames: [UnclampedCurve; 3],
) -> (f32, f32, f32, f32, bool) {
    let mut phase = 0.173_f32;
    let mut direct_previous = 0.0_f32;
    let mut fade_previous = 0.0_f32;
    let mut ideal_previous = 0.0_f32;
    let mut direct_peak = 0.0_f32;
    let mut fade_peak = 0.0_f32;
    let mut direct_excess = 0.0_f32;
    let mut fade_excess = 0.0_f32;
    let mut finite = true;
    for index in 0..256 {
        let old_step = 0.5 / 8.5;
        let new_step = 0.5 / 5.5;
        let phase_step = if index < 128 { old_step } else { new_step };
        let direct = narrow_sample(raw, frames, phase, phase_step);
        let fade = if (128..160).contains(&index) {
            let mix = (index - 128) as f32 / 31.0;
            let old = narrow_sample(raw, frames, phase, old_step);
            (direct - old).mul_add(mix, old)
        } else {
            direct
        };
        let cap = (0.5 / phase_step).floor() as usize;
        let ideal = projected(spectrum, cap, f64::from(phase));
        direct_peak = direct_peak.max((direct - ideal).abs());
        fade_peak = fade_peak.max((fade - ideal).abs());
        if index != 0 {
            let ideal_step = ideal - ideal_previous;
            direct_excess = direct_excess.max(((direct - direct_previous) - ideal_step).abs());
            fade_excess = fade_excess.max(((fade - fade_previous) - ideal_step).abs());
        }
        finite &= direct.is_finite() && fade.is_finite();
        direct_previous = direct;
        fade_previous = fade;
        ideal_previous = ideal;
        phase = (phase + phase_step).fract();
    }
    (direct_peak, fade_peak, direct_excess, fade_excess, finite)
}

fn selector_jump(raw: UnclampedCurve, frames: [UnclampedCurve; 3]) -> (f32, f32, f32) {
    let old_step = 0.5 / 8.5;
    let new_step = 0.5 / 5.5;
    let mut direct = 0.0_f32;
    let mut first = 0.0_f32;
    let mut fade1024 = 0.0_f32;
    for index in 0..GRID {
        let phase = index as f32 / GRID as f32;
        let old = narrow_sample(raw, frames, phase, old_step);
        let new = narrow_sample(raw, frames, phase, new_step);
        let difference = new - old;
        direct = direct.max(difference.abs());
        first = first.max((difference / 32.0).abs());
        fade1024 = fade1024.max((difference / 1024.0).abs());
    }
    (direct, first, fade1024)
}

fn sweep_error(
    low: WaveCurveRt,
    high: WaveCurveRt,
    spectrum: &[Complex],
    low_cap: usize,
    high_cap: usize,
) -> (f32, f32, f32) {
    let mut previous = interpolate(low, high, 0.0).eval(0.173);
    let mut sample_step_peak = 0.0_f32;
    let mut coefficient_error_peak = 0.0_f32;
    for index in 1..=16_384 {
        let mix = index as f32 / 16_384.0;
        let phase = (0.173 + index as f32 * 0.007_31).fract();
        let curve = interpolate(low, high, mix);
        let sample = curve.eval(phase);
        sample_step_peak = sample_step_peak.max((sample - previous).abs());
        previous = sample;
        let ideal = projected(spectrum, low_cap, f64::from(phase)) * (1.0 - mix)
            + projected(spectrum, high_cap, f64::from(phase)) * mix;
        coefficient_error_peak = coefficient_error_peak.max((sample - ideal).abs());
    }
    let before = interpolate(low, high, 0.5 - 1.0 / 16_384.0);
    let after = interpolate(low, high, 0.5 + 1.0 / 16_384.0);
    let transition_peak = (0..GRID).fold(0.0_f32, |peak, index| {
        let phase = index as f32 / GRID as f32;
        peak.max((after.eval(phase) - before.eval(phase)).abs())
    });
    (sample_step_peak, transition_peak, coefficient_error_peak)
}

fn interpolate(a: WaveCurveRt, b: WaveCurveRt, mix: f32) -> WaveCurveRt {
    WaveCurveRt::interpolate(a, b, mix)
}

fn coefficient_index(segment: usize, plane: usize) -> usize {
    if cfg!(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    )) {
        plane * SEGMENTS + segment
    } else {
        segment * 4 + plane
    }
}

fn drawn_curve() -> WaveCurveRt {
    WaveCurveData {
        knots: vec![
            WaveKnot {
                phase: 0.0,
                value: -0.72,
                curve: 0.8,
                curve_x: -0.25,
            },
            WaveKnot {
                phase: 0.13,
                value: 0.91,
                curve: -0.45,
                curve_x: 0.3,
            },
            WaveKnot {
                phase: 0.29,
                value: 0.18,
                curve: 1.2,
                curve_x: -0.15,
            },
            WaveKnot {
                phase: 0.52,
                value: 0.63,
                curve: 0.0,
                curve_x: 0.0,
            },
            WaveKnot {
                phase: 0.71,
                value: -0.88,
                curve: -0.7,
                curve_x: 0.2,
            },
            WaveKnot {
                phase: 0.89,
                value: -0.24,
                curve: 0.35,
                curve_x: -0.1,
            },
        ],
    }
    .compile_rt()
}
