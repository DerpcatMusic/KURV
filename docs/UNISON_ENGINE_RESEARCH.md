# Research: a genuinely dense, efficient VA unison engine

Research date: 2026-08-03. Scope: 1–64 real oscillators per played note, 32-note
polyphony (2,048 simultaneously rendered oscillators), with no FFT, wavetable,
hidden voice reduction, or oscillator substitution.

## Bottom line

There is no documented evidence that DUNE 3 renders independent oscillators at
zero marginal cost. In fact, its manual says that disabling unused oscillator
stacks saves CPU, that doubling its full unison voices doubles CPU, and that
audio-rate operation costs substantially more than block processing. What it
does document is a deliberately cheaper **oscillator stack inside one synth
voice**, block-rate control processing, and optional processing across as many
as six CPU cores. Synapse does not publish the oscillator kernel, its SIMD
layout, or its antialiasing method. Claims about those internals would be
speculation. [DUNE 3 product page][dune-product] [DUNE 3 manual, especially
sections 3.1, 3.2, 4.2, 4.10, and 10][dune-manual]

A true stack still has to update and combine each independent phase, so its
work is fundamentally O(notes × oscillators × samples). The practical win is
to make one machine instruction update several oscillators, share all
note-level work, move invariant math out of the sample loop, and process a
whole block through a waveform-specialized kernel. The best-supported target
for KURV is therefore:

1. one note voice and one envelope around a 64-lane oscillator bank;
2. structure-of-arrays state, processed in SIMD-width chunks across unison;
3. one block-level dispatch for Saw, Pulse, Triangle, or Sine;
4. cached phase increments, detune ratios, pan gains, and normalization;
5. an antialiasing kernel selected by measured cost and alias rejection, not by
   the prestige of its name;
6. optional persistent worker threads only after the single-thread SIMD kernel
   is exhausted.

At 64 oscillators, AVX2 can cover the stack as eight groups of eight `f32`/`u32`
lanes. SSE2 and NEON cover it as sixteen groups of four `f32`/`u32` lanes. This
does not make 64 oscillators cost the
same as two, but it can make the scaling appear nearly flat on a coarse host
meter when the rest of the voice and host block dominate. That last sentence
is an inference, not a statement about DUNE.

## What the primary sources actually establish

### DUNE 3

- Synapse advertises two stacks of 32 oscillators, a third oscillator, 8×
  unison, up to 520 oscillators per note, and a reserve of 8320 oscillators.
  [Product page][dune-product]
- The current manual describes density, symmetric detune and stereo spread,
  multiple stack-tuning distributions, random initial phase when reset is off,
  and warns that common phase reset creates strong phasing with several
  oscillators. [Manual §4.2][dune-manual]
- The manual explicitly says a single synth voice containing an eight-oscillator
  stack is much cheaper than eight complete unison voices containing one
  oscillator each. This establishes **shared voice-level processing**, not a
  constant-time oscillator algorithm. [Manual §10][dune-manual]
- Normal modulation is evaluated less frequently; Audio Rate processes the
  entire engine sample by sample and is documented as substantially more
  expensive. This is direct evidence for block/control-rate work sharing.
  [Manual §§3.2 and 10][dune-manual]
- DUNE can use up to six cores, and its manual recommends larger buffers for
  the best multithreaded result. [Manual §§3.1 and 10][dune-manual]
- Synapse's current publications are internally inconsistent about the maximum
  note polyphony: the product page's 8320/520 reserve corresponds to 16 fully
  loaded notes, while the 3.6 manual says up to 24-note polyphony. These figures
  are product capabilities, not a reproducible CPU benchmark.

The screenshots supplied for KURV and DUNE therefore do not prove that DUNE's
32-density oscillator is free. Host meters have different scales, averaging,
thread accounting, and display resolution. A controlled render on the same
machine and buffer size is required.

### Production open-source synths

