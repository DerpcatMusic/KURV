#![allow(dead_code)]
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/antialias.rs"));
}
mod pm_quality {
    include!(concat!(env!("OUT_DIR"), "/pm_quality.rs"));
}
mod bench;
const N: usize = 4096;
fn trajectory(t: f64, c: f64, m: f64, d: f64, n: f64) -> f64 {
    c * t
        + 0.123
        + if n < 0.0 {
            d * (1.0 - n * (std::f64::consts::TAU * 367.0 * t).sin())
                * (std::f64::consts::TAU * m * t).sin()
        } else {
            d * (std::f64::consts::TAU * (m * t + n * (std::f64::consts::TAU * 367.0 * t).sin()))
                .sin()
        }
}
fn main() {
    if std::env::args().any(|a| a == "--bench") {
        bench::run();
        return;
    }
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    writeln!(
        out,
        "carrier,modulator,depth,nested,index,baseline,local,central,adaa"
    )
    .unwrap();
    for c in [37., 193., 853.] {
        for m in [23., 193., 701., 1501.] {
            for (d, n) in [
                (0., 0.),
                (0.02, 0.),
                (0.15, 0.),
                (0.5, 0.),
                (0.15, 0.5),
                (0.5, 0.5),
                (1.0, 1.25),
                (0.15, -0.8),
            ] {
                for i in 0..N {
                    let t = i as f64 / N as f64;
                    let p = trajectory(t, c, m, d, n);
                    let prev = trajectory(t - 1. / N as f64, c, m, d, n);
                    let next = trajectory(t + 1. / N as f64, c, m, d, n);
                    let baseline = antialias::bandlimited_saw(
                        p.rem_euclid(1.),
                        c / N as f64,
                        antialias::Antialiasing::SplineOptimized,
                    );
                    let local = pm_quality::saw_local_width(p.rem_euclid(1.), p - prev);
                    let central =
                        pm_quality::saw_local_width(p.rem_euclid(1.), (next - prev) * 0.5);
                    let adaa = pm_quality::saw_linear_average(prev, p);
                    writeln!(
                        out,
                        "{c},{m},{d},{n},{i},{baseline:.17},{local:.17},{central:.17},{adaa:.17}"
                    )
                    .unwrap();
                }
            }
        }
    }
}
