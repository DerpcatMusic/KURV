# AVX-512 masked BLEP/BLAMP residual (2026-08-30)

## Verdict

Reject a callable AVX-512 residual backend and do not add production dispatch.
The k-mask implementation is output-identical and branch-free, but a Rust
`#[target_feature]` call boundary costs about 0.84 ns per residual call.  The
full probe loses nearly every workload; only dense high-note pulse gains about
5%.

This closes per-residual ISA dispatch, not AVX-512 arithmetic entirely.  The
disassembly and call lower bound indicate that a future single whole-block
AVX-512 kernel could amortize one dispatch and deserves separate evidence if
the project reaches that assembly boundary.  Production DSP and version remain
unchanged.

## Kernel and host

The host is an AMD Ryzen 7 7800X3D (Zen 4) with AVX-512F, DQ, BW, and VL.  Rust
1.98 stable exposes the required 256-bit AVX-512VL masked intrinsics.

The test-only BLEP/BLAMP kernel uses compare-to-k-mask, `kand`/`kxor`, masked
FMA, and masked multiply for inactive/inner/outer lanes.  It has no scalar
`any()` branch.  Phase traversal, block-hoisted support, and inverse step match
the constant-step renderer.  The existing optimized coefficients and FMA
ordering are unchanged; no new antialiasing algorithm or state was introduced.

## Output

After passing the already-precomputed inverse step into the masked kernel, all
24 low/mid/high, coherent/decorrelated, saw/square/pulse/triangle comparisons
reported RMS and peak difference exactly zero at printed f32 precision.  This
is the shipping optimized residual, not the different higher-quality
branchless polynomial from the previous round.

## CPU

Release x86-64-v3, 64-frame x8 loops, 12,000 blocks, best of five:

| density / phase | shape | AVX2 ns/frame | AVX-512 ns/frame | delta |
|---|---|---:|---:|---:|
| low / decorrelated | saw | 2.506 | 4.491 | +79.23% |
| low / decorrelated | square | 3.737 | 8.604 | +130.27% |
| low / coherent | pulse37 | 3.763 | 8.477 | +125.26% |
| mid / decorrelated | saw | 4.244 | 4.681 | +10.30% |
| mid / decorrelated | square | 7.987 | 8.448 | +5.77% |
| mid / coherent | pulse37 | 7.511 | 8.609 | +14.62% |
| high / decorrelated | saw | 4.095 | 4.375 | +6.82% |
| high / coherent | square | 7.868 | 8.544 | +8.59% |
| high / decorrelated | pulse37 | 9.028 | 8.596 | -4.78% |
| high / coherent | pulse37 | 9.258 | 8.730 | -5.70% |

The triangle wrapper retains scalar corner-mask setup around the masked BLAMP
residual and loses 27-143%; it is sufficient to reject this callable seam but
is not evidence against an integrated vector whole-block triangle kernel.

No obvious Zen 4 frequency-collapse discontinuity appeared—the dense pulse
case can win—but instruction/call overhead erases the benefit elsewhere.

## Dispatch lower bound and codegen

A no-op, non-inlined AVX-512 target-feature function measured:

```text
baseline black_box       0.259 ns/call
direct target call       1.102 ns/call
direct overhead          0.842 ns/call
block feature check plus calls 1.266 ns/sample
```

The last number checks AVX-512F/VL once per 64-sample block but still calls the
target function per sample.  A production residual function would pay one call
for saw and two for pulse/square on every frame; that is not viable.

The stripped binary still permits address-based inspection.  The BLEP target
sequence at approximately `0x418780..0x418878` is about 248 bytes and contains
EVEX `vcmpps` into k registers, `kandw`/`kxorw`, masked `vfmadd213ps`, masked
`vmulps`, `vpternlogd`, and no data-dependent branch.  It ends in
`vzeroupper; ret`.  This confirms the intended masked arithmetic and also makes
the non-inlined call boundary visible.

Exact commands:

```text
CARGO_TARGET_DIR=/tmp/kurv-va-events-target RUSTFLAGS='-C target-cpu=x86-64-v3' cargo test avx512_masked_residual_report --lib --release --no-default-features --locked -- --ignored --nocapture --test-threads=1

bin=$(find /tmp/kurv-va-events-target/release/deps -maxdepth 1 -type f -perm -111 -name 'pure_va_dispersion_core-*' | head -1)
objdump -d --start-address=0x418760 --stop-address=0x418940 "$bin"
```

The final test passed 1/1 with 380 tests filtered out and the checkout's
existing 25 test-build warnings.  The probe remains test-only; no runtime
feature dispatch or dependency was added.
