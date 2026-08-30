# Stateless pitch-derived residual crossover (2026-08-30)

## Verdict

Reject the stateless crossover.  A hard switch preserves low-note current CPU
and obtains the existing branchless mid/high quality and CPU improvements, but
is discontinuous between residual families.  A 0.004 or 0.012 normalized-step
blend evaluates both kernels in-band and costs 40-68% more at the crossover.
No tested band is CPU-at-most-current everywhere, so production remains
unchanged.

Square was deliberately excluded and remains current as required.

## Candidate

The selector uses only the already-available constant phase step, centered at
`0.025`.  Below the band it calls current `SplineOptimized`; above it calls the
existing branchless `Spline`; inside it linearly interpolates their residuals
before ordinary saw/pulse accumulation.  Triangle interpolates the two existing
BLAMP-corrected samples.  Phase traversal and gain accumulation are unchanged,
and there is no retained state, event-density extraction, allocation, lock, or
I/O.

Tested bands were hard (`0`), narrow (`0.004`), and wide (`0.012`).  Each was
run across low `.0046`, center `.025`, mid `.041`, and high `.083` steps for
coherent and decorrelated x8 lanes, 64-frame constant structural blocks.

## CPU

Outside a blend band, results reproduce the previous static comparison.  At
mid/high pitch branchless typically saves 3-17% for saw, pulse, and triangle;
low remains current apart from measurement noise.  The relevant rejection is
the transition band:

| band | phase layout | saw delta | pulse37 delta | triangle delta |
|---:|---|---:|---:|---:|
| .004 | decorrelated | +41.64% | +48.39% | +58.59% |
| .004 | coherent | +41.92% | +45.66% | +68.40% |
| .012 | decorrelated | +41.56% | +47.74% | +57.35% |
| .012 | coherent | +40.19% | +43.94% | +59.11% |

Band width barely affects center CPU because both kernels must run for every
sample there.  A smaller nonzero band therefore shortens the expensive region
but cannot satisfy the requested `<= current everywhere` gate.  Selector-only
low results ranged within roughly -2.6% to +1.6%, i.e. timing noise rather than
a dependable improvement.

## Quality and transition artifacts

Below the band quality is exactly current.  Above it, the prior exact Fourier
comparison showed universal branchless improvement: at high pitch RMS changed
from `.135852` to `.126058` for saw, `.185280` to `.171517` for pulse37, and
`.041121` to `.037116` for triangle.  Wanted/alias error is therefore not the
rejection; transition economics are.

The residual families are not output-identical.  The preceding fixed-step x8
matrix measured peak family deltas of about `.004142` after its 0.137 gain for
BLEP shapes and `.001339` for triangle.  A 4096-sample rapid reversible step
sweep (`.015` to `.035`, increment `.00005`) measured the first difference of
candidate-minus-current only while the mix changed:

| band | saw | pulse37 | triangle |
|---:|---:|---:|---:|
| .004 | .035162011 | .051528488 | .002144841 |
| .012 | .054758394 | .054758394 | .002818861 |

These ungained values include genuine residual-event motion while the blend is
changing—the exact artifact a stateless moving-pitch crossover must tolerate.
The sampled hard switch happened away from an active event and measured zero;
that does not erase its phase-dependent worst case established by the fixed
family delta.  A stateful time crossfade could decorrelate the transition from
pitch, but is outside this round and would add the framework explicitly ruled
out.

Exact command:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test pitch_crossover_residual_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

The final command passed 1/1 with 379 tests filtered out and the checkout's
existing 25 test-build warnings.  The ignored probe is retained as evidence;
no runtime selector or state was added.
