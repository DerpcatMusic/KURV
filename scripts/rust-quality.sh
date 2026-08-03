#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo fmt --all -- --check
if rg -n --glob '*.rs' '\b(dbg|todo|unimplemented)!\s*\(' src; then
    echo "Rust quality gate: remove the debug or placeholder macro above." >&2
    exit 1
fi
cargo clippy -p pure_va_dispersion_core --lib
printf 'Rust quality gate passed.\n'
