# Expression interpolation: confirmed bounds defects

Audited against `d084681`; this change fixes evaluation-stack capacity and exact
endpoints. It does **not** fix nested expression instruction-buffer exhaustion.
Do not treat this as a complete expression-transition safety fix.

## Reproducible findings

The parser accepts a right-nested sum of 16 `x` operands: 31 instructions and
peak stack depth 16. Interpolation evaluates the left program first and keeps
its weighted result live while evaluating the right program. That needs 17
slots, but all three evaluators previously allocated only 16. A legal expression
could therefore panic during scalar, four-lane, or eight-lane evaluation.

Even within the 69-instruction buffer, six nested blends of a constant with that
31-instruction function have 67 instructions and need 22 live stack entries.
Increasing the evaluator stack to just 17 would leave that defect.

The evaluator now reserves `(MAX_INSTRUCTIONS + 1) / 2` = 35 entries, the upper
bound for valid postfix programs producing one result using unary and binary
operators. Every push after the first requires a corresponding binary reduction.
The parser retains its existing 16-value user limit. Storage layout, expression
syntax, and instruction-buffer capacity are unchanged. This does not validate
arbitrary malformed data supplied through `from_words`.

Exact mix endpoints now return the existing program without assembling a new
one. Besides avoiding needless concatenation, this prevents a discarded
expression's non-finite intermediate values from poisoning the selected endpoint.

## Unresolved: composing blended programs still panics

Interpolation requires `previous.len + current.len + 5` instructions. The fixed
buffer has room for 69. Two 37-instruction blends require 79, and `write()` panics
at word index 140 of a 140-element array. This is not unsafe memory corruption,
but panicking during audio processing is unacceptable.

There are production routes to nesting:

- `WaveCurveTransition::retarget` snapshots an in-progress interpolated curve.
- `VaTableTransition::select` blends two table selections, each of which can
  already interpolate adjacent function frames in `VaTableRt::select`.

An exact, arbitrary sequence of distinct expression blends cannot fit in any
fixed instruction buffer. Merely increasing `MAX_INSTRUCTIONS`, truncating
instructions, or dropping one waveform does not solve that contract.

Recommended follow-up: represent source programs separately with bounded blend
weights, and bound/queue transition retargets explicitly. VATABLE transitions
need up to four source frames before retarget policy is considered. Choose and
test output-clamp semantics too: clamping each source before blending is not
identical to the current clamp-after-composition behavior. If fallback to sampled
curves is chosen, specify approximation error and build those curves off the
audio thread. A checked composition API should report capacity failure before
writing; its callers need an explicit behavior, not a silent dropped waveform.

## Running the evidence

The small harness includes the production source file directly and uses the
same `wide` 0.7.33 vector types reexported by `truce_simd`. It excludes host/UI
integration and therefore does not establish complete plugin correctness.

```
cargo test --manifest-path tools/expression-capacity-proof/Cargo.toml
cargo test --manifest-path tools/expression-capacity-proof/Cargo.toml -- --ignored
```

With Rust 1.97.1, the first command passes three regression tests: single and
nested in-capacity interpolation (scalar/four/eight lanes), and endpoint identity.
Running these same three tests against the original `d084681` function source
fails all three: two evaluator stack panics at index 16, and an endpoint
construction panic at word index 140. This verifies that the regressions detect
the pre-change defects.

The second command intentionally runs the known unresolved regression and fails
with `index out of bounds: the len is 140 but the index is 140`.

## Cost and limits

The stack remains fixed and allocation-free; no new per-instruction checks or
heap growth were added. Endpoint interpolation avoids O(program length) copying
and arithmetic instructions when only one source should play. The maximum
scalar/four/eight-lane evaluator stack backing grows from 64/256/512 bytes to
140/560/1120 bytes. Its effect on generated code and wall time has **not** been
benchmarked; do not market this safety change as an evaluator speedup.
