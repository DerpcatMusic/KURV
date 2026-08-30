# Branchless versus optimized cubic residual (2026-08-30)

## Verdict

Do not add a static or density-adaptive selector.  The existing branchless
`Spline` residual improves the ideal-reference error and often wins at mid/high
event density, but it is slower at low density and has a large coherent-square
high-density regression.  Switching families is not output-identical: the
instantaneous difference reaches about `0.00414`, so a pitch/density threshold
would introduce its own transition unless it gained retained crossfade state.

There is no simple unconditional Pareto win.  Production DSP and version remain
unchanged.

## Compared code

No new residual was introduced.  `SplineOptimized` is the current fitted
implementation: it extracts event, inner, and outer masks, tests `any()`, and
only evaluates polynomial regions that are populated.  `Spline` is the already
present branchless cubic: it evaluates inner and outer expressions for the
packed lanes and selects with blends.  Both use the current block-hoisted
support/inverse-step path, phase traversal, gains, and constant-step structural
x8 accumulators.

The release binary is stripped, so named-symbol disassembly is unavailable.
Source/codegen structure is nevertheless unambiguous: optimized lowers three
mask tests to movemask/test/conditional control flow around FMA chains;
branchless retains both arithmetic chains plus compares/blends.  This explains
why empty/low-density blocks favor optimized while dense blocks can amortize
branchless arithmetic better than divergent control flow.

## CPU matrix

Nanoseconds per host frame, 64-frame x8 blocks, 12,000 blocks, best of five:

| density / phase | shape | optimized | branchless | delta |
|---|---|---:|---:|---:|
| low / decorrelated | saw | 2.581 | 2.645 | +2.47% |
| low / decorrelated | square | 3.987 | 4.161 | +4.35% |
| low / coherent | square | 4.023 | 4.048 | +0.62% |
| mid / decorrelated | saw | 4.294 | 3.984 | -7.22% |
| mid / decorrelated | square | 7.987 | 7.397 | -7.38% |
| mid / coherent | pulse37 | 8.348 | 7.405 | -11.30% |
| mid / decorrelated | triangle | 6.736 | 5.887 | -12.60% |
| high / decorrelated | saw | 4.384 | 3.901 | -11.02% |
| high / decorrelated | pulse37 | 8.977 | 7.483 | -16.64% |
| high / coherent | square | 8.032 | 9.862 | +22.79% |
| high / coherent | triangle | 6.110 | 5.881 | -3.76% |

The full probe covers all 24 low/mid/high, coherent/decorrelated, and
saw/square/pulse/triangle combinations.  Low-density branchless never produced
a meaningful CPU win.  At high density square changes sign with phase
coherence, so a shape-only choice is unsafe; detecting coherence requires the
same event-mask/popcount overhead rejected by the preceding experiment.

## Quality versus ideal projection

RMS values use 65,536 phases and exact Fourier projection truncated below
Nyquist.  Lower is better:

| range | shape | optimized RMS | branchless RMS | optimized peak | branchless peak |
|---|---|---:|---:|---:|---:|
| low | saw | .030189673 | .027921763 | .338633739 | .309174207 |
| low | square | .042544129 | .039341211 | .336374657 | .306977862 |
| low | pulse37 | .042565167 | .039361563 | .337954858 | .308601499 |
| low | triangle | .000535278 | .000483156 | .005375559 | .004834138 |
| mid | saw | .091958886 | .085136184 | .352097741 | .322340377 |
| mid | square | .126107607 | .116532624 | .333224830 | .304022034 |
| mid | pulse37 | .132979584 | .123289901 | .378694190 | .348989650 |
| mid | triangle | .014226660 | .012838820 | .047662753 | .042837043 |
| high | saw | .135852439 | .126058140 | .378636950 | .348531864 |
| high | square | .181378183 | .167525040 | .344183843 | .314933754 |
| high | pulse37 | .185280128 | .171517313 | .388801281 | .359770895 |
| high | triangle | .041121405 | .037115827 | .097757969 | .087988850 |

Branchless is consistently closer to this ideal, but it is not equivalent to
shipping output.  Across structural blocks, branchless-minus-optimized RMS
ranges from about `8.3e-6` (low triangle) to `.00206` (high pulse), with peak
difference `.004142` for BLEP shapes and `.001339` for triangle BLAMP.  A
runtime threshold can therefore click or zipper on moving pitch.  Making the
transition safe would require state/crossfade and defeat the requested static
choice.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test branchless_residual_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

The final command passed 1/1 with 378 tests filtered out and the checkout's
existing 25 test-build warnings.  The ignored comparison remains probe-only.
