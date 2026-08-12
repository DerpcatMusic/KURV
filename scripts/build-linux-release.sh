#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_dir"

if [[ -n $(git status --porcelain --untracked-files=all) ]]; then
    echo "error: commit or stash every change before producing a release" >&2
    exit 1
fi

for command_name in git gzip podman readelf sha256sum tar; do
    command -v "$command_name" >/dev/null || {
        echo "error: missing required command: $command_name" >&2
        exit 1
    }
done

snapshot_dir=$(mktemp -d)
stage_dir=$(mktemp -d)
trap 'rm -rf -- "$snapshot_dir" "$stage_dir"' EXIT
git archive --format=tar HEAD | tar -xf - -C "$snapshot_dir"

version=$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' "$snapshot_dir/Cargo.toml")
[[ -n "$version" ]] || {
    echo "error: package version was not found" >&2
    exit 1
}
commit=$(git rev-parse HEAD)
release_target="$repo_dir/target/linux-release-glibc217"
mkdir -p "$release_target" "$repo_dir/target/dist"

scripts/build-linux-bundles.sh "$snapshot_dir" "$release_target"

bundle_dir="$release_target/bundles"
[[ -f "$bundle_dir/KURV.clap" ]] || {
    echo "error: CLAP bundle was not produced" >&2
    exit 1
}
vst3_binary="$bundle_dir/KURV.vst3/Contents/x86_64-linux/KURV.so"
[[ -f "$vst3_binary" ]] || {
    echo "error: VST3 bundle was not produced" >&2
    exit 1
}
package_name="KURV-Linux-x86_64-glibc217-v$version"
package_root="$stage_dir/$package_name"
mkdir -p "$package_root"
cp "$bundle_dir/KURV.clap" "$package_root/KURV.clap"
cp -a "$bundle_dir/KURV.vst3" "$package_root/KURV.vst3"
printf 'KURV %s\ncommit=%s\ntarget=x86_64-unknown-linux-gnu\ntarget_cpu=baseline\nmaximum_glibc=2.17\n' \
    "$version" "$commit" >"$package_root/BUILD-MARKER.txt"
(
    cd "$package_root"
    find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum >SHA256SUMS
)

archive="$repo_dir/target/dist/$package_name.tar.gz"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    -C "$stage_dir" -cf - "$package_name" | gzip -n >"$archive"
sha256sum "$archive" >"$archive.sha256"
echo "Linux release: $archive"
cat "$archive.sha256"
