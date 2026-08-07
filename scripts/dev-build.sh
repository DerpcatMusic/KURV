#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
stable_installer="${KURV_STABLE_INSTALLER:-/mnt/Windows11/DEV_PROJECTS/Repos/scripts/stable-plugin-install.sh}"
artifact_root="${PLUGIN_ARTIFACT_ROOT:-/mnt/Windows11/DEV_WORKSPACE/PluginArtifacts}"
clap_dir="${CLAP_INSTALL_DIR:-$HOME/.clap}"
vst3_dir="${VST3_INSTALL_DIR:-$HOME/.vst3}"
hot_target="${KURV_HOT_TARGET_DIR:-/mnt/Windows11/DEV_WORKSPACE/BuildScratch/plugins/hot/kurv}"
static_target="${KURV_STATIC_TARGET_DIR:-/mnt/Windows11/DEV_WORKSPACE/BuildScratch/plugins/static/kurv}"
mode="${KURV_TRUCE_MODE:-static}"
watch=0

usage() {
  cat <<'EOF'
Usage: scripts/dev-build.sh [--hot|--static] [--watch|--once]

Build KURV and publish it to the host-visible managed plugin paths.

  --hot       Build a Truce shell plus debug logic, publish it,
              and keep the Truce logic watcher running.
  --static    Build a normal release bundle, publish it, and exit (default).
  --watch     Keep the hot logic watcher running after the initial build.
  --once      Build/publish once without starting the watcher.
EOF
}

while (($#)); do
  case "$1" in
    --hot)
      mode=hot
      watch=1
      ;;
    --static)
      mode=static
      watch=0
      ;;
    --watch)
      watch=1
      ;;
    --once)
      watch=0
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$mode" in
  hot)
    target="$hot_target"
    build=(cargo truce build --clap --vst3 --shell --debug)
    ;;
  static)
    target="$static_target"
    build=(cargo truce build --clap --vst3)
    ;;
  *)
    echo "KURV_TRUCE_MODE must be hot or static" >&2
    exit 2
    ;;
esac

[[ -x "$stable_installer" ]] || {
  echo "missing executable stable installer: $stable_installer" >&2
  exit 1
}

cd -- "$repo_root"
export CARGO_TARGET_DIR="$target"

echo "==> KURV $mode build from $repo_root"
"${build[@]}"

clap_bundle="$target/bundles/KURV.clap"
vst3_bundle="$target/bundles/KURV.vst3"
[[ -f "$clap_bundle" && -d "$vst3_bundle" ]] || {
  echo "Truce build completed without both KURV bundles under $target/bundles" >&2
  exit 1
}

# The older workflow could leave a real VST3 directory in the host path,
# which the atomic installer correctly refuses to overwrite. Move that
# legacy entry aside once, preserving it as a recoverable backup; all future
# publishes are symlink swaps.
for destination in "$clap_dir/KURV.clap" "$vst3_dir/KURV.vst3"; do
  if [[ -e "$destination" && ! -L "$destination" ]]; then
    backup="$destination.previous-$(date -u +%Y%m%dT%H%M%S)-$$"
    mv -- "$destination" "$backup"
    echo "Moved legacy host bundle to $backup"
  fi
done

CLAP_INSTALL_DIR="$clap_dir" \
VST3_INSTALL_DIR="$vst3_dir" \
PLUGIN_ARTIFACT_ROOT="$artifact_root" \
  "$stable_installer" \
    --id KURV \
    --clap "$clap_bundle" \
    --vst3 "$vst3_bundle" \
    --clap-name KURV.clap \
    --vst3-name KURV.vst3

[[ -L "$clap_dir/KURV.clap" && -L "$vst3_dir/KURV.vst3" ]] || {
  echo "KURV publish did not leave managed host symlinks" >&2
  exit 1
}

if [[ "$mode" == hot ]]; then
  sidecar="$HOME/.truce/shell/pure_va_dispersion_core.path"
  logic="$target/debug/libpure_va_dispersion_core.so"
  [[ -f "$logic" && -f "$sidecar" ]] || {
    echo "hot build did not produce the Truce logic dylib and sidecar" >&2
    exit 1
  }
  expected_logic="$(readlink -f -- "$logic")"
  actual_logic="$(readlink -f -- "$(sed -n '1p' "$sidecar")")"
  [[ "$actual_logic" == "$expected_logic" ]] || {
    echo "Truce shell sidecar points at $actual_logic, expected $expected_logic" >&2
    exit 1
  }
  echo "Hot shell sidecar: $sidecar -> $actual_logic"
  echo "Host CLAP: $(readlink -f -- "$clap_dir/KURV.clap")"
  echo "Host VST3: $(readlink -f -- "$vst3_dir/KURV.vst3")"

  if ((watch)); then
    command -v bacon >/dev/null || {
      echo "bacon is required for --watch; rerun with --once for a one-shot publish" >&2
      exit 1
    }
    echo "==> Watching KURV logic with Truce hot reload; Ctrl-C stops the watcher"
    exec bacon --headless --job hot --no-listen
  fi
else
  echo "Host CLAP: $(readlink -f -- "$clap_dir/KURV.clap")"
  echo "Host VST3: $(readlink -f -- "$vst3_dir/KURV.vst3")"
fi
