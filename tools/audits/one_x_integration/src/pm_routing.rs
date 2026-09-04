use super::*;

pub fn check() {
    let mut cases = 0;
    for shape in [0.0, 1.0, 1.5, 2.0, 2.5, 3.0] {
        for step in [0.01, 0.199, 0.225, 0.251, 0.375, 0.449] {
            for depth in [0.0, 0.02, 0.15, 0.5] {
                let modulation = std::array::from_fn(|n| {
                    let t = n as f32;
                    depth * (t * 0.31 + 3.1 * (t * 0.17).sin()).sin()
                });
                let render = |mode| {
                    let mut oscillators = states();
                    let mut output = [(0.0, 0.0); N];
                    accumulate_shape8_phase_modulated_block(
                        &mut oscillators,
                        shape,
                        std::array::from_fn(|lane| step * (1.0 + lane as f32 * 0.001)),
                        &modulation,
                        0.37,
                        mode,
                        std::array::from_fn(|lane| 0.03 * (lane + 1) as f32),
                        std::array::from_fn(|lane| 0.03 * (8 - lane) as f32),
                        &mut output,
                    );
                    (output, oscillators.map(|oscillator| oscillator.phase()))
                };
                let (baseline, baseline_phase) = render(Antialiasing::SplineOptimized);
                let (candidate, candidate_phase) = render(Antialiasing::Spline.for_factor(1));
                assert_eq!(baseline_phase, candidate_phase, "PM phase state");
                for (a, b) in baseline.into_iter().zip(candidate) {
                    assert!(a.0.is_finite() && a.1.is_finite());
                    assert_eq!(a.0.to_bits(), b.0.to_bits(), "PM left output");
                    assert_eq!(a.1.to_bits(), b.1.to_bits(), "PM right output");
                }
                cases += 1;
            }
        }
    }
    println!(
        "PASS: {cases} explicit PM route cases preserve baseline output and state bit for bit, including zero depth and nested offsets"
    );
}
