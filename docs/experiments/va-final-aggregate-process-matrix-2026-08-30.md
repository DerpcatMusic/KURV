# VA final aggregate process matrix (2026-08-30)

Status: measured aggregate comparison only. This round changed no production
DSP, package version, dependency, parameter, preset, UI, or test. The only
committed artifact is this report.

## Decision

Current `f51693f01182aac52fe7e49273fda843d6e071e9` is effectively at aggregate
CPU parity with baseline `427917dc30a176b70378b02053f10f6e905ca58f` in this
headless process matrix. Across all 66 paired cells, the geometric-mean
current/baseline ratios are:

- wall time per rendered frame: `0.9870` (-1.30%);
- user-space cycles: `0.9981` (-0.19%);
- user-space retired instructions: `1.0073` (+0.73%).

This is not a universal CPU win. Current wins only 39/66 wall-time cells,
31/66 cycle cells, and 19/66 instruction cells. Canonical and custom workloads
are approximately flat. PM trends faster in wall time and cycles but not in
instructions. The fixed four-oscillator, 64-lane warp-modulation stress workload
is 1.05% faster by geometric-mean wall time while using 0.66% more cycles and
3.08% more instructions. No production promotion follows from this report.

The largest apparent wall-time win and loss disagree with their much smaller
instruction changes. That is direct evidence that one externally observed
process per revision/cell, even with three internal medians and alternating
order, retains frequency/scheduling noise. The full table is preserved below;
the aggregate and stable instruction counts are more defensible than either
outlier.

## Exact revisions and harness equivalence

Two disposable worktrees were used:

```text
baseline 427917dc30a176b70378b02053f10f6e905ca58f
current  f51693f01182aac52fe7e49273fda843d6e071e9
```

The baseline worktree contains one uncommitted `examples/process_lab.rs`
backport: 17 inserted and 3 removed lines. It adds the already-current `custom`
scenario and explicit oversampling CLI argument. It changes no DSP. After the
backport, the complete harness is byte-identical to current:

```text
git -C /tmp/kurv-va-final-baseline.fqVEMD diff --numstat -- examples/process_lab.rs
# 17  3  examples/process_lab.rs

cmp -s \
  /tmp/kurv-va-final-baseline.fqVEMD/examples/process_lab.rs \
  /tmp/kurv-va-final-current.mqHZFI/examples/process_lab.rs
# exit 0
```

The two `Cargo.toml` files differ only in the package version (`0.8.4` versus
`0.8.13`); locked dependencies, release profile, `truce.toml`, and harness
semantics are identical. The baseline backport remains uncommitted.

After timing ended, the main working tree gained uncommitted `0.8.14` host-shell
bus-layout registration and version changes. The host-boundary owner reports
those changes as DSP-identical to measured `0.8.13`; they were not present in,
rebuilt for, or folded into this matrix. The exact measured current remains the
hash above.

## Workload

Every cell uses a 64-frame block. `voices` in the harness is the number of held
note-ons sent at startup, so the ordinary matrix reports note polyphony as
1/16/32 rather than pretending every scenario has 64 notes:

- `osc`, `custom`, and `pm`: 1, 16, and 32 notes; 48 and 96 kHz; 1x, 2x,
  and 4x oversampling; 20,000 callbacks per internal repeat;
- `stress4-warp-mod`: 1 and 4 notes; the scenario itself fixes four oscillators
  at 64 unison lanes each, phase warp `0.65`, and a 5 kHz LFO routed at full
  depth to each oscillator's warp amount; 48 and 96 kHz; 1x, 2x, and 4x;
  200 callbacks per internal repeat.

That is 54 ordinary plus 12 stress cells per revision, 66 paired cells and 132
process invocations total. Each invocation performs three timed internal
repeats and reports their median. Ordinary scenarios warm for 256 callbacks;
stress warms for 16. The first note-on callback and warmup are outside the
harness's `Instant` interval but inside the process-wide `perf stat` counters.
Thus `median_ns_per_frame` measures the steady held render, while cycles and
instructions cover initialization, note-on, warmup, and all three measured
batches.