Surge XT's source is unusually explicit about its optimization. Its sine
oscillator comment says that high-unison work was made faster by SIMD processing
**over unison**, vector sine approximations, and block-level template dispatch
that removes waveform-mode switches from the inner loop. Its implementation
updates four unison lanes at a time, stores their panned samples, then performs
the phase update and final accumulation. [Surge XT sine oscillator, pinned
source][surge-sine]

Surge's shared unison helper precomputes linear detune positions, an alternating
stereo distribution, and `1/sqrt(N)` attenuation. Its oscillator initialization
also randomizes phases unless retrigger is selected. [Surge unison helper,
pinned source][surge-unison] [Surge classic oscillator, pinned source][surge-classic]

Vital is a wavetable synth, so it is not an oscillator algorithm for KURV. Its
source is still relevant as an independent production layout example: it packs
oscillator phases into SIMD integer vectors, keeps phase multipliers in parallel
arrays, specializes distortion mode outside the hot loop, and processes only
the number of packed phase groups required by the active voices and unison
count. [Vital oscillator declaration][vital-header] [Vital oscillator
processing][vital-source] [Vital SIMD integer type][vital-simd]

These sources support SIMD lane packing, integer phase, block specialization,
and active-prefix processing. They do **not** prove how DUNE is implemented.

## Current KURV audit against Truce 6.3

This section is a source audit, not a listening-test claim. Truce 6.3's official
synth is intentionally basic, but it establishes the framework's note/envelope
contract. DUNE, Surge XT, and the measured JP-8000 study then show which musical
stack behaviors KURV currently omits.

### Sustain is a level; the gate owns note lifetime

The official Truce 6.3 synth calls `Voice::release()` on `NoteOff`, which moves
its envelope to `Release`; while the key remains down, the `Sustain` stage holds
the configured level. A sustain value of 1.0 therefore means “hold full level
until note-off,” not “play forever.” An AHDSR **Hold** stage would only add a
timed plateau after Attack and would not repair a missing note-off.
[Truce synth guide][truce-synth-guide] [Truce 6.3 synth source][truce-synth-lib]
[Truce 6.3 envelope source][truce-synth-voice]

KURV's envelope implements that same stage transition, so a voice that stays in
`Sustain` after key release means the release event or an all-notes-off path did
not reach `VaVoice::release`; it is not caused by `sustain = 1.0`.
[KURV voice source](../src/voice.rs) [KURV event dispatch](../src/lib.rs)

The immediate gaps are:

- The audit found that KURV handled MIDI 1.0 `NoteOn`/`NoteOff` and CC64 but
  ignored channel-mode panic/reset messages. It now handles CC120 (All Sound
  Off), CC121 (Reset All Controllers), and the all-notes-off behavior attached
  to CC123–127. [MIDI Association CC table][midi-cc]
- The audit found that `PolySynth::note_off` released only the oldest matching
  voice; it now follows Truce's conservative policy of releasing every matching
  non-releasing voice. MIDI 1.0 cannot identify two overlapping instances of the
  same channel/note, so the policy must remain explicit and paired with an
  unconditional channel panic path.
- The manifest does not opt into MIDI 2.0. Truce documents that this makes the
  wrappers down-convert host MIDI 2.0 channel voice messages before delivery;
  therefore adding `NoteOff2` handling alone would not explain or fix the
  current Bitwig failure. If `midi2 = true` is added later, KURV must handle
  `NoteOn2`/`NoteOff2`, their 16-bit velocity, and the relevant per-note events.
  [Truce MIDI guide][truce-midi] [Truce 6.3 `EventBody`][truce-eventbody]
- Truce normalizes MIDI 1.0 Note On with velocity zero into `NoteOff`, so KURV
  does not need a second velocity-zero workaround. [Truce 6.3 `EventBody`][truce-eventbody]

Actionable gate contract: ordinary note-off enters Release regardless of the
ADSR sustain level; CC64 may defer that release; pedal-up releases every deferred
voice; CC123–127 release the channel; CC120 kills the channel immediately; reset
or deactivation kills every voice. Log event kind/channel/note at a bounded
non-audio-thread seam when reproducing the Bitwig case so delivery loss is
separated from envelope loss.

