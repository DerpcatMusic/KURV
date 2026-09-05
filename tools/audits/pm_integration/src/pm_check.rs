use super::*;

pub fn check() {
    let mut maximum = 0.0_f32;
    let mut cases = 0;
    for mode in [Antialiasing::Spline, Antialiasing::SplineOptimized] {
        for step in [0.0, 1.0 / 1024.0, 0.01, 0.125, 0.249, 0.30, 0.44] {
            for depth in [0.0, 0.02, 0.2, 0.5] {
                let mut oscillators = states();
                let mut phases = oscillators.map(|o| o.phase());
                let steps = std::array::from_fn(|lane| step * (1.0 + lane as f32 * 0.001));
                let modulation = std::array::from_fn(|n| {
                    depth * (n as f32 * 0.31 + 2.1 * (n as f32 * 0.17).sin()).sin()
                });
                let left = std::array::from_fn(|lane| 0.03 * (lane + 1) as f32);
                let right = std::array::from_fn(|lane| 0.03 * (8 - lane) as f32);
                let mut output = [(0.0, 0.0); N];
                accumulate_shape8_phase_modulated_block(
                    &mut oscillators,
                    2.0,
                    steps,
                    &modulation,
                    0.37,
                    mode,
                    left,
                    right,
                    &mut output,
                );
                for n in 0..N {
                    let mut expected = (0.0, 0.0);
                    for lane in 0..8 {
                        let mut phase = phases[lane] + modulation[n];
                        if phase < 0.0 {
                            phase += 1.0;
                        }
                        if phase >= 1.0 {
                            phase -= 1.0;
                        }
                        let mut scalar = VaOscillator::default();
                        scalar.set_phase(f64::from(phase));
                        let value = scalar.generate_shape_step(2.0, steps[lane], 0.37, mode);
                        expected.0 += value * left[lane];
                        expected.1 += value * right[lane];
                        phases[lane] += steps[lane];
                        if phases[lane] >= 1.0 {
                            phases[lane] -= 1.0;
                        }
                    }
                    assert_close("PM scalar left", output[n].0, expected.0, &mut maximum);
                    assert_close("PM scalar right", output[n].1, expected.1, &mut maximum);
                }
                assert_eq!(phases, oscillators.map(|o| o.phase()));
                cases += 1;
            }
        }
    }
    println!(
        "PASS {cases} full-module generic PM cases vs scalar oracle; maximum error {maximum:.9}"
    );
}