## Host and build

```text
CPU: AMD Ryzen 7 7800X3D 8-Core Processor, 8 cores / 16 threads
pin: logical CPU 8 (core 0), scaling governor performance
OS: Linux 7.2.0-rc7-1-cachyos-rc x86_64
rustc: 1.98.0 (88d9e12ae 2026-08-18), LLVM 22.1.8
cargo: 1.98.0 (797e8a9bc 2026-08-05)
perf: 7.2.1-1
```

Both binaries were built serially into separate targets. An initial `/tmp`
target ran out of tmpfs space before linking; it was removed with `cargo clean`
and neither its partial output nor its failed build was timed. The successful
commands were:

```sh
env CARGO_TARGET_DIR=/var/tmp/kurv-va-final-target-baseline \
  RUSTFLAGS='-C target-cpu=x86-64-v3' \
  cargo build --release --no-default-features \
  --example process_lab --locked

env CARGO_TARGET_DIR=/var/tmp/kurv-va-final-target-current \
  RUSTFLAGS='-C target-cpu=x86-64-v3' \
  cargo build --release --no-default-features \
  --example process_lab --locked
```

The measured binaries had SHA-256 digests:

```text
bd99945c39a6f5fe60ba3350f600876645b5bc165087d71ced69ab71c9d915f3 baseline
78097f37fb0db8e6029f13ba5a7221f3a05047f77d887f6bf4c88c7a84268d2d current
```

## Exact timing command shape

Cells were enumerated in scenario, note, sample-rate, oversampling order. Even
cell numbers ran baseline then current; odd cells ran current then baseline.
Only one process was active at a time. Each revision invocation used:

```sh
taskset -c 8 perf stat -x ';' \
  -e cycles:u,instructions:u -- \
  "$binary" 64 "$callbacks" 3 \
  "$scenario" "$notes" "$sample_rate" "$oversampling"
```

The ordinary enumeration was:

```sh
for scenario in osc custom pm; do
  for notes in 1 16 32; do
    for sample_rate in 48000 96000; do
      for oversampling in 1 2 4; do
        run_pair "$scenario" "$notes" "$sample_rate" "$oversampling" 20000
      done
    done
  done
done
```

The stress enumeration substituted `scenario=stress4-warp-mod`, notes `1 4`,
and `callbacks=200`. `perf` was first checked on CPU 8 and returned supported,
counted user-space cycles and instructions. Worker threads inherit the CPU-8
affinity; this controls host interference but is not a multicore host-throughput
measurement.

## Aggregate results

Ratios are current divided by baseline; below 1 is lower. `geo` is the
geometric mean of paired cell ratios. A win is a ratio below 1 without a noise
margin.

| group | cells | time geo C/B | time median | time wins | cycles geo C/B | cycles median | cycle wins | instr geo C/B | instr median | instr wins |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all | 66 | 0.9870 | 0.9916 | 39/66 | 0.9981 | 1.0010 | 31/66 | 1.0073 | 1.0040 | 19/66 |
| osc | 18 | 1.0011 | 0.9904 | 11/18 | 1.0080 | 1.0009 | 8/18 | 1.0008 | 0.9998 | 9/18 |
| custom | 18 | 1.0040 | 1.0035 | 8/18 | 1.0084 | 1.0034 | 7/18 | 1.0014 | 0.9999 | 9/18 |
| pm | 18 | 0.9550 | 0.9855 | 13/18 | 0.9726 | 0.9908 | 12/18 | 1.0042 | 1.0040 | 1/18 |
| stress4-warp-mod | 12 | 0.9895 | 0.9884 | 7/12 | 1.0066 | 1.0065 | 4/12 | 1.0308 | 1.0225 | 0/12 |

The worst observed wall-time/cycle cell is one-note `osc`, 96 kHz, 4x:
`1.2295x` time and `1.1530x` cycles, despite only `1.0063x` instructions. The
largest apparent win is 16-note `pm`, 48 kHz, 1x: `0.5332x` time and `0.7276x`
cycles despite `1.0040x` instructions. Neither outlier is treated as a stable
microarchitectural claim. The worst retired-instruction ratio is one-note
`stress4-warp-mod`, 96 kHz, 1x at `1.0813x`.

