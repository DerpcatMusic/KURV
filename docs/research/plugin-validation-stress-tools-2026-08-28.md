# KURV plugin validation and stress-tool research (2026-08-28)

## Result

Keep `clap-validator`, Steinberg's VST3 validator, and `pluginval` as the format-conformance gates. The smallest non-duplicative extension is:

1. **DawDreamer** for deterministic, headless VST3 instrument rendering under a second host implementation.
2. **Rust RealtimeSanitizer** for allocations and blocking calls inside KURV's callback and RT worker entries.
3. **AddressSanitizer**, plus a narrowly scoped **ThreadSanitizer** pass, for KURV's unsafe pool and publication code.
4. **`cargo-fuzz`** for malformed legacy state and bounded control/event sequences that must never panic or produce non-finite audio.

No other general plugin checker found adds enough coverage to justify another permanent dependency or CI lane.

## High-value additions

| Tool | Distinct coverage | Automation and formats | Current status | Exact KURV integration cost | Recommendation |
|---|---|---|---|---|---|
| [DawDreamer](https://github.com/DBraun/DawDreamer) | Loads the shipped binary in a real JUCE host, renders MIDI instruments offline, drives normalized parameter automation at audio rate, and saves/loads plugin state. Its documented `PluginProcessor` supports VST2/3 and AU, MIDI, automation, and state management. | Headless Python API on Linux, Windows, and macOS. VST3 applies to all three; it has no CLAP support. [Official plugin docs](https://dbraun.github.io/DawDreamer/user_guide/plugin_processor.html), [platform/features](https://github.com/DBraun/DawDreamer#features) | PyPI `0.9.0`, published 2026-08-12; Python 3.11-3.14 and all three desktop OS families are declared. [PyPI](https://pypi.org/project/dawdreamer/) | One isolated Python dependency and one roughly 80-line runner. Build KURV first, then instantiate a new `RenderEngine(sample_rate, block_size)` per matrix cell, load `KURV.vst3`, schedule MIDI/automation, render, inspect samples, and exercise `save_state`/`load_state`. No Rust dependency or product-code change. | **Add.** This is the only researched host that materially extends the existing validators rather than rescanning metadata. |
| [Rust RealtimeSanitizer](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html#realtimesanitizer) | Reports `malloc`, `free`, locks, and other known non-deterministic calls reached from a function marked `#[sanitize(realtime = "nonblocking")]`. This directly tests KURV's strongest RT invariant. | Headless executable instrumentation, not a plugin-format validator. Current Rust target specs include it on Linux x86-64 and aarch64; LLVM supports RTSan on Linux and Darwin. [Rust x86-64 target source](https://github.com/rust-lang/rust/blob/master/compiler/rustc_target/src/spec/targets/x86_64_unknown_linux_gnu.rs), [LLVM support source](https://github.com/llvm/llvm-project/blob/main/compiler-rt/cmake/config-ix.cmake), [Clang docs](https://clang.llvm.org/docs/RealtimeSanitizer.html) | Present in current nightly documentation but still unstable: it needs nightly, `#![feature(sanitize)]`, `-Zsanitizer=realtime`, and `-Zbuild-std`. | One opt-in Cargo feature/cfg, the crate feature gate, and attributes on the `PluginLogic::process` implementation **and** the internal RT-worker processing entry. Run the existing `process_lab` scenarios under `cargo +nightly run -Zbuild-std --target x86_64-unknown-linux-gnu` with RTSan in `RUSTFLAGS`. Marking only the host callback would miss work executed on the pool's independent worker threads. | **Add as a Linux stress gate.** It is more precise than a custom allocation-counting allocator and also catches blocking calls. |
| [Rust AddressSanitizer](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html#addresssanitizer) | Heap/stack/global bounds errors, use-after-free/scope/return, double-free, and Linux leaks. This matters around KURV's unsafe RT pool, raw resynth publication pointers, SIMD state, and FFI boundaries. | Headless Rust tests/examples; Linux and macOS x86-64/aarch64 are documented. It does not validate CLAP/VST3 protocol behavior. | Upstream Rust sanitizer support is active but remains a nightly `-Z` facility. | No source dependency. Run selected existing library checks and `process_lab` with `RUSTFLAGS=-Zsanitizer=address`, nightly, `-Zbuild-std`, and the explicit Linux target. Sanitizer slowdown means timing/deadline assertions must not be treated as performance results. | **Add as a scheduled or pre-release lane.** Do not add a separate leak checker on Linux; ASan already enables leak detection there. |
| [Rust ThreadSanitizer](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html#threadsanitizer) | Runtime data-race detection across the internal worker pool and lock-free publication paths. | Headless Rust tests/examples on Linux/macOS x86-64/aarch64. | Active upstream, nightly-only. Rust explicitly warns that all synchronization code needs instrumentation and that `std::sync::atomic::fence` is unsupported. | No source dependency. Instrument the existing internal-pool checks with nightly/build-std and run serially. KURV uses atomic fences in resynth telemetry, so findings there require manual confirmation; start with `voices::internal_rt_pool`, which is the highest-value fence-free scope. | **Probe, then keep only if clean and stable.** It is useful for the pool but cannot be an unquestioned whole-repo pass. |
| [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) | Coverage-guided libFuzzer input generation with crash minimization and coverage. It can explore malformed serialized state far beyond hand-picked fixtures and run numeric/property oracles such as finite output and bounded completion. | Headless Rust harnesses. The current Fuzz Book documents x86-64 Linux, x86-64/Apple-Silicon macOS, and Windows via MSVC ASan; nightly and a C++ compiler are required. [Setup](https://rust-fuzz.github.io/book/cargo-fuzz/setup.html) | Release `0.13.2`, published 2026-06-09. [Release](https://github.com/rust-fuzz/cargo-fuzz/releases/tag/0.13.2) | One independent `fuzz/` workspace and initially **one** target around `Kurv::migrate_state`/the legacy JSON boundary, seeded with known valid legacy states. A bounded run is `cargo +nightly fuzz run migrate_state -- -max_total_time=300`. Add a process/control-sequence target only after the state target or a real failure demonstrates value; full synth construction per fuzz case otherwise destroys throughput. | **Add one narrow state target.** Do not fuzz CLAP/VST3 ABI calls already exercised by their dedicated validators. |

## DawDreamer stress matrix

The runner should vary only dimensions that expose host/plugin contract bugs:

- sample rate: 44.1, 48, 96, and 192 kHz;
- block size: 1, 7, 16, 64, 257, and 1024 frames;
- polyphony: 1, 16, and 64 simultaneous notes;
- audio-rate ramps across cutoff, Q, slope/poles, and morph, including endpoint crossings;
- repeated note-on/off, reset, state save/reload, and render-after-reload.

Each render should fail on a load error, exception, timeout, NaN/Inf, missing output after note-on, or a state-reload mismatch. Record peak and checksum for diagnosis, but do not use this offline host as a CPU benchmark: Python/JUCE host overhead and non-real-time rendering make its wall clock incomparable to `process_lab` or a DAW callback.

## Sanitizer commands

These are the upstream Rust invocation shapes, adapted to KURV's existing target and runner. They require the small RTSan annotations described above before the first command is meaningful.

```bash
RUSTFLAGS="-Zsanitizer=realtime" \
  cargo +nightly run -Zbuild-std --target x86_64-unknown-linux-gnu \
  --example process_lab -- 64 200 1 stress4-filter-mod 64 48000

RUSTFLAGS="-Zsanitizer=address" \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu --lib

RUSTFLAGS="-Zsanitizer=thread" \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu \
  --lib voices::internal_rt_pool -- --test-threads=1
```

Passing these proves only the exercised paths. RTSan also cannot prove deadlines, and sanitizer builds must never be used for DSP performance comparisons.

## Evaluated and rejected

- [Spotify Pedalboard](https://github.com/spotify/pedalboard) is maintained and offers simple headless VST3/AU instrument rendering with MIDI and selectable buffer size on Linux, Windows, and macOS. [Official API](https://spotify.github.io/pedalboard/reference/pedalboard.html#pedalboard.ExternalPlugin.process) It is a good fallback if DawDreamer proves unreliable, but running both is redundant; DawDreamer wins here because its documented audio-rate parameter automation and plugin state API better match KURV's failure modes.
- [Carla](https://github.com/falkTX/Carla) is a maintained multi-format host with OSC control, but its official feature list names VST2, VST3, AU, LADSPA, DSSI, and LV2—not CLAP—and it is not a conformance or RT-safety checker. [README](https://github.com/falkTX/Carla#features) It adds less deterministic stress coverage than DawDreamer.
- [DawDreamer's predecessor RenderMan](https://github.com/fedden/RenderMan), MrsWatson-style VST2 hosts, LV2 linters, and Apple `auval` do not target KURV's shipped CLAP/VST3 pair or add stronger checks than the retained validators.
- [Miri](https://github.com/rust-lang/miri) is excellent for Rust undefined behavior, alignment, aliasing, and some data races, but it cannot access most FFI/platform APIs and interprets code very slowly. KURV's full plugin/editor path is therefore a poor fit. Trial it only on a small existing pure-Rust unsafe-code check; do not create a parallel Miri-specific architecture.
- [Loom](https://github.com/tokio-rs/loom) systematically permutes concurrent executions, but requires synchronization primitives behind `cfg(loom)` and dedicated model tests. Its own documentation notes incomplete C11 coverage, including weaker treatment of `SeqCst`. [README](https://github.com/tokio-rs/loom#unsupported-features) That source churn is not justified while TSan and the existing pool stress checks have no confirmed race to minimize.
- A custom no-allocation global allocator, Valgrind/Helgrind, extra metadata scanners, and generic CPU-load generators are redundant once RTSan, ASan/TSan, the format validators, and the existing `process_lab` are in place. CPU starvation can induce xruns, but it does not localize a KURV defect and is not a correctness gate.

## Coverage boundary

This stack still does not prove Bitwig, REAPER, FL Studio, or Ableton behavior, editor embedding, GPU/backend startup, Windows teardown, or real-time deadline compliance on customer hardware. Keep at least one release-binary smoke in each supported DAW/platform family. Validator success and offline rendering are evidence, not substitutes for those host checks.
