# Dependency recovery

Supersedes the earlier missing-source status in ci-repair-2026-09-05.md.

Found authentic drag-and-drop in https://github.com/DerpcatMusic/matari-audio-drag-and-drop at 56b81b315ea3582a27574bcc65c1c9e0175c7865, package version 0.1.6. CI and signed macOS builds now share a checkout of this revision into the existing vendor path. This upstream revision is a proposed pinned replacement for the absent local tree, not a claim that unknown local patches were recovered. Full integration remains to be compiled.

The missing licensing entry point was recoverable from KURV history: d084681:src/licensing.rs is byte-identical to the current src/licensing/backend.rs. Commit12b1aa4 moved the implementation without tracking an entry point. A minimal src/licensing/mod.rs reconnects that implementation; no licensing logic, keys, feature checks or test exceptions were changed. Full-plugin CI now explicitly enables licensing, since no authentic nonlicensing backend is present.

With the confirmed drag-and-drop source restored locally and the entry point repaired, the preflight reports only ../derpcat-access/Cargo.toml. The two private repositories named in the existing release workflow return 404 via the connected GitHub account: Matari-Audio/derpcat-access and Matari-Audio/derpcat-activation. A 404 does not establish that they do not exist; this connection may lack organization/private-repository access. They are absent from the repositories visible through this connection.

The earlier Unknown tool connector failures have cleared for branch creation and repository reads. Publication is being retried. No whole-plugin build or speedup is claimed from resolving these source paths. Signed-release and CI use the same restore action to prevent dependency drift.

## GitHub execution update

Published as PR #20 (https://github.com/DerpcatMusic/KURV/pull/20). CI run33967122075 successfully restored all pinned dependencies and compiled the full plugin; source formatting and the public DSP job passed. The Actions token has dependency access despite this chat connection returning404. The first lint run reported6764 warnings against the unchanged6523 baseline. A follow-up applies230 numeric-literal underscore formatting fixes with exact spelling equivalence checks and50 equivalent PoisonError::into_inner closure simplifications; no DSP arithmetic or warning suppression is involved. Full tests and the follow-up warning count remain to be observed.
