#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Mirrors .github/workflows/ci.yml. Keep the two in step.
#
# The synthesis core builds without the private `derpcat-access` licensing
# crate, so the gate runs on that feature set: it is what CI enforces and what
# a bare checkout can reproduce.
core_features=(--no-default-features --features clap,vst3)

cargo fmt --all -- --check

if rg -n --glob '*.rs' '\b(dbg|todo|unimplemented)!\s*\(' src; then
    echo "Rust quality gate: remove the debug or placeholder macro above." >&2
    exit 1
fi

# --all-targets so lints denied in Cargo.toml (notably
# undocumented_unsafe_blocks) also run over test and example code. Those
# deny-level lints fail the build by themselves. The warn-level pedantic and
# nursery backlog is too large to gate with `-D warnings`, so the ratchet holds
# it instead: the count may shrink but never grow.
cargo clippy -p pure_va_dispersion_core --all-targets "${core_features[@]}" \
    --message-format json | python3 scripts/clippy-ratchet.py

cargo test -p pure_va_dispersion_core --lib "${core_features[@]}"

printf 'Rust quality gate passed.\n'
