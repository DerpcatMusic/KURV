use super::*;

pub fn check() {
    if !Antialiasing::Spline.for_factor(1).is_one_x() {
        return;
    }
    let sample = |shape, phase, step| {
        let mut oscillator = VaOscillator::default();
        oscillator.set_phase(phase);
        oscillator.generate_shape_step(shape, step, 0.37, Antialiasing::Spline.for_factor(1))
    };
    let mut maximum_jump = 0.0_f32;
    for shape in [1.0, 2.0] {
        for boundary in [0.20_f32, 0.225, 0.25, 0.45] {
            for index in 0..257 {
                let phase = index as f64 / 257.0;
                let before = sample(shape, phase, boundary - 0.000_001);
                let after = sample(shape, phase, boundary + 0.000_001);
                maximum_jump = maximum_jump.max((after - before).abs());
                assert!(
                    (after - before).abs() < 0.000_1,
                    "pitch boundary discontinuity"
                );
            }
        }
        for index in 0..1024 {
            // Binary-exact phase makes the analytic and production phase equal.
            let phase = index as f64 / 1024.0;
            let angle = std::f64::consts::TAU * phase;
            let reference = if shape == 2.0 {
                -std::f64::consts::FRAC_2_PI * angle.sin()
            } else {
                -8.0 / std::f64::consts::PI.powi(2) * angle.cos()
            };
            let actual = sample(shape, phase, 0.30);
            assert!(
                (f64::from(actual) - reference).abs() < 0.000_001,
                "high-note Fourier oracle"
            );
        }
    }
    println!(
        "PASS: actual high-note API Fourier oracle and crossover/taper boundaries; maximum neighboring-step difference {maximum_jump:.9}"
    );
}