### Why 32 currently sounds smaller than it is

KURV does render all requested oscillators. The audit found three policies that
suppressed perceptual growth or made the result unstable:

1. The audit found that `PolySynth::refresh_active_gain` applied
   `0.8 / sqrt(active_notes)` to the entire synth. Every new held note therefore
   turned down all notes already sounding (four notes: -6 dB per note; sixteen:
   -12 dB per note). The official Truce synth simply sums active voices and
   applies one master level; it does not renormalize the patch on every note-on.
   KURV now uses fixed master headroom, so adding or releasing a note no longer
   changes the gain of every other note.
2. Each stack is energy-normalized near `1 / sqrt(unison_count)`. That is a
   defensible headroom law—Surge uses `1 / sqrt(N)`—but by design 32 decorrelated
   oscillators have roughly the same RMS level as one. KURV now adds a gradual
   amplitude-density bias reaching 1.2× (+1.58 dB) at 32. That is a modest,
   defensible voicing choice if peak and stereo-width headroom remain verified;
   it makes count audible without pretending 32 coherent signals can be summed
   safely at unity gain. DUNE instead exposes an Amount relationship for
   center-versus-satellite levels, and the JP-8000 study found separate center
   and side gain curves. [Surge unison helper][surge-unison]
   [DUNE manual §4.2.1][dune-manual] [Szabo supersaw study][supersaw]
3. The audit found that even stack sizes created **two oscillators at exactly
   the center pitch** with different random locked phases, yielding only 31
   unique frequencies at count 32 and an unstable tonal core. The layout now
   uses one center only for odd counts and symmetric pairs for even counts.

Minimal corrective direction: remove active-note-count gain modulation, retain
the corrected odd/even topology, and listen to the 1.2× density bias under fixed
output gain before adding another amount control. A separate center/satellite
mix is the next musical control only if the fixed voicing cannot cover both
focused basses and diffuse pads. A limiter or gentle saturation belongs after
that decision, not as a substitute for it.

### Phase randomization, movement, detune, and stereo

KURV's hash produces independent full-range phases for every oscillator at
`Phase Random = 100%`, and `age` changes the seed on every note trigger. It is
real deterministic pseudorandomization and is appropriate for repeatable offline
renders. The control only runs in `VaVoice::start`, however, so moving it while a
note is already held cannot change that note. This is correct for note-start
phase, but the UI should make the behavior explicit.

Random initial phase changes the transient and the starting point of beat cycles;
it does not create ongoing motion. Szabo's measurements specifically found that
random phase should remain static during a held trigger. Surge obtains ongoing
analog-like movement separately: every unison lane has a low-pass random drift
LFO which is added to its static unison detune. DUNE likewise distinguishes
random initial phase from its Swarm mode, where oscillators are modulated
individually. [Szabo supersaw study][supersaw] [Surge drift source][surge-unison]
[Surge classic oscillator][surge-classic] [DUNE manual §4.2.1][dune-manual]

Therefore the quality route is:

- keep phase randomization as a note-on operation over `[0, 1)`;
- add very slow, band-limited **frequency drift** per lane at control rate, with
  zero-mean pair correction so the perceived note does not wander. This is
  justified now—not merely a future optimization—because the reported defect is
  static, non-lush movement and both Surge and DUNE separate it from start phase.
  Keep it subtle and fixed initially; add a Drift/Swarm knob only if listening
  shows one amount cannot serve the instrument;
- offer a center-dense nonlinear detune curve before adding more voices—DUNE's
  manual explicitly distinguishes Linear, Nonlinear, Gaussian, Random, and
  Swarm stack modes, and the JP-8000's measured detune control is nonlinear;
- do not call static deterministic detune “random” and do not continuously
  randomize oscillator phase.

