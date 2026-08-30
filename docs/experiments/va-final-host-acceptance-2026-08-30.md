# VA final host acceptance — 2026-08-30

## Artifact

- Version: `0.8.14`
- Managed local build: `build-20260830T183914-1308694`
- Installed CLAP SHA-256: `965458d378f4602ac4032c3ba66c8b91a6a5efb1eab2f50b7d680a4be87dd163`

The first fresh `0.8.13` VST3 build did not register: the shell advertised one
eight-bus stereo layout and one one-bus mono layout, while VST3 requires every
layout to preserve bus count and kind. The unused mono alternative was removed
at the shared shell declaration and the patch version advanced to `0.8.14`.
No DSP code changed.

## Automated host-format gates

Commands were run against the installed managed artifact:

```text
clap-validator -v warn validate --only-failed ~/.clap/KURV.clap
vst3-validator -q -e ~/.vst3/KURV.vst3
pluginval --validate ~/.vst3/KURV.vst3 --strictness-level 10 \
  --rtcheck relaxed --repeat 2 --randomise --skip-gui-tests --timeout-ms 60000
```

Results after the shell fix:

- CLAP: 38 passed, 0 failed, 0 warnings, 6 skipped.
- VST3 validator: exit 0, no diagnostics.
- pluginval: success at strictness 10, two runs; cold/warm load, state,
  parameter fuzz/thread safety, automation, bus processing, and audio processing
  passed at 44.1/48/96 kHz and block sizes 64/128/256/512/1024.

## REAPER VST3 render

REAPER 7.79 was launched with a disposable configuration. A Lua bootstrap made
a fresh project, instantiated `VST3: KURV`, and inserted C2, C4, C6 plus rapid
C2/C6/C3/C7 transitions. The bootstrap saved the host-authored opaque state in
`/tmp/kurv-vst3-smoke.rpp`; its close flag did not terminate REAPER, so the
60-second wrapper ended that isolated process after the project was safely
written. A second `-renderproject` invocation exited 0.

`/tmp/kurv-vst3-smoke.wav` is stereo 24-bit PCM, 48 kHz, exactly 4 seconds.
FFmpeg `astats` reported 192,000 samples, peak -17.644634 dBFS, RMS -25.756922
dBFS, DC offset 0.000024, and nonzero entropy. This proves VST3 discovery,
instantiation, MIDI processing, finite non-silent DSP output, and offline render.

## Remaining human boundary

Bitwig is already running a user project with an older mapped KURV build
(`ad77ec34...a2f78`). `bitwig-kurv-preflight.sh --require-match` correctly exits
4 because the loaded host process cannot hot-swap to the new artifact. The
session also uses Dummy Silent Device. It was left untouched: a Bitwig reload
and human listening comparison remain acceptance steps, not automated evidence.
