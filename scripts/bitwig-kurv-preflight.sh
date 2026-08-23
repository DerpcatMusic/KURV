#!/usr/bin/env bash
# Read-only release-gate preflight for KURV instances hosted by Bitwig.
# This script never signals a process or changes Bitwig/project state.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/bitwig-kurv-preflight.sh [--require-match]

Print the current KURV candidate and every live Bitwig KURV host mapping.
--require-match exits 4 unless every discovered host maps the candidate hash.
The script is read-only and never signals a process.
EOF
}

require_match=0
case "${1:-}" in
    "") ;;
    --require-match) require_match=1 ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 64 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 64; }

artifact_root="${KURV_ARTIFACT_ROOT:-/mnt/Windows11/DEV_WORKSPACE/PluginArtifacts/KURV}"
current_clap="${artifact_root}/current/KURV.clap"

if [[ ! -f "${current_clap}" ]]; then
    printf 'error: current CLAP not found: %s\n' "${current_clap}" >&2
    exit 2
fi

current_real="$(readlink -f "${current_clap}")"
current_sha="$(sha256sum "${current_real}" | awk '{print $1}')"
printf 'candidate=%s\n' "${current_real}"
printf 'candidate_sha256=%s\n' "${current_sha}"
printf 'display=%s wayland_display=%s xdg_session_type=%s\n' \
    "${DISPLAY:-}" "${WAYLAND_DISPLAY:-}" "${XDG_SESSION_TYPE:-}"

found=0
all_match=1
for proc in /proc/[0-9]*; do
    pid="${proc##*/}"
    [[ -r "${proc}/cmdline" && -r "${proc}/maps" ]] || continue
    cmdline="$(tr '\0' ' ' < "${proc}/cmdline")"
    [[ "${cmdline}" == *BitwigPluginHost* && "${cmdline}" == *KURV* ]] || continue
    found=1
    printf 'host_pid=%s host_cmd=%s\n' "${pid}" "${cmdline}"
    mapfile -t mapped < <(awk '$NF ~ /KURV\.clap$/ { print $NF }' "${proc}/maps" | sort -u)
    if ((${#mapped[@]} == 0)); then
        all_match=0
        printf 'host_pid=%s mapped_clap=none\n' "${pid}"
        continue
    fi
    for bundle in "${mapped[@]}"; do
        if [[ -r "${bundle}" ]]; then
            mapped_sha="$(sha256sum "${bundle}" | awk '{print $1}')"
            if [[ "${mapped_sha}" == "${current_sha}" ]]; then
                match=yes
            else
                match=no
                all_match=0
            fi
            printf 'host_pid=%s mapped_clap=%s mapped_sha256=%s matches_candidate=%s\n' \
                "${pid}" "${bundle}" "${mapped_sha}" "${match}"
        else
            all_match=0
            printf 'host_pid=%s mapped_clap=%s mapped_sha256=unreadable matches_candidate=unknown\n' \
                "${pid}" "${bundle}"
        fi
    done
done

if ((found == 0)); then
    printf 'host=none\n'
    exit 3
fi
if ((require_match != 0 && all_match == 0)); then
    exit 4
fi