KURV's pan positions are deterministic and now follow a coherent alternating
pair policy. Its linear gains, `L = 1 - pan`, `R = 1 + pan`, match Surge's
first-party helper and preserve each lane's mono sum exactly. They are not an
equal-power law: stereo energy rises with width by the factor `1 + pan²`, up to
+3 dB for a fully panned lane. That behavior can sound larger and is defensible,
but the width control must be checked for loudness jumps, L/R balance, mono sum,
and correlation rather than called neutral. DUNE documents a different but also
coherent symmetric spread (two voices hard L/R; three L/C/R at maximum). Keep
KURV's alternating mirrored-pair policy unless a seeded pan permutation wins a
controlled listening comparison; stereo width must never rely on polarity
inversion. [Surge unison helper][surge-unison] [DUNE manual §4.10][dune-manual]

### Oscillator quality implications for the current files

- `src/oscillator.rs` uses a compact two-sample PolyBLEP for saw/pulse. It is a
  good fast baseline, not an alias-free endpoint. Compare the already identified
  third-order PolyBLEP, DPW4, and BLIT-FDF candidates under the same alias/error
  target before replacing it.
- The analytic triangle has uncorrected slope discontinuities; PolyBLAMP is the
  direct literature-backed correction. [DAFx PolyBLAMP paper][blamp-paper]
- A rounded 5 kHz saw edge in a 48 kHz single-cycle display is expected from a
  bandlimited oscillator: a truly vertical discrete-time edge would require
  harmonics above Nyquist. Judge wanted-harmonic accuracy and aliased energy,
  not pixel sharpness.
- `src/voice.rs` now adds pitch-only, deterministic control-rate JITTER to its
  static layout and no longer pumps gain by active-note count. It still has no independent
  center/satellite amount control or post-stack color/saturation. Those are
  voicing choices, not prerequisites for a correct 32-oscillator stack.
- Truce's example waveform generator is naive saw/square and exists to
  demonstrate framework wiring, not production VA quality. Reuse its event,
  gate, preallocation, and tail patterns—not its oscillator kernel.

Thread boundary: phase seeding and layout coefficients are note/control-rate;
per-lane JITTER state is fixed-size and stepped at bounded control rate; oscillator
generation stays allocation-free in the audio loop. Truce explicitly requires
sample-offset event handling and no allocation, locks, or I/O in `process`.
[Truce processing guide][truce-processing] [Truce MIDI guide][truce-midi]

## Antialiasing choices

No finite discrete-time saw or pulse is a mathematically vertical, infinitely
bandwidth waveform. “Perfect” should mean that all wanted harmonics below
Nyquist are accurate and aliases are below an explicit audibility/error target.
The peer-reviewed VA literature evaluates that tradeoff using hearing thresholds
and masking, not by demanding a vertical line between samples.

