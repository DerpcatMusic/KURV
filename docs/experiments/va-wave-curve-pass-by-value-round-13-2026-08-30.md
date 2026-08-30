# VA round 13: `WaveCurveRt` pass-by-value traffic

## Verdict

Rejected the borrow-signature churn. On this x86-64 Rust ABI, the optimized
out-of-line value and borrow probes receive the 256-byte curve through the same
pointer-shaped argument and compile to the same evaluator bodies. Shipping hot
render functions are inlined, so there is no surviving evaluator-boundary copy
to remove. Timing differences were small and unstable across repeated runs.
Production code and version 0.8.7 are unchanged.

## Call-chain audit

The hot custom-wave routes are:

- host block: `WaveCurveTransition::value` -> VA table transition `select` ->
  `OscillatorSettings::custom_curve`
- scalar: voice render -> `generate_custom_step` or
  `sample_custom_shape_with_antialiasing_warped` -> `WaveCurveRt::eval`
- x4/x8: voice render -> `generate_custom4/8` -> `eval4/8`
- block: voice/block render -> `accumulate_custom4/8_block[_constant]` ->
  `eval4/8`
- structural/table modulation: `OscillatorDspSettings::custom_curve` -> block
  accumulators; positioned table selection may interpolate two curves first

`WaveCurveTransition::value` and positioned table selection genuinely produce
a new coefficient set when a transition is active. Borrowing their inputs does
not remove the 256-byte result. The settings structures are passed by value in
many internal APIs, but their optimized aggregate ABI is indirect and local
inlining/scalar replacement keeps evaluator reads pointed at the existing
storage. Changing only the leaf custom-wave signatures therefore adds borrows
at many call sites without addressing a demonstrated copy.

## Generated-code evidence

The probe forced paired value/borrow wrappers out of line, retained symbols,
and disassembled the release test binary. `nm -CS` reported identical sizes:

| evaluator | by value | borrowed |
|---|---:|---:|
| scalar | 208 B | 208 B |
| x4 | 304 B | 304 B |
| x8 | 5 B | 5 B |
| interpolation plus scalar | 486 B | 486 B |

The scalar instruction streams are identical and both read coefficients from
the pointer in `rdi`; the value form does not copy the aggregate in its callee.
Both transition forms reserve the same `0x198`-byte stack frame. The five-byte
x8 bodies are compiler-generated jumps to the same implementation.

## Timing and identity

All 256 scalar comparisons across 64 representative/adversarial compiled
curves were bit-identical. Initial nanoseconds per output sample:

| path | by value | borrowed |
|---|---:|---:|
| scalar | 11.283 | 10.365 |
| x4 | 7.505 | 7.264 |
| x8 | 2.731 | 2.577 |
| interpolate plus scalar | 132.025 | 125.729 |

Three immediate repeats showed noise and crossovers: scalar value/borrow was
`9.555/10.772`, `14.904/10.131`, and `10.984/11.761` ns. x4 stayed within
roughly 0.3-0.9 ns/sample, x8 within 0.2-0.6 ns/sample, and interpolation within
0.7-6.2 ns/call. Because the generated bodies are identical, these are caller
placement/cache/frequency effects rather than evidence for a signature win.

`perf stat` was available, but a combined test necessarily mixes value and
borrow loops; it cannot attribute counters to one signature and was not used to
claim a result.

## Commands

```text
cargo fmt --all
cargo test --release \
  wave_curve::compiler_experiment::wave_curve_pass_by_value_report \
  -- --ignored --nocapture

CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 \
  cargo test --release --no-run \
  wave_curve::compiler_experiment::wave_curve_pass_by_value_report

nm -CS target/release/deps/pure_va_dispersion_core-a6e96a1aa496cd4c | \
  rg 'abi_(value|borrow)_(scalar|x4|x8|transition)'

objdump -Cd --no-show-raw-insn \
  target/release/deps/pure_va_dispersion_core-a6e96a1aa496cd4c
```

The ignored report passed 1/1 with zero identity failures. No production
signature, publication protocol, lifetime, evaluator, or layout was changed.

