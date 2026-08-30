# Pruned recurrence finite-residual handoff, round 23 (rejected)

Date: 2026-08-30
Baseline: `2fbd45d99c9e93422e52ae01b96565e060be1aa6`

## Verdict

Reject and revert. Replacing the previous dual full-block crossfade with two
departing-family endpoint samples plus one arriving-family block preserved exact
phase publication and recovered the recurrence's CPU advantage. It did not
repair the 5.9-to-6.1 kHz backend seam. The minimum value-only cubic increased
RMS or transition error for every shape and direction. Matching both value and
first difference produced severe overshoot, up to 2.8608 peak error. No runtime
renderer, selector, correction state, test harness, or version bump is retained.

## Distinct candidate

This was a finite boundary-residual handoff, not another continuous transition
or dual-render fade. At entry, the prototype evaluated only the current renderer's
first two samples at the authoritative new phase, rendered the full block only
with the cap-three real recurrence, and added a correction which was exactly zero
at the block end. Exit reversed the roles: it evaluated the recurrence's first
two samples, rendered only the current family, and corrected that arriving block.

The value-only candidate used the zero-value and zero-slope endpoint polynomial

```text
q[n] = d0 (2 t^3 - 3 t^2 + 1),  t = n / (N - 1)
```

where `d0` is the departing-minus-arriving first-sample residual. The exact
discrete value-and-first-difference candidate used `M = N - 2`:

```text
q[n] = d0 (n-M)(n-M-1)(1-n) / (M(M+1))
     + d1 (n-M)(n-M-1)n     / (M(M-1))
```

This enforces `q[0] = d0`, `q[1] = d1`, and
`q[N-2] = q[N-1] = 0`. Both variants therefore had zero latency, zero
post-block tail, and no persistent correction state. A production selector would
still require at least one byte per oscillator. The existing oscillator is 40
bytes; fixed local x8 recurrence seed and endpoint scratch were 320 and 64 bytes.
There was no per-sample trigonometry. Block setup used the existing phase/step x8
sine-cosine evaluation and one scalar pulse-width sine-cosine evaluation.

Rounds 5, 17, and 20 blended two complete waveform families or continuously
changed harmonic weights. This probe rendered only one complete arriving block
and sampled the departing family at two endpoints, so it tested a materially
smaller and different handoff cost.

## Quality and artifact gate

The focused probe swept 256 starting phases in both directions at 48 kHz, over
eight blocks with the switch after block four. It covered actual 24- and 32-frame
blocks and saw, square, 31% pulse, and triangle. The wanted waveform was the f64
analytic Fourier projection with the exact legal cap at every sample. All x8
oscillator phase bits matched a scalar f32 reference after every block.

The value-only correction lost every representative gate below. Ranges span the
24- and 32-frame blocks; lower is better.

| shape | entry RMS hard | entry RMS corrected | entry excess-delta hard | entry excess-delta corrected | exit peak hard | exit peak corrected |
|---|---:|---:|---:|---:|---:|---:|
| saw | 0.119027 | 0.123082-0.123096 | 0.392508-0.392539 | 0.592654-0.592718 | 0.392576 | 0.596966-0.599110 |
| square | 0.154968 | 0.162706-0.162733 | 0.346685-0.346689 | 0.612037-0.612104 | 0.346689 | 0.663856-0.675858 |
| pulse31 | 0.157655 | 0.162924-0.162942 | 0.406521-0.406551 | 0.489724-0.489768 | 0.406579 | 0.827006-0.831164 |
| triangle | 0.052418 | 0.055098-0.055107 | 0.144535-0.144540 | 0.149576-0.149589 | 0.144544 | 0.277051-0.282060 |

The exit excess-delta metric also worsened for every shape: corrected ranges
were 0.7491-0.7516 saw, 0.6132-0.6163 square, 0.6685-0.6729 pulse, and
0.1489-0.1502 triangle, versus hard values of about 0.7366, 0.5967, 0.6524,
and 0.1445. Exact value-and-first-difference matching was substantially worse
than value-only matching in every shape. Its worst peak was 2.8608 for square,
and its worst saw peak was 2.1653.

The point residual is therefore not the seam's cause. Forcing the arriving
waveform through a departing-family sample spreads a full waveform-family
difference across the block; adding a first-difference constraint raises the
polynomial enough to overshoot. The correction cannot be both zero-ended and
closer to the authoritative new cap-limited curve under these constraints.

The pulse implementation retained round 22's verified coefficient sign:
`Re(c_k) = +sin(2 pi k width) / (pi k)`. No compensating sign or phase logic
was introduced.

## Actual short-note CPU

Two release executions used CPU 4 affinity, AVX2/FMA, `-C target-cpu=native`,
30,000 actual blocks per cell, and five paired medians per execution. Each
interval contained one corrected entry, recurrence steady blocks, and one
corrected exit. Eight lanes were slightly detuned and phase publication was bit
exact. Ratios below are the value-only candidate divided by all-current x8;
lower is better, and ranges span both executions and both 24/32-frame blocks.

| shape | 4-block duty | 16-block duty | 64-block duty |
|---|---:|---:|---:|
| saw | 0.894-1.004 | 0.749-0.791 | 0.707-0.739 |
| square | 0.686-0.714 | 0.512-0.548 | 0.472-0.492 |
| pulse31 | 0.572-0.608 | 0.381-0.407 | 0.329-0.356 |
| triangle | 0.480-0.501 | 0.346-0.371 | 0.314-0.332 |

The pair-cubic CPU was similar but slightly worse: saw four-block ratios were
0.979-1.000, while the best 64-block triangle cells were 0.311-0.330. Those
wins cannot rescue a renderer which fails the waveform/artifact gate. Even if
the correction were perceptually acceptable, four-block saw is not a universal
CPU win.

CPU evidence is coarse rather than pristine. No compiler overlapped either
accepted execution. Live aggregate idle was about 81-94%, with a low steady
Bitwig/VM load. The large quality loss is deterministic; the timing is included
only to show that removing dual full-block rendering did recover the expected
frontier. No near/parity/win cell is accepted as production evidence.

## Reproduction

```text
CARGO_BUILD_JOBS=1 \
CARGO_TARGET_DIR=/tmp/kurv-va-pruned-idft-target \
RUSTFLAGS='-C target-cpu=native' \
taskset -c 4 cargo test --release --no-default-features --lib \
  pruned_recurrence_handoff_residual_report --locked --no-run

KURV_HANDOFF_BLOCKS=30000 taskset -c 4 \
  /tmp/kurv-va-pruned-idft-target/release/deps/\
pure_va_dispersion_core-c78bc2803a9e2383 \
  oscillators::va::experiment::\
pruned_recurrence_handoff_residual_report \
  --exact --ignored --nocapture --test-threads=1
```

The release compile completed in 2m42s with only pre-existing warnings. The
smoke run and both full report executions passed 1/1 with 390 filtered tests;
each full report took about 2.62 seconds. The ignored 607-line probe was removed
after recording these results.

## Decision

Finite residual correction solves the dual-render CPU problem but not the
waveform-family seam. Assembly could reduce the recurrence's steady arithmetic;
it cannot make a wrong boundary curve authoritative. A future recurrence attempt
needs a selector whose two renderers meet on the same sampled waveform, or a
transition defined in harmonic space, rather than a polynomial fitted to one or
two time-domain samples.