| Method | Cost and state | Published quality/limitations | Fit for 1,024 oscillators |
|---|---|---|---|
| Current two-sample PolyBLEP | A few arithmetic operations and edge tests per oscillator/sample; pulse corrects two edges | This is the low-order member of a larger PolyBLEP family. It reduces aliases but must not be described as mathematically bandlimited. | Very good first SIMD target: edge branches become masks and no table traffic is needed. |
| Higher-order PolyBLEP | More polynomial work and several corrected samples around an edge | Välimäki, Pekonen, and Nam report that the integrated third-order B-spline offered the best cost/quality tradeoff among their tested methods and perceptually alias-free saw emulation up to 7.8 kHz at 44.1 kHz. [JASA paper][polyblep-paper] | Strong quality-mode candidate after the low-order SIMD baseline. More arithmetic, still compact and vectorizable. |
| minBLEP / tabulated BLEP | Insert a precomputed correction into a short per-oscillator residual buffer at each discontinuity; lookup and overlap costs depend on correction length and frequency | Longer correction kernels can approach an ideal bandlimited step, but table size, interpolation, and overlapping events trade memory/cache and CPU for rejection. The BLIT literature documents those tradeoffs. [Antialiasing review][aa-review] [BLIT-FDF paper][blit-fdf] | Not automatically the fastest at 1,024 lanes. Benchmark only if its measured alias advantage is required. |
| DPW2–DPW4 | Polynomial evaluation, one or more finite differences, scaling, and a small amount of history; branch-light and table-free | The DPW paper reports fourth-order DPW as perceptually alias-free over the grand-piano register, but also discusses large low-frequency scaling and combining orders to avoid it. [DPW paper][dpw-paper] Faust's authoritative implementation notes low-frequency noise as a practical ceiling for orders through four. [Faust oscillator docs][faust-dpw] | Excellent saw/triangle SIMD candidate because it is regular arithmetic. It needs a controlled low-note fallback/crossover. |
| Low-order fractional-delay BLIT | Generate a short fractional-delay impulse at each period, remove DC, then integrate; the cited implementation computes only four nonzero pulse samples per period for its third-order case | The paper reports a third-order B-spline as perceptually alias-free over practical fundamentals, no lookup table, and discusses PWM, hard sync, supersaw, transient DC, and period-rate parameter updates. [BLIT-FDF paper][blit-fdf] | Especially interesting for low notes, where events are sparse. More state and control complexity than PolyBLEP/DPW; modulation behavior must be verified. |
| PolyBLAMP for Triangle | Corrects discontinuities in the first derivative rather than waveform steps | The paper reports up to 50 dB alias-component reduction and about 20 dB SNR improvement, and found it more efficient than oversampling for the studied corner cases. [DAFx BLAMP paper][blamp-paper] | Direct fix for KURV's currently uncorrected analytic triangle corners. |
| Oversampling a naive shape | Multiplies oscillator work and adds resampling filters | The cited papers consistently treat oversampling as a valid but costly baseline, not the automatic best solution for classical shapes. | KURV combines local PolyBLEP edge correction with selectable 1x-4x synthesis and fixed-allocation decimation at 2x-4x. This is deliberately not naive-only oversampling and remains bounded at the 2,048-oscillator maximum. |

The least risky route is not a wholesale switch to minBLEP. First vectorize the
existing PolyBLEP sound, then compare higher-order PolyBLEP, DPW4, and BLIT-FDF
with the same perceptual/spectral acceptance limits. A hybrid selected by
waveform and frequency is legitimate if every lane remains a real oscillator;
it is not hidden voice reduction.

## Phase, layout, and the hot loop

### State layout

Use parallel, fixed-size, aligned arrays for phase, phase increment, left gain,
right gain, and any antialias history. Operate on a padded active prefix. A
`[Oscillator { phase: f64 }; 32]` is harmless while phase is the only field, but
it becomes an auto-vectorization obstacle as soon as per-oscillator fields are
added. The hot kernel should see contiguous homogeneous vectors.

Do not compute `exp2`, `sqrt`, pan law, detune position, finite-value recovery,
sample-rate reciprocals, or pulse-width bounds per oscillator/sample. KURV
already caches detune ratios and pan gains; keep that. Also cache the base phase
increment once per voice/control update and multiply it by the cached detune
ratio outside the sample loop whenever modulation permits.

Dispatch the waveform once per process block (or parameter segment), so Saw,
Pulse, Triangle, and Sine each have a monomorphic inner kernel. LLVM documents
that its loop and SLP vectorizers are enabled by default, but complicated
control flow, external math calls, and floating-point reductions can block or
limit vectorization. Optimization remarks and generated assembly must confirm
the result; source that merely looks vectorizable is not evidence.
[LLVM vectorizer documentation][llvm-vectorizer]

For a no-regression first SIMD pass, retain the current scalar accumulation
order after generating lanes in parallel. Surge uses this pattern. A horizontal
SIMD reduction can reorder floating-point additions and change low bits; it can
be adopted later only if the measured output delta is accepted.

### Phase representation

KURV uses normalized `f64` phase. This is robust, but it halves SIMD lane count
relative to `f32`/`u32`. Its audio path already uses the appropriate cheap wrap
for the constrained increment: add once and conditionally subtract one. The
general `floor` wrap remains useful for arbitrary editor/display phases and is
not part of the oscillator hot path.