## Output integrity

All 132 terminal buffers are finite. Peak and checksum are terminal-buffer
smoke signals, not waveform-accuracy metrics. Canonical, PM, and stress outputs
are exact at the printed precision. Custom output differs only at floating-point
roundoff scale, consistent with the current aggregate's changed custom path;
this matrix does not substitute for the retained spectral and transition gates.

| scenario | finite | base peak range | current peak range | peak-identical cells | checksum-identical cells | max abs peak delta | max abs checksum delta |
|---|---:|---:|---:|---:|---:|---:|---:|
| osc | 36/36 | 0.045865..1.052586 | 0.045865..1.052586 | 18/18 | 18/18 | 0.000000000 | 0.000000000 |
| custom | 36/36 | 0.037062..0.874761 | 0.037062..0.874761 | 17/18 | 0/18 | 0.000001000 | 0.000367731 |
| pm | 36/36 | 0.101986..1.651803 | 0.101986..1.651803 | 18/18 | 18/18 | 0.000000000 | 0.000000000 |
| stress4-warp-mod | 24/24 | 0.106432..0.835938 | 0.106432..0.835938 | 12/12 | 12/12 | 0.000000000 | 0.000000000 |

## Full paired matrix

Every counter below is the exact integer printed by `perf`. Times are the
harness's three-repeat median in nanoseconds per rendered frame, rounded here to
the three decimals printed by the harness. `C/B` is current divided by baseline.

