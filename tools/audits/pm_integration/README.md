# Actual VA module integration

```sh
KURV_SIMD=avx2 cargo +1.97.1 run --release --offline --manifest-path tools/audits/pm_integration/Cargo.toml
KURV_SIMD=baseline tools/audits/pm_integration/target/release/kurv-pm-integration
```

The build script copies complete production VA, curve, warp, ratio, oversampling,
performance and numeric modules. It removes only framework serialization imports,
derives and adapters, and supplies the exact `wide` SIMD types. It substitutes no
oscillator arithmetic, storage, backend selection or render functions.

448 existing canonical cases compare actual scalar, x4/x8, time SIMD and selected
saw backend paths. Another 56 generic PM cases compare the new dispatch against
scalar sampling with heterogeneous phases/steps, zero step/depth, nested offsets,
both spline modes and pitches above/below the native PM eligibility boundary.
Final phases are exact. Both baseline and AVX2 selection pass; maximum PM error
is 0.000000075 and canonical scalar/SIMD difference is 0.000007883.

This closes the generic VA compilation/routing gap in the extracted PM-kernel
proof. It does not compile the full voice/host layer or substitute for a DAW test.
Private `derpcat-access` availability remains required for the complete plugin.
