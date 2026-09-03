#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_root="${PLUGIN_ARTIFACT_ROOT:-/mnt/Windows11/DEV_WORKSPACE/PluginArtifacts}/KURV"
target_dir="${KURV_NATIVE_TARGET_DIR:-/home/derpcat/projects/target-kurv-native}"
portable_target_dir="${KURV_STATIC_TARGET_DIR:-/mnt/Windows11/DEV_WORKSPACE/BuildScratch/plugins/static/kurv}"
clap_dir="${CLAP_INSTALL_DIR:-$HOME/.clap}"
vst3_dir="${VST3_INSTALL_DIR:-$HOME/.vst3}"
portable=false

usage() {
  cat <<'EOF'
Usage: scripts/dev-build.sh [--once] [--release]

Build fast local CLAP and VST3 bundles, publish them under
PluginArtifacts/KURV, and atomically install them through the stable
PluginArtifacts/KURV/current symlink.

--release builds the portable glibc 2.17 release bundles in Podman.
--static and --once remain accepted for compatibility.
EOF
}

while (($#)); do
  case "$1" in
    --static|--once)
      ;;
    --release)
      portable=true
      target_dir="$portable_target_dir"
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --hot|--watch)
      echo "$1 is no longer supported: dev-build.sh always publishes release bundles" >&2
      exit 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

atomic_link() {
  local target="$1"
  local destination="$2"
  local parent temporary
  parent="$(dirname -- "$destination")"
  temporary="$parent/.KURV-link-$$"

  mkdir -p -- "$parent"
  if [[ -e "$destination" && ! -L "$destination" ]]; then
    archive_legacy "$destination"
  fi
  ln -s -- "$target" "$temporary"
  mv -Tf -- "$temporary" "$destination"
}

archive_legacy() {
  local source="$1"
  local legacy_root="$artifact_root/legacy-host-installs"
  local destination="$legacy_root/$(basename -- "$source")"
  mkdir -p -- "$legacy_root"
  if [[ -e "$destination" || -L "$destination" ]]; then
    destination="$destination-$(date -u +%Y%m%dT%H%M%S)-$$"
  fi
  mv -- "$source" "$destination"
  echo "Archived stale KURV install at $destination"
}

archive_scan_backups() {
  local scan_dir="$1"
  local bundle_name="$2"
  local stale
  while IFS= read -r -d '' stale; do
    archive_legacy "$stale"
  done < <(find "$scan_dir" -maxdepth 1 -mindepth 1 -name "$bundle_name.previous-*" -print0)
}

cd -- "$repo_root"

if [[ "$portable" == true ]]; then
  echo "==> Building KURV portable release CLAP + VST3"
  scripts/build-linux-bundles.sh "$repo_root" "$target_dir"
else
  echo "==> Building KURV fast local CLAP + VST3"
  build_env=(env CARGO_TARGET_DIR="$target_dir" RUSTFLAGS="-C target-cpu=native -C target-feature=-avx512f -C link-arg=-fuse-ld=lld")
  if command -v sccache >/dev/null; then
    build_env+=(RUSTC_WRAPPER=sccache)
  fi
  "${build_env[@]}" cargo build --profile iter --lib --no-default-features --features clap,vst3
  mkdir -p -- "$target_dir/bundles/KURV.vst3/Contents/x86_64-linux"
  cp -- "$target_dir/iter/libpure_va_dispersion_core.so" "$target_dir/bundles/KURV.clap"
  cp -- "$target_dir/iter/libpure_va_dispersion_core.so" \
    "$target_dir/bundles/KURV.vst3/Contents/x86_64-linux/KURV.so"
fi

clap_bundle="$target_dir/bundles/KURV.clap"
vst3_bundle="$target_dir/bundles/KURV.vst3"
[[ -f "$clap_bundle" ]] || {
  echo "missing CLAP bundle: $clap_bundle" >&2
  exit 1
}
[[ -d "$vst3_bundle" ]] || {
  echo "missing VST3 bundle: $vst3_bundle" >&2
  exit 1
}

mkdir -p -- "$artifact_root"
build_name="build-$(date -u +%Y%m%dT%H%M%S)-$$"
staging="$artifact_root/.staging-$build_name"
published="$artifact_root/$build_name"
trap 'rm -rf -- "$staging"' EXIT
mkdir -- "$staging"
cp -- "$clap_bundle" "$staging/KURV.clap"
cp -a -- "$vst3_bundle" "$staging/KURV.vst3"
if [[ "$portable" == true ]]; then
  scripts/check-linux-glibc.sh 2.17 "$staging/KURV.clap" "$staging/KURV.vst3"
fi
mv -- "$staging" "$published"

mkdir -p -- "$clap_dir" "$vst3_dir"
archive_scan_backups "$clap_dir" KURV.clap
archive_scan_backups "$vst3_dir" KURV.vst3

if [[ -e "$artifact_root/current" && ! -L "$artifact_root/current" ]]; then
  echo "$artifact_root/current must be a symlink, refusing to replace it" >&2
  exit 1
fi
atomic_link "$build_name" "$artifact_root/current"
atomic_link "$artifact_root/current/KURV.clap" "$clap_dir/KURV.clap"
atomic_link "$artifact_root/current/KURV.vst3" "$vst3_dir/KURV.vst3"

expected_clap="$published/KURV.clap"
expected_vst3="$published/KURV.vst3"
actual_clap="$(readlink -f -- "$clap_dir/KURV.clap")"
actual_vst3="$(readlink -f -- "$vst3_dir/KURV.vst3")"
[[ "$actual_clap" == "$expected_clap" && -f "$actual_clap" ]] || {
  echo "CLAP link validation failed: $actual_clap" >&2
  exit 1
}
[[ "$actual_vst3" == "$expected_vst3" && -d "$actual_vst3" ]] || {
  echo "VST3 link validation failed: $actual_vst3" >&2
  exit 1
}
[[ "$(readlink -- "$clap_dir/KURV.clap")" == "$artifact_root/current/KURV.clap" ]] || {
  echo "CLAP host link does not pass through current" >&2
  exit 1
}
[[ "$(readlink -- "$vst3_dir/KURV.vst3")" == "$artifact_root/current/KURV.vst3" ]] || {
  echo "VST3 host link does not pass through current" >&2
  exit 1
}
[[ -z "$(find "$clap_dir" -maxdepth 1 -mindepth 1 -name 'KURV.clap.previous-*' -print -quit)" ]] || {
  echo "stale KURV CLAP backup remains in $clap_dir" >&2
  exit 1
}
[[ -z "$(find "$vst3_dir" -maxdepth 1 -mindepth 1 -name 'KURV.vst3.previous-*' -print -quit)" ]] || {
  echo "stale KURV VST3 backup remains in $vst3_dir" >&2
  exit 1
}
if [[ "$portable" == true ]]; then
  scripts/check-linux-glibc.sh 2.17 "$actual_clap" "$actual_vst3"
fi

echo "Published: $published"
echo "Current:   $(readlink -f -- "$artifact_root/current")"
echo "CLAP:      $clap_dir/KURV.clap -> $actual_clap"
echo "VST3:      $vst3_dir/KURV.vst3 -> $actual_vst3"