| cell | scenario | notes | SR | OS | base ns/f | current ns/f | time C/B | base cycles | current cycles | cycles C/B | base instructions | current instructions | instr C/B |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | osc | 1 | 48k | 1x | 128.699 | 126.469 | 0.9827 | 2465354808 | 2481940163 | 1.0067 | 6355668669 | 6415689885 | 1.0094 |
| 1 | osc | 1 | 48k | 2x | 151.103 | 146.387 | 0.9688 | 2878830897 | 2854327439 | 0.9915 | 7671804767 | 7728928750 | 1.0074 |
| 2 | osc | 1 | 48k | 4x | 167.603 | 171.645 | 1.0241 | 3227288574 | 3294800101 | 1.0209 | 9087590100 | 9138933244 | 1.0056 |
| 3 | osc | 1 | 96k | 1x | 128.328 | 124.498 | 0.9702 | 2461283321 | 2451744175 | 0.9961 | 6354714396 | 6414732080 | 1.0094 |
| 4 | osc | 1 | 96k | 2x | 146.427 | 154.547 | 1.0555 | 2813146429 | 3033864050 | 1.0785 | 7671392008 | 7728518131 | 1.0074 |
| 5 | osc | 1 | 96k | 4x | 173.930 | 213.843 | 1.2295 | 3287328904 | 3790424634 | 1.1530 | 9087185071 | 9144132669 | 1.0063 |
| 6 | osc | 16 | 48k | 1x | 236.245 | 224.226 | 0.9491 | 4403560666 | 4312589378 | 0.9793 | 13014231939 | 13030863547 | 1.0013 |
| 7 | osc | 16 | 48k | 2x | 338.766 | 336.447 | 0.9932 | 6454505939 | 6455721987 | 1.0002 | 20957430809 | 20927788225 | 0.9986 |
| 8 | osc | 16 | 48k | 4x | 552.119 | 541.719 | 0.9812 | 10482910825 | 10331531914 | 0.9856 | 35525859059 | 35403662007 | 0.9966 |
| 9 | osc | 16 | 96k | 1x | 222.638 | 223.780 | 1.0051 | 4230282676 | 4318063126 | 1.0208 | 12977836022 | 13020320036 | 1.0033 |
| 10 | osc | 16 | 96k | 2x | 336.506 | 342.391 | 1.0175 | 6380251017 | 6479214118 | 1.0155 | 20894040041 | 20917243167 | 1.0011 |
| 11 | osc | 16 | 96k | 4x | 549.427 | 547.133 | 0.9958 | 10427403592 | 10444974085 | 1.0017 | 35622286071 | 35393251114 | 0.9936 |
| 12 | osc | 32 | 48k | 1x | 328.774 | 330.085 | 1.0040 | 6217965366 | 6246478426 | 1.0046 | 20095785418 | 20066141220 | 0.9985 |
| 13 | osc | 32 | 48k | 2x | 547.416 | 539.170 | 0.9849 | 10369559812 | 10239253123 | 0.9874 | 35160700980 | 34931665650 | 0.9935 |
| 14 | osc | 32 | 48k | 4x | 972.316 | 960.225 | 0.9876 | 18340170926 | 18168261661 | 0.9906 | 63973029306 | 63665723010 | 0.9952 |
| 15 | osc | 32 | 96k | 1x | 330.144 | 322.618 | 0.9772 | 6288275089 | 6171344973 | 0.9814 | 20111517880 | 20029028919 | 0.9959 |
| 16 | osc | 32 | 96k | 2x | 589.757 | 541.186 | 0.9176 | 10824417833 | 10200090261 | 0.9423 | 35123588248 | 34894553225 | 0.9935 |
| 17 | osc | 32 | 96k | 4x | 970.297 | 978.253 | 1.0082 | 18334803422 | 18397686377 | 1.0034 | 63721574332 | 63629081555 | 0.9985 |
| 18 | custom | 1 | 48k | 1x | 129.640 | 125.742 | 0.9699 | 2470141553 | 2462032103 | 0.9967 | 6396723806 | 6456503796 | 1.0093 |
| 19 | custom | 1 | 48k | 2x | 146.758 | 147.362 | 1.0041 | 2833953134 | 2844960032 | 1.0039 | 7753593214 | 7810234880 | 1.0073 |
| 20 | custom | 1 | 48k | 4x | 173.861 | 173.188 | 0.9961 | 3305481042 | 3294693543 | 0.9967 | 9262065308 | 9306839874 | 1.0048 |
| 21 | custom | 1 | 96k | 1x | 126.664 | 125.500 | 0.9908 | 2471034404 | 2452588322 | 0.9925 | 6396725380 | 6456502271 | 1.0093 |
| 22 | custom | 1 | 96k | 2x | 144.612 | 148.825 | 1.0291 | 2801513192 | 2886448909 | 1.0303 | 7755820889 | 7810238775 | 1.0070 |
| 23 | custom | 1 | 96k | 4x | 169.552 | 171.185 | 1.0096 | 3264657590 | 3307044456 | 1.0130 | 9262067630 | 9306840937 | 1.0048 |
| 24 | custom | 16 | 48k | 1x | 239.798 | 234.966 | 0.9798 | 4615661300 | 4549349595 | 0.9856 | 13671877546 | 13684658160 | 1.0009 |
| 25 | custom | 16 | 48k | 2x | 367.135 | 362.979 | 0.9887 | 6970739584 | 6816379781 | 0.9779 | 22240967808 | 22256459499 | 1.0007 |
| 26 | custom | 16 | 48k | 4x | 698.147 | 685.253 | 0.9815 | 12507372069 | 12781210338 | 1.0219 | 38326820168 | 38189196585 | 0.9964 |
| 27 | custom | 16 | 96k | 1x | 239.585 | 250.707 | 1.0464 | 4558553350 | 4917081756 | 1.0786 | 13646028219 | 13658807980 | 1.0009 |
| 28 | custom | 16 | 96k | 2x | 357.993 | 366.558 | 1.0239 | 6816663232 | 6818376457 | 1.0003 | 22293817593 | 22256458018 | 0.9983 |
| 29 | custom | 16 | 96k | 4x | 585.795 | 595.097 | 1.0159 | 11111477834 | 11262452807 | 1.0136 | 38219983752 | 38189193940 | 0.9992 |
| 30 | custom | 32 | 48k | 1x | 345.352 | 340.351 | 0.9855 | 6558134120 | 6577097129 | 1.0029 | 21379032077 | 21341675668 | 0.9983 |
| 31 | custom | 32 | 48k | 2x | 575.736 | 577.396 | 1.0029 | 10991723648 | 10960991950 | 0.9972 | 37694581285 | 37663796172 | 0.9992 |
| 32 | custom | 32 | 48k | 4x | 1017.829 | 1040.770 | 1.0225 | 19412454247 | 19748992895 | 1.0173 | 69114817878 | 68991471187 | 0.9982 |
| 33 | custom | 32 | 96k | 1x | 343.440 | 353.517 | 1.0293 | 6532388349 | 6673310262 | 1.0216 | 21379033452 | 21341676819 | 0.9983 |
| 34 | custom | 32 | 96k | 2x | 590.038 | 574.398 | 0.9735 | 11123064067 | 10900008700 | 0.9799 | 37694581809 | 37556958615 | 0.9963 |
| 35 | custom | 32 | 96k | 4x | 1093.250 | 1122.667 | 1.0269 | 21047950928 | 21583166878 | 1.0254 | 69329634534 | 68991475320 | 0.9951 |
| 36 | pm | 1 | 48k | 1x | 141.760 | 151.878 | 1.0714 | 2691073674 | 2805579933 | 1.0426 | 6530635010 | 6593544483 | 1.0096 |
| 37 | pm | 1 | 48k | 2x | 166.920 | 197.087 | 1.1807 | 3118449569 | 3456443494 | 1.1084 | 8015609763 | 8083038164 | 1.0084 |
| 38 | pm | 1 | 48k | 4x | 199.992 | 201.926 | 1.0097 | 3731829388 | 3753454623 | 1.0058 | 9782457914 | 9845368679 | 1.0064 |
| 39 | pm | 1 | 96k | 1x | 190.761 | 143.569 | 0.7526 | 3375393904 | 2695359606 | 0.7985 | 6529582573 | 6595325737 | 1.0101 |
| 40 | pm | 1 | 96k | 2x | 332.996 | 311.760 | 0.9362 | 4206439789 | 4217644001 | 1.0027 | 8019076115 | 8081986996 | 1.0078 |
| 41 | pm | 1 | 96k | 4x | 468.800 | 459.729 | 0.9807 | 4995278715 | 4968373098 | 0.9946 | 9773575563 | 9836488514 | 1.0064 |
| 42 | pm | 16 | 48k | 1x | 577.027 | 307.688 | 0.5332 | 7718717456 | 5615810960 | 0.7276 | 15781341296 | 15844251655 | 1.0040 |
| 43 | pm | 16 | 48k | 2x | 512.139 | 550.796 | 1.0755 | 9300071555 | 9824783405 | 1.0564 | 26452974772 | 26460755613 | 1.0003 |
| 44 | pm | 16 | 48k | 4x | 777.502 | 755.205 | 0.9713 | 14944338128 | 14451455561 | 0.9670 | 46582156415 | 46645069252 | 1.0014 |
| 45 | pm | 16 | 96k | 1x | 278.579 | 276.475 | 0.9924 | 5367092262 | 5318476516 | 0.9909 | 15726334471 | 15789247640 | 1.0040 |
| 46 | pm | 16 | 96k | 2x | 450.461 | 444.451 | 0.9867 | 8574810876 | 8440992498 | 0.9844 | 26426110855 | 26433885516 | 1.0003 |
| 47 | pm | 16 | 96k | 4x | 783.696 | 760.947 | 0.9710 | 14692131619 | 14526755091 | 0.9887 | 46447780649 | 46619819151 | 1.0037 |
| 48 | pm | 32 | 48k | 1x | 455.257 | 434.329 | 0.9540 | 8570760787 | 8363061095 | 0.9758 | 25671325488 | 25789370077 | 1.0046 |
| 49 | pm | 32 | 48k | 2x | 763.580 | 782.435 | 1.0247 | 14642310111 | 14707788673 | 1.0045 | 46195402311 | 46258313548 | 1.0014 |
| 50 | pm | 32 | 48k | 4x | 1416.457 | 1409.710 | 0.9952 | 26841497213 | 26283256897 | 0.9792 | 85911077841 | 85756882390 | 0.9982 |
| 51 | pm | 32 | 96k | 1x | 440.465 | 439.972 | 0.9989 | 8346385765 | 8299715534 | 0.9944 | 25576768003 | 25694817536 | 1.0046 |
| 52 | pm | 32 | 96k | 2x | 779.128 | 760.532 | 0.9761 | 14581261402 | 14445839822 | 0.9907 | 46100848078 | 46163760760 | 1.0014 |
| 53 | pm | 32 | 96k | 4x | 1425.841 | 1403.607 | 0.9844 | 27398794233 | 26498458128 | 0.9671 | 85605095911 | 85885107153 | 1.0033 |
| 54 | stress4-warp-mod | 1 | 48k | 1x | 2477.147 | 2282.228 | 0.9213 | 464658591 | 454373806 | 0.9779 | 781296908 | 844373722 | 1.0807 |
| 55 | stress4-warp-mod | 1 | 48k | 2x | 4065.654 | 4014.350 | 0.9874 | 768407722 | 780153673 | 1.0153 | 1486448073 | 1549536231 | 1.0424 |
| 56 | stress4-warp-mod | 1 | 48k | 4x | 7689.418 | 7603.718 | 0.9889 | 1447115694 | 1443500715 | 0.9975 | 2887414773 | 2950988891 | 1.0220 |
| 57 | stress4-warp-mod | 1 | 96k | 1x | 2219.588 | 2183.402 | 0.9837 | 409693074 | 414039992 | 1.0106 | 776300908 | 839388875 | 1.0813 |
| 58 | stress4-warp-mod | 1 | 96k | 2x | 4233.841 | 4146.204 | 0.9793 | 759690897 | 756529850 | 0.9958 | 1482341366 | 1545896232 | 1.0429 |
| 59 | stress4-warp-mod | 1 | 96k | 4x | 7763.774 | 7894.795 | 1.0169 | 1393525100 | 1461571706 | 1.0488 | 2884767950 | 2948345614 | 1.0220 |
| 60 | stress4-warp-mod | 4 | 48k | 1x | 7283.193 | 7322.185 | 1.0054 | 1435215358 | 1449125241 | 1.0097 | 2720009830 | 2782635639 | 1.0230 |
| 61 | stress4-warp-mod | 4 | 48k | 2x | 14018.292 | 14075.144 | 1.0041 | 2711270737 | 2733707657 | 1.0083 | 5319140010 | 5381284881 | 1.0117 |
| 62 | stress4-warp-mod | 4 | 48k | 4x | 27320.858 | 26989.887 | 0.9879 | 5313973373 | 5273480909 | 0.9924 | 10518423246 | 10581979343 | 1.0060 |
| 63 | stress4-warp-mod | 4 | 96k | 1x | 7826.933 | 7661.107 | 0.9788 | 1389610179 | 1396172805 | 1.0047 | 2696972355 | 2760543759 | 1.0236 |
| 64 | stress4-warp-mod | 4 | 96k | 2x | 14917.025 | 15205.793 | 1.0194 | 2650647029 | 2692868634 | 1.0159 | 5301797724 | 5364889144 | 1.0119 |
| 65 | stress4-warp-mod | 4 | 96k | 4x | 29817.965 | 29954.419 | 1.0046 | 5207232623 | 5224626060 | 1.0033 | 10506520199 | 10567228839 | 1.0058 |

## Interpretation boundary

This comparison answers whether the complete current DSP checkout remains
performance-safe against the chosen historical baseline under four honest
headless workloads. It does not isolate any one commit in the intervening
history. It also does not measure plugin format overhead, GUI work, automation
bursts, note release/steal, DAW scheduling, multicore host throughput, package
validation, or human audition.

The defensible conclusion is therefore narrow: current retains approximate
aggregate process cost while preserving finite output across the matrix, but it
does not beat baseline in every cell or every hardware metric. The stress path's
retired-instruction increase is real enough to keep as a future optimization
target; the wall-time outliers require repeated external runs before attribution.
