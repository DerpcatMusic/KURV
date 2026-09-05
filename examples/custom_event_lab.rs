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
    1.041_521_912_6e-1,
    4.944_157_787_8e-2,
    9.852_823_759_2e-3,
    -1.579_116_672_7e-2,
    -2.934_588_234_8e-2,
    -3.314_940_451_5e-2,
    -2.980_141_268_3e-2,
    -2.192_814_303_9e-2,
    -1.196_051_128_9e-2,
    -1.949_799_565_8e-3,
    6.560_612_332_8e-3,
    1.259_707_683_5e-2,
    1.574_970_145_9e-2,
    1.610_585_168_9e-2,
    1.413_750_808_3e-2,
    1.056_146_627_7e-2,
    6.193_021_417_4e-3,
    1.812_449_977_8e-3,
    -1.940_282_486_1e-3,
    -4.632_820_999_6e-3,
    -6.066_413_682_2e-3,
    -6.263_496_875_1e-3,
    -5.424_845_215_9e-3,
    -3.867_542_931_6e-3,
    -1.956_220_563_3e-3,
    -3.914_595_178_2e-5,
    1.601_767_941_9e-3,
    2.781_570_904_3e-3,
    3.421_985_080_6e-3,
    3.542_324_659_6e-3,
    3.236_470_174_8e-3,
    2.642_252_804_7e-3,
    1.909_801_228_1e-3,
    1.174_437_962_7e-3,
    5.379_203_691_3e-4,
    5.964_477_961_5e-5,
    -2.426_761_119_3e-4,
    -3.850_118_350_2e-4,
    -4.048_033_172_6e-4,
    -3.478_496_761_9e-4,
    -2.571_016_952_7e-4,
    -1.651_914_009_4e-4,
    -9.134_387_195_5e-5,
    -4.221_310_080_8e-5,
    -1.538_636_622_3e-5,
    -3.942_479_878_5e-6,
    -5.454_329_739_4e-7,
    -1.744_925_787_2e-8,
    0.0,
];
const SINC24_BLAMP: [f32; 97] = [
    1.020_084_915_7e-1,
    4.730_208_662_8e-2,
    7.737_694_574_2e-3,
    -1.783_026_002_0e-2,
    -3.121_615_357_6e-2,
    -3.472_160_922_2e-2,
    -3.092_931_786_1e-2,
    -2.247_961_757_6e-2,
    -1.185_383_187_1e-2,
    -1.186_843_843_0e-3,
    7.872_771_159_0e-3,
    1.424_293_422_2e-2,
    1.742_387_084_9e-2,
    1.745_250_436_1e-2,
    1.480_535_090_9e-2,
    1.026_522_162_7e-2,
    4.770_256_985_4e-3,
    -7.354_345_890_8e-4,
    -5.431_954_530_9e-3,
    -8.720_810_858_8e-3,
    -1.028_059_896_9e-2,
    -1.007_860_279_6e-2,
    -8.341_607_645_7e-3,
    -5.494_192_388_8e-3,
    -2.076_266_842_2e-3,
    1.346_723_571_8e-3,
    4.267_779_997_8e-3,
    6.305_402_754_4e-3,
    7.245_972_538_9e-3,
    7.057_871_602_6e-3,
    5.878_052_676_0e-3,
    3.975_592_442_8e-3,
    1.699_626_997_4e-3,
    -5.793_684_323_2e-4,
    -2.525_842_928_6e-3,
    -3.885_035_637_0e-3,
    -4.512_720_813_7e-3,
    -4.385_966_980_5e-3,
    -3.595_074_813_8e-3,
    -2.319_603_358_8e-3,
    -7.934_749_073_9e-4,
    7.346_861_878_0e-4,
    2.039_672_464_9e-3,
    2.950_758_638_9e-3,
    3.371_749_785_6e-3,
    3.288_052_885_3e-3,
    2.760_932_320_4e-3,
    1.911_016_975_7e-3,
    8.945_508_418_6e-4,
    -1.233_533_803_9e-4,
    -9.941_462_556_7e-4,
    -1.606_256_023_7e-3,
    -1.897_898_649_1e-3,
    -1.861_103_918_8e-3,
    -1.537_232_839_0e-3,
    -1.005_534_924_2e-3,
    -3.672_053_485_8e-4,
    2.721_563_115_9e-4,
    8.188_361_216_8e-4,
    1.203_685_255_7e-3,
    1.389_638_225_1e-3,
    1.373_487_425_6e-3,
    1.182_290_113_7e-3,
    8.655_815_137_0e-4,
    4.850_936_517_1e-4,
    1.038_777_619_3e-4,
    -2.233_986_216_2e-4,
    -4.576_051_901_7e-4,
    -5.789_507_072_8e-4,
    -5.872_663_969_4e-4,
    -4.991_913_143_9e-4,
    -3.431_276_048_6e-4,
    -1.530_830_668_8e-4,
    3.744_849_294_6e-5,
    2.005_817_609_0e-4,
    3.174_298_193_0e-4,
    3.794_922_119_6e-4,
    3.881_252_797_2e-4,
    3.524_931_413_5e-4,
    2.865_983_605_9e-4,
    2.060_577_057_8e-4,
    1.252_264_637_2e-4,
    5.511_311_675_9e-5,
    2.308_249_317_5e-6,
    -3.107_420_153_1e-5,
    -4.663_494_517_1e-5,
    -4.842_046_291_8e-5,
    -4.150_172_950_8e-5,
    -3.071_762_898_1e-5,
    -1.980_379_328_2e-5,
    -1.100_091_844_3e-5,
    -5.111_115_537_0e-6,
    -1.873_925_485_7e-6,
    -4.831_694_919_6e-7,
    -6.728_346_863_9e-8,
    -2.167_070_395_4e-9,
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
