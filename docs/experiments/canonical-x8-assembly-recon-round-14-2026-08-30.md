# Canonical x8 assembly reconnaissance (round 14, 2026-08-30)

## Question

Does native code generation leave enough concrete headroom to justify hand-written intrinsics or assembly for the current optimized cubic/support-2 saw residual, or the round-11 support-3/degree-7 Estrin quality kernel?

This round changes no production code. The retained probe is under `cfg(test)` through `oscillators::va::experiment`, exports stable symbols, and exercises the real `accumulate_saw8_block` current path beside a structurally matched support-3 block loop.

## Reproduction

Host: Ryzen 7 7800X3D (Zen 4), rustc 1.98.0 / LLVM 22.1.8. The native build enables AVX-512 mask instructions around 256-bit arithmetic, so these results are host-native rather than a generic shipping-ISA guarantee.

```text
CARGO_PROFILE_RELEASE_STRIP=none RUSTFLAGS='-C target-cpu=native' \
  cargo test canonical_x8_symbol_probe --lib --release --locked --no-run

nm -S --size-sort target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
  | rg 'kurv_probe_(current|support3)_x8_blocks'

llvm-objdump --no-show-raw-insn -d \
  --disassemble-symbols=kurv_probe_current_x8_blocks,kurv_probe_support3_x8_blocks \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce

KURV_ASM_PROBE=current KURV_ASM_BLOCKS=200000 KURV_ASM_HZ=7040 \
  taskset -c 8 perf stat -r 5 \
  -e cycles:u,instructions:u,branches:u,branch-misses:u,L1-dcache-loads:u,L1-dcache-load-misses:u -- \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
  oscillators::va::experiment::canonical_x8_symbol_probe \
  --ignored --exact --nocapture --test-threads=1
```

Repeat the last command with `KURV_ASM_PROBE=support3`. The workload is 200,000 64-frame blocks, eight lanes, 48 kHz, 7040 Hz (M105 vicinity), with accumulation and phase advancement included.

## Measurements

| kernel | cycles | instructions | branches | branch misses | L1 loads | L1 misses |
|---|---:|---:|---:|---:|---:|---:|
| current support-2 cubic | 235,045,231 +/-0.87% | 600,607,445 | 56,611,272 | 73,787 | 78,961,199 | 53,864 |
| support-3 degree-7 Estrin | 239,759,940 +/-0.85% | 581,890,426 | 26,574,060 | 18,406 | 52,480,165 | 56,508 |

At twice the workload, additional Zen-4 counters showed approximately 981.1 M packed-256 FP uops and 1.69 M front-end-stalled cycles for current, versus 1,075.2 M (+9.6%) and 0.85 M (-50%) for support-3. The divisor counters were implausibly tiny on this host and are not treated as authoritative.

Symbols were 1,513 bytes (current) and 911 bytes (support-3). The current real wrapper is branchier because it includes event plumbing and the 64-element step block. The candidate loop is compact and unrolled two samples at a time.

## Code-generation findings

- rustc already uses vector FMA, vector comparisons, AVX-512 mask registers, masked blends, and hoisted constants. There are no hot polynomial coefficient loads or stack spills.
- The support-3 polynomial has four independent first-level FMAs followed by two combining FMA levels. Its lower instruction, branch, load, and front-end counts still produce 2.0% more cycles. The evidence points to additional FP work/dependency depth, not front-end, branch, or cache pressure.
- Disassembly exposed one source-level `vdivps` in the candidate (`edge / step`). Passing a precomputed reciprocal removes it without intrinsics. A verification rebuild was attempted, but the thin-LTO link was killed under contemporaneous system memory pressure (43/46 GiB RAM and all swap occupied). Therefore the reciprocal variant is explicitly unmeasured and is not a promotion claim.
- `llvm-mca` 22.1.8 is installed, but an exact LTO hot-loop extraction would omit the masked event/control context that determines the measured result. Hardware counters and full symbol disassembly are stronger evidence here; no synthetic throughput number is substituted.

## RT, state, and publication

The probe adds no production state or publication bytes and cannot enter a release audio build. Both measured loops allocate no heap memory and perform no lock, I/O, or logging in their timed bodies. Environment access and printing occur only in the ignored test driver.

## Verdict

Assembly is premature. There is no demonstrated compiler code-generation defect: native rustc already emits the expected vector masks and FMAs, while the quality kernel loses on cycles despite substantially reducing front-end work. The only concrete defect found is expressible as ordinary Rust reciprocal plumbing and could not be remeasured under the host's link-memory pressure. No intrinsic or inline-assembly prototype is justified, and production remains unchanged.

Limitation: this probe isolates canonical saw x8 at a stable high note. It does not replace the prior rounds' quality, transition, or full-instrument evidence, and the native AVX-512 mask selection cannot be generalized to non-Zen-4 shipping hosts.
