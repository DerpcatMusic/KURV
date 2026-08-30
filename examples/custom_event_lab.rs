#![allow(dead_code)]

#[path = "../src/dsp.rs"]
mod dsp;
#[path = "../src/oversampling.rs"]
mod oversampling;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::fs::File;
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::time::Instant;

use oversampling::StereoOversampler;
use truce_simd::simd::f32x8;
use wave_curve::WaveCurveRt;
use wide::CmpLt;

const EVENTS: [(f32, f32); 2] = [(0.25, -8.0), (0.75, 8.0)];
const SINC12_RESOLUTION: f32 = 8.0;
const SINC12_HALF_SUPPORT: f32 = 6.0;
const SINC12_BLAMP: [f32; 49] = [
    1.0415219126e-1,
    4.9441577878e-2,
    9.8528237592e-3,
    -1.5791166727e-2,
    -2.9345882348e-2,
    -3.3149404515e-2,
    -2.9801412683e-2,
    -2.1928143039e-2,
    -1.1960511289e-2,
    -1.9497995658e-3,
    6.5606123328e-3,
    1.2597076835e-2,
    1.5749701459e-2,
    1.6105851689e-2,
    1.4137508083e-2,
    1.0561466277e-2,
    6.1930214174e-3,
    1.8124499778e-3,
    -1.9402824861e-3,
    -4.6328209996e-3,
    -6.0664136822e-3,
    -6.2634968751e-3,
    -5.4248452159e-3,
    -3.8675429316e-3,
    -1.9562205633e-3,
    -3.9145951782e-5,
    1.6017679419e-3,
    2.7815709043e-3,
    3.4219850806e-3,
    3.5423246596e-3,
    3.2364701748e-3,
    2.6422528047e-3,
    1.9098012281e-3,
    1.1744379627e-3,
    5.3792036913e-4,
    5.9644779615e-5,
    -2.4267611193e-4,
    -3.8501183502e-4,
    -4.0480331726e-4,
    -3.4784967619e-4,
    -2.5710169527e-4,
    -1.6519140094e-4,
    -9.1343871955e-5,
    -4.2213100808e-5,
    -1.5386366223e-5,
    -3.9424798785e-6,
    -5.4543297394e-7,
    -1.7449257872e-8,
    0.0,
];
const SINC24_BLAMP: [f32; 97] = [
    1.0200849157e-1,
    4.7302086628e-2,
    7.7376945742e-3,
    -1.7830260020e-2,
    -3.1216153576e-2,
    -3.4721609222e-2,
    -3.0929317861e-2,
    -2.2479617576e-2,
    -1.1853831871e-2,
    -1.1868438430e-3,
    7.8727711590e-3,
    1.4242934222e-2,
    1.7423870849e-2,
    1.7452504361e-2,
    1.4805350909e-2,
    1.0265221627e-2,
    4.7702569854e-3,
    -7.3543458908e-4,
    -5.4319545309e-3,
    -8.7208108588e-3,
    -1.0280598969e-2,
    -1.0078602796e-2,
    -8.3416076457e-3,
    -5.4941923888e-3,
    -2.0762668422e-3,
    1.3467235718e-3,
    4.2677799978e-3,
    6.3054027544e-3,
    7.2459725389e-3,
    7.0578716026e-3,
    5.8780526760e-3,
    3.9755924428e-3,
    1.6996269974e-3,
    -5.7936843232e-4,
    -2.5258429286e-3,
    -3.8850356370e-3,
    -4.5127208137e-3,
    -4.3859669805e-3,
    -3.5950748138e-3,
    -2.3196033588e-3,
    -7.9347490739e-4,
    7.3468618780e-4,
    2.0396724649e-3,
    2.9507586389e-3,
    3.3717497856e-3,
    3.2880528853e-3,
    2.7609323204e-3,
    1.9110169757e-3,
    8.9455084186e-4,
    -1.2335338039e-4,
    -9.9414625567e-4,
    -1.6062560237e-3,
    -1.8978986491e-3,
    -1.8611039188e-3,
    -1.5372328390e-3,
    -1.0055349242e-3,
    -3.6720534858e-4,
    2.7215631159e-4,
    8.1883612168e-4,
    1.2036852557e-3,
    1.3896382251e-3,
    1.3734874256e-3,
    1.1822901137e-3,
    8.6558151370e-4,
    4.8509365171e-4,
    1.0387776193e-4,
    -2.2339862162e-4,
    -4.5760519017e-4,
    -5.7895070728e-4,
    -5.8726639694e-4,
    -4.9919131439e-4,
    -3.4312760486e-4,
    -1.5308306688e-4,
    3.7448492946e-5,
    2.0058176090e-4,
    3.1742981930e-4,
    3.7949221196e-4,
    3.8812527972e-4,
    3.5249314135e-4,
    2.8659836059e-4,
    2.0605770578e-4,
    1.2522646372e-4,
    5.5113116759e-5,
    2.3082493175e-6,
    -3.1074201531e-5,
    -4.6634945171e-5,
    -4.8420462918e-5,
    -4.1501729508e-5,
    -3.0717628981e-5,
    -1.9803793282e-5,
    -1.1000918443e-5,
    -5.1111155370e-6,
    -1.8739254857e-6,
    -4.8316949196e-7,
    -6.7283468639e-8,
    -2.1670703954e-9,
    0.0,
];

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("render") if args.len() == 5 => render(&args[1..]),
        Some("bench") if args.len() == 6 => bench(&args[1..]),
        _ => {
            eprintln!(
                "custom_event_lab check | render <raw1|events1|sinc12|sinc24|raw2> <fft-bin> <samples> <output.f32> | bench <raw1|events1|sinc12|sinc24|raw2> <fft-bin> <lanes> <frames> <repeats>"
            );
            std::process::exit(2);
        }
    }
}

