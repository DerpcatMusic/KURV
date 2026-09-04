#![allow(dead_code)]
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/antialias.rs"));
}
mod one_x_high {
    include!(concat!(env!("OUT_DIR"), "/one_x_high.rs"));
}
fn main() {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    writeln!(
        out,
        "shape,carrier_bin,depth,nested_depth,index,baseline,candidate"
    )
    .unwrap();
    for shape in ["saw", "triangle"] {
        for carrier in [853, 1105] {
            for (depth, nested) in [
                (0.0, 0.0),
                (0.02, 0.0),
                (0.15, 0.0),
                (0.5, 0.0),
                (0.15, 0.5),
                (0.5, 0.5),
            ] {
                for i in 0..4096 {
                    let t = i as f64 / 4096.0;
                    let p = (carrier as f64 * t
                        + 0.123
                        + depth
                            * (std::f64::consts::TAU
                                * (193.0 * t
                                    + nested * (std::f64::consts::TAU * 367.0 * t).sin()))
                            .sin())
                    .rem_euclid(1.0);
                    let step = carrier as f64 / 4096.0;
                    let (a, b) = if shape == "saw" {
                        (
                            antialias::bandlimited_saw(
                                p,
                                step,
                                antialias::Antialiasing::SplineOptimized,
                            ),
                            one_x_high::saw(p, step),
                        )
                    } else {
                        (
                            antialias::bandlimited_triangle(
                                p,
                                step,
                                antialias::Antialiasing::SplineOptimized,
                            ),
                            one_x_high::triangle(p, step),
                        )
                    };
                    writeln!(
                        out,
                        "{shape},{carrier},{depth},{nested},{i},{a:.17},{b:.17}"
                    )
                    .unwrap();
                }
            }
        }
    }
}
