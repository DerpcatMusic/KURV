# Open-source VST3 CPU benchmark hosts (2026-08-08)

## Recommendation

Use a small purpose-built Linux benchmark executable built on the official
[Steinberg VST3 hosting API](https://github.com/steinbergmedia/vst3_public_sdk/tree/master/samples/vst-hosting),
or fork [shakfu/minihost](https://github.com/shakfu/minihost) if its minimal
host loop is sufficient. Neither existing app is a complete deterministic
benchmark runner, but this is the shortest path to explicit sample-rate,
block-size, MIDI, parameter/state, warm-up, and CPU-measurement control.

If modifying/building a host is out of scope, use [pluginval](https://github.com/Tracktion/pluginval)
for repeatable validation and smoke measurements, not for the requested
benchmark matrix. It is the best ready-made CLI/Linux validation tool here,
but its official command-line surface is validation-oriented rather than a
named-scenario benchmark API.

## Comparison

| Project | Linux | Deterministic load/state/parameters | Fixed MIDI/polyphony | SR/block/quality matrix | CPU result | Verdict |
|---|---|---|---|---|---|---|
| [minihost](https://github.com/shakfu/minihost) | Native source build | Minimal host; no documented complete scenario format | Host-level MIDI possible; no documented scripted polyphony runner | Host controls are useful starting points; no documented benchmark matrix | No documented stable benchmark report | Best small fork candidate |
| [pluginval](https://github.com/Tracktion/pluginval) | Official Linux target/builds | Loads VST3 and exercises state/parameters through validation tests; not a user-named scenario runner | Validation MIDI tests exist, but no documented fixed performance script | Tests multiple host conditions, not a user-defined quality/SR/block matrix | Validation diagnostics, not a controlled CPU benchmark | Best ready-made validator; not the benchmark |
| [JUCE AudioPluginHost](https://github.com/juce-framework/JUCE/tree/master/extras/AudioPluginHost) | JUCE supports Linux; build the extra | GUI/manual graph and plugin state; no headless benchmark protocol | Manual MIDI routing/playing | Manual host settings; no batch matrix | Useful host meter, not a reproducible CLI result | Good manual reference host |
| [Carla](https://github.com/falkTX/Carla) | First-class Linux host/packages | Can save projects and control a graph; deterministic named parameter/state runs require external control/script work | MIDI routing and generators are available | Engine settings and DSP load are exposed, but a benchmark matrix is not the product surface | Real-time DSP-load display | Best existing app for interactive Linux experiments |
| [Steinberg VST3 test host](https://github.com/steinbergmedia/vst3_public_sdk/tree/master/samples/vst-hosting) | SDK has Linux build support | Reference hosting code, not a finished scenario runner | VST3 event hosting is available to code | Must be implemented in the harness | Must be implemented in the harness | Best standards-based foundation |
| Dedicated benchmark | No clearly maintained, official, cross-platform VST3 benchmark was identified in these source trees | — | — | — | — | Build the narrow harness instead of adopting an unverified third-party tool |

## What the harness must own

For each run, record plugin path/version/hash, format, sample rate, maximum
block size, actual block sequence, channel layout, named parameter values,
serialized state hash, MIDI event list (including note-on velocity and note-off
times), warm-up/discard interval, measured interval, CPU clock source, and
host/plugin errors. Set parameters/state before activation, use the same
offline event schedule for every case, warm up before measuring, and report
median plus worst-case CPU. “Quality” is only comparable when it is a named
plugin parameter/state value.

## Linux and yabridge/Wine

Native Linux VST3 is the clean benchmark lane: use a native Linux build of the
plugin and a native Linux host, with no Wine bridge in the measured path.
JUCE, Carla, pluginval, minihost, and the Steinberg SDK host are all usable as
Linux-source/build starting points according to their official project trees.

[yabridge](https://github.com/robbert-vdh/yabridge) makes Windows VST2/VST3
plugins visible to Linux hosts by running the plugin under Wine and presenting
a Linux-side bridge. Its own [README](https://github.com/robbert-vdh/yabridge)
documents the Wine requirement and host/plugin bridge model. A yabridged run
is therefore a separate compatibility benchmark: pin the Wine prefix,
Wine/yabridge versions, bridge architecture, CPU governor, and plugin binary,
and report bridge/host overhead separately where possible. Do not compare a
Windows-under-Wine result directly with a native Linux VST3 result and call it
plugin CPU.

### Bottom line

For KURV’s requested matrix: fork minihost or use Steinberg’s hosting sample as
the thin core, add one deterministic scenario file/CLI, and measure around the
plugin `process` call. Keep pluginval as a regression/compatibility gate and
Carla/JUCE AudioPluginHost as interactive sanity-check hosts.