fn check() {
    let curve = WaveCurveRt::default();
    let step = 601.0 / 65_536.0;
    let phases = f32x8::from([0.249, 0.251, 0.50, 0.749, 0.751, 0.1, 0.9, 0.0]);
    let vector: [f32; 8] = event_sample8(&curve, phases, f32x8::splat(step)).into();
    let scalar_phases: [f32; 8] = phases.into();
    for (phase, actual) in scalar_phases.into_iter().zip(vector) {
        let expected = event_sample(&curve, phase, step);
        assert!(
            (actual - expected).abs() < 2.0e-6,
            "{phase}: {actual} != {expected}"
        );
    }
    let vector: [f32; 8] = sinc12_sample8(&curve, phases, f32x8::splat(step)).into();
    for (phase, actual) in scalar_phases.into_iter().zip(vector) {
        let expected = sinc12_sample(&curve, phase, step);
        assert!(
            (actual - expected).abs() < 2.0e-6,
            "sinc12 {phase}: {actual} != {expected}"
        );
    }
    let vector: [f32; 8] = sinc24_sample8(&curve, phases, f32x8::splat(step)).into();
    for (phase, actual) in scalar_phases.into_iter().zip(vector) {
        let expected = sinc24_sample(&curve, phase, step);
        assert!(
            (actual - expected).abs() < 2.0e-6,
            "sinc24 {phase}: {actual} != {expected}"
        );
    }
    println!("custom-event scalar/SIMD check passed");
}

fn render(args: &[String]) {
    let (kind, factor) = mode(&args[0]);
    let fft_bin = parse_usize(&args[1]);
    let samples = parse_usize(&args[2]);
    let internal_samples = samples * usize::from(factor);
    let step = fft_bin as f32 / internal_samples as f32;
    let curve = WaveCurveRt::default();
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(factor);
    oversampler.set_spline_correction_immediate(factor == 2);
    let mut output = BufWriter::new(File::create(&args[3]).expect("create output"));
    for frame in 0..samples * 2 {
        for subframe in 0..usize::from(factor) {
            let index = frame * usize::from(factor) + subframe;
            let phase = ((index * fft_bin) % internal_samples) as f32 / internal_samples as f32;
            let sample = match kind {
                1 => event_sample(&curve, phase, step),
                2 => sinc12_sample(&curve, phase, step),
                3 => sinc24_sample(&curve, phase, step),
                _ => curve.eval(phase),
            };
            oversampler.push(sample, sample);
        }
        let sample = oversampler.output().0;
        if frame >= samples {
            output
                .write_all(&sample.to_le_bytes())
                .expect("write sample");
        }
    }
}

