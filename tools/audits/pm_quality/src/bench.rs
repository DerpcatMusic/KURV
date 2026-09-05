use crate::{antialias, pm_quality, trajectory};
use std::{hint::black_box, time::Instant};
use wide::f32x8;
pub fn run() {
    println!("carrier,depth,nested,repeat,algorithm,ns_per_lane_sample,checksum");
    for c in [37., 193., 853.] {
        for (d, n) in [(0.02, 0.), (0.5, 0.5)] {
            let inputs: Vec<_> = (0..4096)
                .map(|i| {
                    let t = i as f64 / 4096.;
                    let prev = trajectory(t - 1. / 4096., c, 193., d, n);
                    let curr = trajectory(t, c, 193., d, n);
                    (
                        prev.rem_euclid(1.),
                        prev.rem_euclid(1.) + curr - prev,
                        curr.rem_euclid(1.),
                    )
                })
                .collect();
            let vectors: Vec<_> = inputs
                .chunks_exact(8)
                .map(|q| {
                    (
                        f32x8::from(std::array::from_fn(|i| q[i].0 as f32)),
                        f32x8::from(std::array::from_fn(|i| q[i].1 as f32)),
                        f32x8::from(std::array::from_fn(|i| q[i].2 as f32)),
                    )
                })
                .collect();
            for repeat in 0..7 {
                let order = if repeat % 2 == 0 {
                    [0, 1, 2, 3]
                } else {
                    [3, 2, 1, 0]
                };
                for algorithm in order {
                    let start = Instant::now();
                    let mut sum = 0.;
                    let mut v = f32x8::ZERO;
                    for _ in 0..1000 {
                        match algorithm {
                            0 => {
                                for &(_a, _b, p) in &inputs {
                                    sum += black_box(antialias::bandlimited_saw(
                                        black_box(p),
                                        black_box(c / 4096.),
                                        antialias::Antialiasing::SplineOptimized,
                                    ));
                                }
                            }
                            1 => {
                                for &(a, b, _p) in &inputs {
                                    sum += black_box(pm_quality::saw_linear_average_rebased(
                                        black_box(a),
                                        black_box(b),
                                    ));
                                }
                            }
                            2 => {
                                for &(_a, _b, p) in &vectors {
                                    v += black_box(antialias::bandlimited_saw8(
                                        black_box(p),
                                        black_box(f32x8::splat((c / 4096.) as f32)),
                                        antialias::Antialiasing::SplineOptimized,
                                    ));
                                }
                            }
                            _ => {
                                for &(a, b, _p) in &vectors {
                                    v += black_box(pm_quality::saw_linear_average8(
                                        black_box(a),
                                        black_box(b),
                                    ));
                                }
                            }
                        }
                    }
                    let ns = start.elapsed().as_nanos() as f64 / 4096000.;
                    sum += v.to_array().iter().sum::<f32>() as f64;
                    println!("{c},{d},{n},{repeat},{algorithm},{ns},{sum}");
                }
            }
        }
    }
}
