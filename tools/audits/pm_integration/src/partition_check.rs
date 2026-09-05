//! Metamorphic check: changing callback partitions must preserve the trajectory.
//! No copied oscillator formula; both sides call the actual public block API.
use super::*;

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / 16_777_216.0
}
fn render<const B: usize>(
    state: &mut [VaOscillator; 8],
    shape: f32,
    steps: [f32; 8],
    pm: &[f32],
    width: f32,
    mode: Antialiasing,
    gains: [[f32; 8]; 2],
    output: &mut [(f32, f32)],
) {
    let mut n = 0;
    while n + B <= output.len() {
        let modulation: [f32; B] = pm[n..n + B].try_into().unwrap();
        let mut block: [(f32, f32); B] = output[n..n + B].try_into().unwrap();
        accumulate_shape8_phase_modulated_block(
            state,
            shape,
            steps,
            &modulation,
            width,
            mode,
            gains[0],
            gains[1],
            &mut block,
        );
        output[n..n + B].copy_from_slice(&block);
        n += B;
    }
    if n < output.len() {
        render::<1>(
            state,
            shape,
            steps,
            &pm[n..],
            width,
            mode,
            gains,
            &mut output[n..],
        );
    }
}
pub fn check() {
    let seed = std::env::var("KURV_TEST_SEED")
        .map(|s| s.parse::<u64>().expect("invalid seed"))
        .unwrap_or(0x4b5552562026);
    assert_ne!(seed, 0, "xorshift seed must be nonzero");
    let mut rng = seed;
    let mut maximum = 0.0;
    let mut comparisons = 0;
    for case in 0..128 {
        let initial = std::array::from_fn(|_| {
            let mut s = VaOscillator::default();
            s.set_phase(random(&mut rng) as f64);
            s
        });
        // Half the packs are wholly narrow; unconstrained random packs almost
        // always include a high lane and would miss narrow-only SIMD dispatch.
        let step_limit = if case % 2 == 0 { 0.249 } else { 0.449 };
        let steps = std::array::from_fn(|_| 0.0001 + random(&mut rng) * step_limit);
        let pm: [f32; 137] = std::array::from_fn(|_| (random(&mut rng) - 0.5) * 0.98);
        let width = 0.001 + random(&mut rng) * 0.998;
        let gains =
            std::array::from_fn(|_| std::array::from_fn(|_| (random(&mut rng) - 0.5) * 0.25));
        let initial_output: [(f32, f32); 137] =
            std::array::from_fn(|_| (random(&mut rng) - 0.5, random(&mut rng) - 0.5));
        for mode in [Antialiasing::Spline, Antialiasing::SplineOptimized] {
            for shape in [0.0, 0.43, 1.0, 1.71, 2.0, 2.63, 3.0] {
                let mut expected = initial_output;
                let mut reference_state = initial;
                render::<1>(
                    &mut reference_state,
                    shape,
                    steps,
                    &pm,
                    width,
                    mode,
                    gains,
                    &mut expected,
                );
                for block in [7, 16, 31, 64] {
                    let mut actual = initial_output;
                    let mut state = initial;
                    match block {
                        7 => render::<7>(
                            &mut state,
                            shape,
                            steps,
                            &pm,
                            width,
                            mode,
                            gains,
                            &mut actual,
                        ),
                        16 => render::<16>(
                            &mut state,
                            shape,
                            steps,
                            &pm,
                            width,
                            mode,
                            gains,
                            &mut actual,
                        ),
                        31 => render::<31>(
                            &mut state,
                            shape,
                            steps,
                            &pm,
                            width,
                            mode,
                            gains,
                            &mut actual,
                        ),
                        _ => render::<64>(
                            &mut state,
                            shape,
                            steps,
                            &pm,
                            width,
                            mode,
                            gains,
                            &mut actual,
                        ),
                    }
                    assert_eq!(
                        state.map(|o| o.phase()),
                        reference_state.map(|o| o.phase()),
                        "seed={seed}, block={block}"
                    );
                    for (a, b) in actual.into_iter().zip(expected) {
                        assert_close("partition left", a.0, b.0, &mut maximum);
                        assert_close("partition right", a.1, b.1, &mut maximum);
                        comparisons += 2;
                    }
                }
            }
        }
    }
    println!(
        "PASS partition invariance: seed={seed}, {comparisons} stereo sample comparisons, maximum={maximum:.9}"
    );
}
