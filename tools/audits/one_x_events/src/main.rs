#![allow(dead_code)]
#[path = "../../../../src/oscillators/va/one_x_events.rs"]
mod events;
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/antialias.rs"));
}
use events::*;
use std::{hint::black_box, time::Instant};
const N: usize = 8192;
fn trajectory(case: usize, n: f64) -> f64 {
    let t = n / N as f64;
    let tau = std::f64::consts::TAU;
    match case {
        0 => 0.123 + 83.0 * t,
        1 => 0.123 + 1703.0 * t,
        2 => 0.123 + 83.0 * t + 0.22 * (tau * 997.0 * t).sin(),
        3 => {
            0.123
                + 83.0 * t
                + 0.22
                    * (tau * 997.0 * t
                        + 1.4 * (tau * 211.0 * t + 1.1 * (tau * 53.0 * t).sin()).sin())
                    .sin()
        }
        _ => 0.123 - 83.0 * t,
    }
}
fn main() {
    let kernel = EventKernel::new();
    if std::env::args().any(|a| a == "--bench") {
        for case in 0..5 {
            let input: Vec<_> = (0..N).map(|n| trajectory(case, n as f64)).collect();
            let mut all = [[0.0; 7]; 2];
            for round in 0..7 {
                for ordering in 0..2 {
                    let method = (ordering + round) % 2;
                    let start = Instant::now();
                    let mut s = EventSaw::new();
                    for _ in 0..128 {
                        for &p in &input {
                            black_box(if method == 0 {
                                s.process(black_box(p), black_box(&kernel))
                            } else {
                                antialias::bandlimited_saw(
                                    black_box(p - p.floor()),
                                    if case == 1 {
                                        1703.0 / N as f64
                                    } else {
                                        83.0 / N as f64
                                    },
                                    antialias::Antialiasing::SplineOptimized,
                                )
                            });
                        }
                    }
                    all[method][round] = start.elapsed().as_nanos() as f64 / (128 * N) as f64;
                }
            }
            for series in &mut all {
                series.sort_by(f64::total_cmp);
            }
            println!(
                "case={case} event_ns={} shipping_scalar_ns={} ratio={} event_range={:?} shipping_range={:?}",
                all[0][3],
                all[1][3],
                all[0][3] / all[1][3],
                [all[0][0], all[0][6]],
                [all[1][0], all[1][6]]
            );
        }
        return;
    }
    let case: usize = std::env::args()
        .nth(1)
        .unwrap_or("0".into())
        .parse()
        .unwrap();
    let mut saw = EventSaw::new();
    for n in -(2 * N as i64)..N as i64 + LATENCY as i64 {
        let p = trajectory(case, n as f64);
        let event = saw.process(p, &kernel);
        if n >= LATENCY as i64 {
            let i = n - LATENCY as i64;
            let p = trajectory(case, i as f64);
            let shipping = antialias::bandlimited_saw(
                p - p.floor(),
                if case == 1 {
                    1703.0 / N as f64
                } else {
                    83.0 / N as f64
                },
                antialias::Antialiasing::SplineOptimized,
            );
            println!("{event:.12},{shipping:.12}");
        }
    }
}
