# Current-source VA correctness harness

Run from any working directory:

```sh
python3 tools/audits/pm_integration/run.py
python3 tools/audits/pm_integration/run.py --seeds 37 1299709 --output /tmp/kurv-holdout.json
python3 tools/audits/pm_integration/run.py --verify-record tools/audits/pm_integration/results.json
```

The default seeds are reproducible examples, not a claim of exhaustive coverage.
The runner compiles current checkout source, records source and binary SHA-256,
compiler version, requested and actual SIMD backend, and output. It rejects source
changes during a run. `--verify-record` rejects historical results when any
compiled source or harness input changes. Timings of this runner are **not DSP
benchmarks**. CI uploads a freshly generated temporary record, never an old
checked-in result following a failure.

The build script copies production VA, curve, warp, ratio, oversampling,
performance and numeric modules. It supplies the exact `wide` SIMD types. The
following dependencies are explicitly omitted:

- Framework serialization imports, derives and persistence adapters.
- Two table serialization tests, disabled by exact names with assertions that
  those source markers still exist.
- `advance_rich_timeline` and its resynthesis control import. This method belongs
  to the Rich engine; VA oscillator storage fields remain unchanged.

No VA waveform, antialiasing, phase accumulator, backend selection, or block
renderer arithmetic is replaced. This harness does **not** compile or test the
whole voice graph, Rich synthesis, UI, host, or serialization. The missing private
and untracked dependencies in PR16 still block the full plugin build.

## What the checks establish

- Existing production DSP tests run through `cargo test`; ignored manual reports
  remain separately visible. Their ignored status is not quality evidence.
- Controlled canonical fixtures compare scalar, x4/x8, time SIMD, and selected
  saw backend at ordinary frequencies and dispatch boundaries.
- Controlled PM fixtures compare the actual public block API with scalar samples,
  with heterogeneous phases/steps and nested offset samples. Their range is stated
  in the source and does not establish arbitrary phase-wrap or audio quality.
- Seeded partition checks render the same PM stream through 1-, 7-, 16-, 31-,
  and 64-frame calls, including ragged tails, mixed signs, nonzero accumulators,
  width/morph variation, and heterogeneous lane steps. Half the packs are wholly
  narrow so random high lanes cannot accidentally exclude the narrow SIMD path.
  Sample output must agree within 3e-5 and phase state exactly. That tolerance is
  a numerical consistency limit, not a listening threshold or alias specification.

Every default seed executes 1,964,032 stereo sample comparisons per backend.
The same corpus is repeated under baseline and AVX2 requests; actual selection is
printed, so an unsupported machine cannot be mistaken for AVX2 coverage. Repeated
comparisons across backends are not independent spectral test cases.

Independent spectral negative controls distinguish pure gain loss, a known
coincident alias, and an off-grid spur. Measurement tests reject invalid audio and
recover analytic multitone magnitudes. Separate MinBLEP and trajectory experiments
supply the quality references; partition equality alone cannot prove clean audio.

Snapshot results are in `results.json`. Re-run rather than quoting those figures
for a modified checkout. A deliberately stale record was rejected during review;
the fresh record was then regenerated.