A wrapping 32-bit or 64-bit fixed-point phase accumulator is a serious
candidate, not a blind requirement. DDS literature describes the same modulo
accumulator: the tuning word is added once per sample and natural overflow
forms the cycle. At 48 kHz, a 32-bit accumulator's tuning step is about
0.00001118 Hz; at 192 kHz it is about 0.00004470 Hz. Phase-to-amplitude
truncation can create spurs, so only the amplitude conversion may discard low
bits, and the result must be measured. [Analog Devices DDS tutorial][dds]

Vital's production source demonstrates packed 32-bit integer phases, while
Surge retains double phase in at least one SIMD-unison oscillator. Both are
credible designs. For KURV, fixed phase should win only if it matches the
current tuning, alias, and long-duration tests.

### Sine

KURV now uses a table-free quarter-wave degree-11 SIMD polynomial rather than
`wide::sin()`, whose path also computes cosine and performs quadrant reduction.
Against an `f64::sin` reference across the phase domain, the replacement measured
maximum error `2.02e-7` and RMS error `4.01e-8`, improving on the replaced path's
`4.16e-7` maximum and `9.65e-8` RMS. The 16-note × 32-oscillator sine benchmark
improved from 332.24 to 540–548 million oscillator-samples/s.

## Unison sound behavior that also helps engineering

- Initialize phases at note-on, not every sample. DUNE's manual recommends
  free/random phase for larger stacks to avoid strong phase-reset phasing;
  Surge does the same unless retrigger is selected. This also avoids a
  worst-case coherent 32-oscillator peak at every note onset.
- Generate deterministic pseudorandom start phases from note/voice generation
  if reproducible rendering is required. The PRNG work is note-rate, outside
  the sample loop.
- Precompute detune and pan topology when count, spread, or sample rate changes.
  Linear spacing is only one sound. DUNE documents nonlinear, Gaussian,
  alternate, random, chord, sub, and swarm distributions; they change cached
  coefficients, not hot-loop complexity.
- `1/sqrt(N)` normalization is used by Surge and preserves approximate energy
  for decorrelated oscillators. It does not bound coherent peaks. Phase policy,
  headroom, and an explicit gain law must be designed together.
- Adam Szabo's measured JP-8000 study found seven oscillators, randomized phase,
  nonlinear detune behavior, and a separate center/side mix relationship. It is
  useful evidence for a musical supersaw distribution, but it is not evidence
  about DUNE and not a CPU optimization. [Szabo thesis][supersaw]

## Denormals and real-time constraints

The audio callback must allocate nothing, lock nothing, perform no I/O, and
make no system calls. All oscillator arrays, residual buffers, and any worker
coordination must be created before processing.

Phase counters and direct PolyBLEP arithmetic do not naturally decay into
subnormals. Envelopes, leaky integrators, DPW history, DC blockers, and filters
can. Snap a state to exact zero when it enters an inaudibly tiny range and stop
processing an idle envelope. Do not casually set x86 MXCSR FTZ/DAZ from Rust:
Rust's own `_mm_setcsr` documentation says changing denormal mode causes
undefined behavior under Rust's floating-point assumptions, even if restored.
[Rust `_mm_setcsr` documentation][rust-mxcsr]

## Compiler and CPU dispatch

- Keep shipping code portable. `-C target-cpu=native` is appropriate for a
  machine-local experiment, not a redistributable plug-in.
- Build a scalar/SSE2 baseline and select AVX2 (and later AVX-512 only if it
  measures better) with one runtime dispatch outside the audio loop. Rust
  provides `is_x86_feature_detected!`; target-feature functions remain an
  unsafe boundary that needs a small, audited wrapper. [Rust runtime feature
  detection][rust-cpuid]
- AArch64 should use NEON with the same lane-oriented data model.
- Release `opt-level=3` is necessary but not sufficient. Benchmark
  `codegen-units=1` and thin versus fat LTO; Cargo and rustc document their
  tradeoffs. [Cargo profiles][cargo-profiles] [rustc codegen options][rustc-codegen]
