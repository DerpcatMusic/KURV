#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Mirrors .github/workflows/ci.yml. Keep the two in step.
#
# Full-plugin checks use the authentic licensing backend, as CI does.
# Restore the pinned private siblings and drag-and-drop source first.
core_features=(--no-default-features --features clap,vst3,licensing)

python3 scripts/ci/check_build_inputs.py
python3 scripts/ci/check_format.py

if rg -n --glob '*.rs' '\b(dbg|todo|unimplemented)!\s*\(' src; then
    echo "Rust quality gate: remove the debug or placeholder macro above." >&2
    exit 1
fi

# --all-targets so lints denied in Cargo.toml (notably
# undocumented_unsafe_blocks) also run over test and example code. Those
# deny-level lints fail the build by themselves. The warn-level pedantic and
# nursery backlog is too large to gate with `-D warnings`, so the ratchet holds
# it instead: the count may shrink but never grow.
cargo clippy --locked -p pure_va_dispersion_core --all-targets "${core_features[@]}" \
    --message-format json | python3 scripts/clippy-ratchet.py

cargo test --locked -p pure_va_dispersion_core --lib "${core_features[@]}"

printf 'Rust quality gate passed.\n'
