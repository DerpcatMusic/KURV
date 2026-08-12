#!/usr/bin/env bash
set -euo pipefail

repo_dir=${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
target_dir=${2:-$repo_dir/target/linux-glibc217}
image=localhost/kurv-linux-build:glibc217-rust1971-truce63-base0a42cb7

command -v podman >/dev/null || {
    echo "error: podman is required for portable Linux bundles" >&2
    exit 1
}

podman build --pull=missing -t "$image" \
    -f "$repo_dir/packaging/linux/Containerfile" "$repo_dir/packaging/linux"
mkdir -p "$target_dir"
podman run --rm \
    -e CARGO_TARGET_DIR=/target \
    -e RUSTUP_TOOLCHAIN=1.97.1-x86_64-unknown-linux-gnu \
    -v "$repo_dir:/workspace" \
    -v "$target_dir:/target" \
    -w /workspace \
    "$image" \
    cargo truce build --clap --vst3 -p pure_va_dispersion_core --target-cpu baseline

"$repo_dir/scripts/check-linux-glibc.sh" 2.17 \
    "$target_dir/bundles/KURV.clap" "$target_dir/bundles/KURV.vst3"
