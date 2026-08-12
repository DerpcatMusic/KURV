#!/usr/bin/env bash
set -euo pipefail

maximum=${1:-2.17}
shift || true
(( $# > 0 )) || {
    echo "usage: $0 [maximum-glibc] <binary-or-bundle> [...]" >&2
    exit 2
}
command -v readelf >/dev/null || {
    echo "error: readelf is required" >&2
    exit 1
}

failed=0
found=0
while IFS= read -r -d '' binary; do
    readelf -h "$binary" >/dev/null 2>&1 || continue
    found=1
    required=$(
        readelf --version-info "$binary" 2>/dev/null \
            | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' \
            | sort -Vu \
            | tail -1
    )
    required=${required:-0}
    if [[ $(printf '%s\n%s\n' "$maximum" "$required" | sort -V | tail -1) != "$maximum" ]]; then
        echo "error: $binary requires GLIBC_$required (maximum is GLIBC_$maximum)" >&2
        failed=1
    else
        echo "$binary: GLIBC_$required"
    fi
done < <(find -L "$@" -type f -print0)
(( found > 0 )) || {
    echo "error: no ELF binaries found" >&2
    exit 1
}
exit "$failed"
