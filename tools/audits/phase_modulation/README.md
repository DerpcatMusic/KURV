# Phase-modulation diagnostic

The executable now shares the current-source VA adapter with ../pm_integration/build.rs. See that harness's README for SIMD substitution and omitted framework plumbing. Old reports and CSV files remain historical evidence from their recorded revisions.

The PM/direct-tuning difference printed by this diagnostic is not an alias-only metric and has no acceptance threshold. A successful process exit indicates execution, not DSP quality acceptance. Use the current integration harness for asserted scalar/backend and partition comparisons.