- PGO can improve inlining, branch layout, and register allocation when trained
  on representative notes, waveforms, frequencies, and block sizes. Rust
  supports instrumented PGO, but it comes after kernel/data-layout work and
  only stays if the measured gain justifies the release complexity.
  [rustc PGO guide][rust-pgo]

## Multicore: useful, but not the first move

DUNE documents up to six internal cores and says the benefit is strongest for
high-polyphony pads at buffers of at least 128 samples, recommending 512 for
best results. This can explain part of a low wall-clock meter, but it is not a
free oscillator algorithm.

For KURV, first make one core efficient. If 32×32 still misses the callback
deadline after SIMD, a persistent, preallocated worker pool can partition
active notes and reduce into per-worker stereo buffers. It must have bounded
RT-safe coordination and no mutex, allocation, thread creation, or sleeping in
the callback. Internal threading can lose at small buffers and can oversubscribe
a host already parallelizing tracks, so enable it only above a measured work
threshold and keep a single-thread path.

## Measurement contract

Pluginval proves format/lifecycle correctness; its own documentation describes
strictness in terms of host compatibility, state, and parameter fuzzing—not CPU
throughput or oscillator quality. [Pluginval headless documentation][pluginval]
The optimization loop needs a deterministic offline render of the actual DSP
kernel and a same-host plug-in stress run.

Measure all of these without reducing work:

- 16 held notes × 1, 2, 4, 8, 16, 32, 48, and 64 oscillators;
- Saw, Pulse at several widths, Triangle, and Sine;
- 20, 110, 440, 5000, and 10000 Hz where valid;
- 44.1, 48, 96, and 192 kHz;
- blocks of 32, 64, 128, and 512 samples;
- cycles per output sample, cycles per oscillator-sample, callback maximum and
  high percentile—not only average wall time;
- zero allocations and no RT locks;
- frequency error over long renders, DC, peak/RMS gain, NaN/Inf, and idle-state
  behavior;
- wanted-harmonic error and aliased energy against a high-quality reference,
  plus the noise-to-mask approach used by the cited oscillator papers.

Each optimization must compare audio against the previous accepted reference.
For structural changes such as phase layout or SIMD generation, retain the same
oscillator count and inspect both numerical delta and spectrum. “The host meter
didn't move” is useful user evidence, but it is not a reproducible acceptance
criterion.

## Highest-confidence optimization order

1. Hoist remaining per-sample invariants and dispatch waveform once per block;
   verify generated assembly. The cheap conditional phase wrap is already in
   place.
2. Convert oscillator state to aligned SoA and SIMD the unison loop, initially
   preserving current PolyBLEP formulas and scalar accumulation order.
3. Specialize 1- and 2-oscillator paths, then packed 4/8-lane paths and a masked
   tail. Count 64 is eight full AVX2 groups, with no tail.
4. Keep the measured bounded-error vector sine kernel and recheck it after any
   compiler or target-CPU change.
5. Compare SIMD PolyBLEP with DPW4 and low-order BLIT-FDF under the same alias
   and tuning contract; use PolyBLAMP for Triangle.
6. Evaluate fixed-point phase, LTO/codegen-unit variants, then representative
   PGO.
7. Add internal multicore rendering only if the optimized single-core callback
   still misses its deadline at the target buffer size.

This sequence is an engineering inference from the published algorithms,
compiler documentation, and production source. It is not a claim that DUNE
uses the same sequence.

## KURV measured result

The current engine was spot-checked on an AMD Ryzen 7 7800X3D with one process
pinned to logical CPU 14, rustc 1.97.1, `target-cpu=x86-64-v3`, optimization
level 3, and one codegen unit, at the pathological maximum of 32 held notes ×
64 unison oscillators with the default HIGH 3x quality. Every oscillator is
evaluated three times per host sample, then the stereo sum passes through the
193-tap decimator. These are
median thread-CPU measurements from seven runs, not a DAW's differently
averaged meter.