fn bench(args: &[String]) {
    let (kind, factor) = mode(&args[0]);
    let fft_bin = parse_usize(&args[1]);
    let lanes = parse_usize(&args[2]);
    let frames = parse_usize(&args[3]);
    let repeats = parse_usize(&args[4]);
    assert!(lanes > 0 && lanes % 8 == 0);
    let step = f32x8::splat(fft_bin as f32 / 65_536.0 / f32::from(factor));
    let curve = WaveCurveRt::default();
    let groups = lanes / 8;
    let initial = (0..groups)
        .map(|group| {
            f32x8::from(std::array::from_fn(|lane| {
                ((group * 8 + lane) as f32 / lanes as f32).fract()
            }))
        })
        .collect::<Vec<_>>();
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0_f32;
    for _ in 0..repeats {
        let mut phases = initial.clone();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(factor == 2);
        for _ in 0..4096 {
            run_frame(
                kind,
                factor,
                &curve,
                step,
                &mut phases,
                &mut oversampler,
                &mut checksum,
            );
        }
        let start = Instant::now();
        for _ in 0..frames {
            run_frame(
                kind,
                factor,
                &curve,
                step,
                &mut phases,
                &mut oversampler,
                &mut checksum,
            );
        }
        measurements.push(start.elapsed().as_nanos() as f64 / frames as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "mode={},fft_bin={fft_bin},lanes={lanes},frames={frames},repeats={repeats},median_ns_per_frame={:.3},min_ns_per_frame={:.3},max_ns_per_frame={:.3},checksum={checksum:.9}",
        args[0],
        measurements[repeats / 2],
        measurements[0],
        measurements[repeats - 1]
    );
}

#[inline(always)]
fn run_frame(
    kind: u8,
    factor: u8,
    curve: &WaveCurveRt,
    step: f32x8,
    phases: &mut [f32x8],
    oversampler: &mut StereoOversampler,
    checksum: &mut f32,
) {
    for _ in 0..factor {
        let mut mixed = 0.0;
        for phase in &mut *phases {
            let sample = match kind {
                1 => event_sample8(curve, *phase, step),
                2 => sinc12_sample8(curve, *phase, step),
                3 => sinc24_sample8(curve, *phase, step),
                _ => curve.eval8(*phase),
            };
            *phase = wrap8(*phase + step);
            let values: [f32; 8] = black_box(sample).into();
            mixed += values.into_iter().sum::<f32>();
        }
        oversampler.push(mixed, mixed);
    }
    *checksum += black_box(oversampler.output().0);
}

#[inline(always)]
fn event_sample(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    EVENTS
        .iter()
        .fold(curve.eval(phase), |sample, &(event, slope_jump)| {
            slope_jump.mul_add(
                step * blamp_residual(signed_distance(phase, event) / step),
                sample,
            )
        })
}

#[inline(always)]
fn event_sample8(curve: &WaveCurveRt, phase: f32x8, step: f32x8) -> f32x8 {
    EVENTS
        .iter()
        .fold(curve.eval8(phase), |sample, &(event, slope_jump)| {
            let position = signed_distance8(phase, f32x8::splat(event)) / step;
            (step * f32x8::splat(slope_jump)).mul_add(blamp_residual8(position), sample)
        })
}

#[inline(always)]
fn sinc12_sample(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    EVENTS
        .iter()
        .fold(curve.eval(phase), |sample, &(event, slope_jump)| {
            let correction = [-1.0, 0.0, 1.0].into_iter().fold(0.0, |sum, cycle| {
                sum + sinc12_residual((phase - event + cycle) / step)
            });
            slope_jump.mul_add(step * correction, sample)
        })
}

#[inline(always)]
fn sinc12_sample8(curve: &WaveCurveRt, phase: f32x8, step: f32x8) -> f32x8 {
    EVENTS
        .iter()
        .fold(curve.eval8(phase), |sample, &(event, slope_jump)| {
            // ponytail: three periodic images cover this experiment through 8 kHz;
            // use an event ring if a production path must cover higher fundamentals.
            let correction = [-1.0, 0.0, 1.0]
                .into_iter()
                .fold(f32x8::ZERO, |sum, cycle| {
                    sum + sinc12_residual8((phase - f32x8::splat(event - cycle)) / step)
                });
            (step * f32x8::splat(slope_jump)).mul_add(correction, sample)
        })
}

#[inline(always)]
fn sinc12_residual(position: f32) -> f32 {
    let scaled = position.abs() * SINC12_RESOLUTION;
    if scaled >= (SINC12_BLAMP.len() - 1) as f32 {
        return 0.0;
    }
    let index = scaled as usize;
    (SINC12_BLAMP[index + 1] - SINC12_BLAMP[index])
        .mul_add(scaled - index as f32, SINC12_BLAMP[index])
}

#[inline(always)]
fn sinc12_residual8(position: f32x8) -> f32x8 {
    let distance = position.abs();
    let active = distance.cmp_lt(f32x8::splat(SINC12_HALF_SUPPORT));
    if !active.any() {
        return f32x8::ZERO;
    }
    let scaled: [f32; 8] = (distance * f32x8::splat(SINC12_RESOLUTION)).into();
    f32x8::from(std::array::from_fn(|lane| {
        if scaled[lane] >= (SINC12_BLAMP.len() - 1) as f32 {
            return 0.0;
        }
        let index = scaled[lane] as usize;
        (SINC12_BLAMP[index + 1] - SINC12_BLAMP[index])
            .mul_add(scaled[lane] - index as f32, SINC12_BLAMP[index])
    }))
}

#[inline(always)]
fn sinc24_sample(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    EVENTS
        .iter()
        .fold(curve.eval(phase), |sample, &(event, slope_jump)| {
            let correction = [-2.0, -1.0, 0.0, 1.0, 2.0]
                .into_iter()
                .fold(0.0, |sum, cycle| {
                    sum + table_residual((phase - event + cycle) / step, &SINC24_BLAMP)
                });
            slope_jump.mul_add(step * correction, sample)
        })
}

#[inline(always)]
fn sinc24_sample8(curve: &WaveCurveRt, phase: f32x8, step: f32x8) -> f32x8 {
    EVENTS
        .iter()
        .fold(curve.eval8(phase), |sample, &(event, slope_jump)| {
            let correction =
                [-2.0, -1.0, 0.0, 1.0, 2.0]
                    .into_iter()
                    .fold(f32x8::ZERO, |sum, cycle| {
                        sum + table_residual8(
                            (phase - f32x8::splat(event - cycle)) / step,
                            &SINC24_BLAMP,
                        )
                    });
            (step * f32x8::splat(slope_jump)).mul_add(correction, sample)
        })
}

#[inline(always)]
fn table_residual(position: f32, table: &[f32]) -> f32 {
    let scaled = position.abs() * SINC12_RESOLUTION;
    if scaled >= (table.len() - 1) as f32 {
        return 0.0;
    }
    let index = scaled as usize;
    (table[index + 1] - table[index]).mul_add(scaled - index as f32, table[index])
}

#[inline(always)]
fn table_residual8(position: f32x8, table: &[f32]) -> f32x8 {
    let scaled: [f32; 8] = (position.abs() * f32x8::splat(SINC12_RESOLUTION)).into();
    f32x8::from(std::array::from_fn(|lane| {
        if scaled[lane] >= (table.len() - 1) as f32 {
            return 0.0;
        }
        let index = scaled[lane] as usize;
        (table[index + 1] - table[index]).mul_add(scaled[lane] - index as f32, table[index])
    }))
}

#[inline(always)]
fn blamp_residual(position: f32) -> f32 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 1.0 {
        0.018_896_732_f32
            .mul_add(distance, -0.068_349_12)
            .mul_add(distance, -0.000_122_838_29)
            .mul_add(distance, 0.313_372_55)
            .mul_add(distance, -0.5)
            .mul_add(distance, 0.248_045_86)
    } else {
        let tail = 2.0 - distance;
        0.005_821_323_f32
            .mul_add(tail, 0.006_685_827_4)
            .mul_add(tail, -0.001_985_740_6)
            .mul_add(tail, 0.001_321_771_3)
            * tail
            * tail
    }
}

