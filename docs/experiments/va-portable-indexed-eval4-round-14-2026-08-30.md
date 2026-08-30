# VA round 14: portable indexed `eval4` selection

## Verdict

Shipped in 0.8.8. The portable four-lane evaluator now calculates each lane's
segment once and loads its four cubic coefficients directly, replacing fifteen
vector comparisons and sixty blends. The polynomial, clamp, 256-byte curve
layout, atomic publication, transitions, and AVX2/FMA `eval8`/`eval4` path are
unchanged.

The boundary index is `ceil(phase * 16) - 1`, saturated to segment 0..15. This
deliberately preserves the old strict-`>` scan contract: a phase exactly on an
interior boundary evaluates the left segment at `t = 1`, which matters for an
intentional hard join.

## Identity

The release probe covered 64 deterministic representative/adversarial curves,
all 17 segment boundaries at the exact value and one `f32::EPSILON` to either
side, plus 256 random four-lane phase vectors per curve. The portable indexed
selector matched the old portable scan bit for bit:

- samples checked: 69,888
- bit-identity failures: 0

The x86-64-v3 comparison reports differences because its shipping baseline is
the pre-existing AVX2/FMA eight-lane implementation padded to four lanes; it is
not the portable scan and is intentionally not replaced.

## CPU

Release-mode nanoseconds per output sample on the generic build:

| phases | old scan | indexed |
|---|---:|---:|
| coherent | 7.440 | 3.673 |
| decorrelated | 7.717 | 3.770 |
| transition interpolation plus decorrelated eval | 34.283 | 33.550 |

On `x86-64-v2`, the first run measured `5.009 -> 2.257` coherent and
`5.010 -> 2.284` decorrelated. Three immediate repeats retained a roughly
43-50% direct-evaluator reduction. The transition case was dominated by the
64-coefficient interpolation and crossed over between runs, so no transition
speedup is claimed.

On `x86-64-v3`, the retained AVX2/FMA path measured 1.750-1.758 ns/sample versus
2.073-2.220 for the portable indexed probe. The production cfg continues to
select AVX2/FMA there.

## Generated code

The debuginfo `x86-64-v2` test binary showed:

- old scan: 232 bytes, 52 disassembled instructions
- indexed selector: 584 bytes, 132 branchless instructions

The indexed form is larger because Rust's defined float-to-integer conversion
emits saturation handling independently for four lanes, but it avoids the long
coefficient broadcast/blend dependency chain and is materially faster. Its
source replaces rather than adds to the old selector and uses no unsafe code.

The `x86-64-v3` executable contained a 595-byte indexed probe, but production
`eval4` continued through the existing AVX2/FMA evaluator. A supplementary
v3 debuginfo cdylib link ended with an LLVM/lld SIGBUS after producing the test
executable; the normal v3 release test had already run successfully. The
multiple debuginfo variants then filled the 11 GB worktree `target` directory;
only generated Cargo artifacts were cleared with `cargo clean` before final
generic validation.

AArch64/NEON disassembly was not attempted: this host only has an Apple
AArch64 Rust target, with no usable Apple SDK/linker. No toolchain was installed.

## Commands

```text
cargo test --release \
  wave_curve::compiler_experiment::indexed_eval4_selector_report \
  -- --ignored --nocapture

RUSTFLAGS='-C target-cpu=x86-64-v2' cargo test --release \
  wave_curve::compiler_experiment::indexed_eval4_selector_report \
  -- --ignored --nocapture

RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test --release \
  wave_curve::compiler_experiment::indexed_eval4_selector_report \
  -- --ignored --nocapture

CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 \
RUSTFLAGS='-C target-cpu=x86-64-v2' cargo test --release --no-run \
  wave_curve::compiler_experiment::indexed_eval4_selector_report

nm -CS target/release/deps/pure_va_dispersion_core-66ad85e5c1b3c2b6 | \
  rg '(indexed_eval4|scan_eval4)'
objdump -Cd --no-show-raw-insn \
  target/release/deps/pure_va_dispersion_core-66ad85e5c1b3c2b6

cargo clean
cargo test --release wave_curve --locked
```

Final generic suite: 8 passed, 0 failed, 16 ignored.