| Waveform | 128-sample callback | One-core deadline |
|---|---:|---:|
| Saw | 0.293 ms | 10.98% |
| Sine | 0.373 ms | 13.98% |
| Saw to Pulse morph | 0.563 ms | 21.11% |

The accepted kernel keeps packed `f32` phase and phase increments, evaluates the
canonical saw and pulse edge corrections in four/eight-lane SIMD, evaluates eight
sine amplitudes together, and accumulates stereo gains in SIMD. It performs all
2,048 independent oscillator lanes; there is no density-dependent lane culling,
shared phase, wavetable, or hidden voice reduction.

Several plausible optimizations were rejected after controlled comparison. A
balanced PGO profile regressed high-note saw and pulse, and cached reciprocal
phase steps caused comparable high-note regressions. Those paths are not in
KURV.

[dune-product]: https://www.synapse-audio.com/dune3.html
[dune-manual]: https://www.synapse-audio.com/DUNE-36-Manual.pdf
[surge-sine]: https://github.com/surge-synthesizer/surge/blob/ccf40bc2fcad2bd43bb47824593ce6f2489446db/src/common/dsp/oscillators/SineOscillator.cpp
[surge-classic]: https://github.com/surge-synthesizer/surge/blob/ccf40bc2fcad2bd43bb47824593ce6f2489446db/src/common/dsp/oscillators/ClassicOscillator.cpp
[surge-unison]: https://github.com/surge-synthesizer/sst-basic-blocks/blob/22fc5e1605201e0b9ddc331c36e82a7b3ffea92b/include/sst/basic-blocks/dsp/OscillatorDriftUnisonCharacter.h
[vital-header]: https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.h
[vital-source]: https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/producers/synth_oscillator.cpp
[vital-simd]: https://github.com/mtytel/vital/blob/636ca0ef517a4db087a6a08a6a8a5e704e21f836/src/synthesis/framework/poly_values.h
[aa-review]: https://research.aalto.fi/en/publications/antialiasing-oscillators-in-subtractive-synthesis/
[polyblep-paper]: https://mac.kaist.ac.kr/pubs/ValimakiPeknenNam-jasa2012.pdf
[dpw-paper]: https://mac.kaist.ac.kr/pubs/ValimakiNamSmithAbel-taslp2010.pdf
[blit-fdf]: https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf
[blamp-paper]: https://dafx.de/paper-archive/2016/dafxpapers/18-DAFx-16_paper_33-PN.pdf
[faust-dpw]: https://faustlibraries.grame.fr/libs/oscillators/
[llvm-vectorizer]: https://llvm.org/docs/Vectorizers.html
[dds]: https://www.analog.com/media/en/training-seminars/tutorials/mt-085.pdf
[sleef]: https://arxiv.org/abs/2001.09258
[supersaw]: https://www.adamszabo.com/internet/adam_szabo_how_to_emulate_the_super_saw.pdf
[rust-mxcsr]: https://doc.rust-lang.org/core/arch/x86/fn._mm_setcsr.html
[rust-cpuid]: https://doc.rust-lang.org/std/macro.is_x86_feature_detected.html
[cargo-profiles]: https://doc.rust-lang.org/cargo/reference/profiles.html
[rustc-codegen]: https://doc.rust-lang.org/rustc/codegen-options/index.html
[rust-pgo]: https://doc.rust-lang.org/rustc/profile-guided-optimization.html
[pluginval]: https://github.com/Tracktion/pluginval#running-in-headless-mode
[truce-synth-guide]: https://truce.audio/docs/examples/synth/
[truce-synth-lib]: https://github.com/truce-audio/truce/blob/v6.3.0/examples/truce-example-synth/src/lib.rs
[truce-synth-voice]: https://github.com/truce-audio/truce/blob/v6.3.0/examples/truce-example-synth/src/voice.rs
[truce-midi]: https://truce.audio/docs/guide/midi/
[truce-processing]: https://truce.audio/docs/guide/processing/
[truce-eventbody]: https://rustdoc.truce.audio/truce_core/events/enum.EventBody.html
[midi-cc]: https://midi.org/midi-1-0-control-change-messages