#[inline(always)]
fn blamp_residual8(position: f32x8) -> f32x8 {
    let distance = position.abs();
    let event = distance.cmp_lt(f32x8::splat(2.0));
    if !event.any() {
        return f32x8::ZERO;
    }
    let inner = f32x8::splat(0.018_896_732)
        .mul_add(distance, f32x8::splat(-0.068_349_12))
        .mul_add(distance, f32x8::splat(-0.000_122_838_29))
        .mul_add(distance, f32x8::splat(0.313_372_55))
        .mul_add(distance, f32x8::splat(-0.5))
        .mul_add(distance, f32x8::splat(0.248_045_86));
    let tail = f32x8::splat(2.0) - distance;
    let outer = f32x8::splat(0.005_821_323)
        .mul_add(tail, f32x8::splat(0.006_685_827_4))
        .mul_add(tail, f32x8::splat(-0.001_985_740_6))
        .mul_add(tail, f32x8::splat(0.001_321_771_3))
        * tail
        * tail;
    event.blend(distance.cmp_lt(f32x8::ONE).blend(inner, outer), f32x8::ZERO)
}

#[inline(always)]
fn signed_distance(phase: f32, event: f32) -> f32 {
    let distance = phase - event;
    if distance < -0.5 {
        distance + 1.0
    } else if distance > 0.5 {
        distance - 1.0
    } else {
        distance
    }
}

#[inline(always)]
fn signed_distance8(phase: f32x8, event: f32x8) -> f32x8 {
    let distance = phase - event;
    distance
        .cmp_lt(f32x8::splat(-0.5))
        .blend(distance + f32x8::ONE, distance)
        .cmp_lt(f32x8::splat(0.5))
        .blend(distance, distance - f32x8::ONE)
}

#[inline(always)]
fn wrap(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}

#[inline(always)]
fn wrap8(phase: f32x8) -> f32x8 {
    phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE)
}

fn mode(value: &str) -> (u8, u8) {
    match value {
        "raw1" => (0, 1),
        "events1" => (1, 1),
        "sinc12" => (2, 1),
        "sinc24" => (3, 1),
        "raw2" => (0, 2),
        _ => panic!("mode must be raw1, events1, sinc12, sinc24, or raw2"),
    }
}

fn parse_usize(value: &str) -> usize {
    value.parse().expect("unsigned integer")
}
