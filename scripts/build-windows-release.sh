#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_dir"

if [[ -n $(git status --porcelain --untracked-files=all) ]]; then
    echo "error: commit or stash every change before producing a release" >&2
    exit 1
fi

for command_name in 7z cargo git tar gzip sha256sum x86_64-w64-mingw32-objdump; do
    command -v "$command_name" >/dev/null || {
        echo "error: missing required command: $command_name" >&2
        exit 1
    }
done

for runtime in libstdc++-6.dll libgcc_s_seh-1.dll libwinpthread-1.dll; do
    [[ -f "/usr/x86_64-w64-mingw32/bin/$runtime" ]] || {
        echo "error: missing MinGW runtime: $runtime" >&2
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
mkdir -p "$repo_dir/target/dist"

for cpu_tier in x86-64; do
    release_target="$repo_dir/target/windows-release-$cpu_tier"
    (
        cd "$snapshot_dir"
        env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
            -u CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS \
            CARGO_TARGET_DIR="$release_target" cargo metadata --locked --no-deps --format-version 1 >/dev/null
        env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
            -u CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS \
            CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS='-C target-feature=-sse3' \
            CARGO_TARGET_DIR="$release_target" cargo truce build \
            --clap \
            --vst3 \
            -p pure_va_dispersion_core \
            --target x86_64-pc-windows-gnu \
            --target-cpu "$cpu_tier"
    )

    bundle_dir="$release_target/bundles/x86_64-pc-windows-gnu"
    [[ -f "$bundle_dir/KURV.clap" ]] || {
        echo "error: $cpu_tier CLAP bundle was not produced" >&2
        exit 1
    }
    vst3_binary="$bundle_dir/KURV.vst3/Contents/x86_64-win/KURV.vst3"
    [[ -f "$vst3_binary" ]] || {
        echo "error: $cpu_tier VST3 bundle was not produced" >&2
        exit 1
    }

    package_name="KURV-Windows-x86_64-universal-v$version"
    package_root="$stage_dir/$package_name"
    mkdir -p "$package_root"
    cp "$bundle_dir/KURV.clap" "$package_root/KURV.clap"
    cp -a "$bundle_dir/KURV.vst3" "$package_root/KURV.vst3"

    vst3_runtime_dir="$package_root/KURV.vst3/Contents/x86_64-win"
    cp /usr/x86_64-w64-mingw32/bin/libstdc++-6.dll "$vst3_runtime_dir/"
    cp /usr/x86_64-w64-mingw32/bin/libgcc_s_seh-1.dll "$vst3_runtime_dir/"
    cp /usr/x86_64-w64-mingw32/bin/libwinpthread-1.dll "$vst3_runtime_dir/"

    while IFS= read -r -d '' binary; do
        binary_dir=$(dirname "$binary")
        while IFS= read -r dependency; do
            dependency_lower=${dependency,,}
            case "$dependency_lower" in
                api-ms-*.dll|ext-ms-*.dll|kernel32.dll|ntdll.dll|userenv.dll|ws2_32.dll|avrt.dll|setupapi.dll|user32.dll|gdi32.dll|ole32.dll|opengl32.dll|combase.dll|rpcrt4.dll|oleaut32.dll|shell32.dll|winmm.dll|bcryptprimitives.dll)
                    continue
                    ;;
            esac
            [[ -f "$binary_dir/$dependency" ]] || {
                echo "error: unresolved Windows dependency $dependency required by $binary" >&2
                exit 1
            }
        done < <(x86_64-w64-mingw32-objdump -p "$binary" | sed -n 's/.*DLL Name: //p' | sort -fu)
    done < <(find "$package_root" -type f \( -name '*.clap' -o -name '*.vst3' -o -name '*.dll' \) -print0)

    printf 'KURV %s\ncommit=%s\ntarget=x86_64-pc-windows-gnu\ntarget_cpu=%s\n' \
        "$version" "$commit" "$cpu_tier" >"$package_root/BUILD-MARKER.txt"
    (
        cd "$package_root"
        find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum >SHA256SUMS
    )

    archive="$repo_dir/target/dist/$package_name.tar.gz"
    tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
        -C "$stage_dir" -cf - "$package_name" | gzip -n >"$archive"
    sha256sum "$archive" >"$archive.sha256"

    zip_archive="$repo_dir/target/dist/$package_name.zip"
    (
        cd "$stage_dir"
        7z a -tzip -mx=9 -mtc=off -mtm=off -mta=off "$zip_archive" "$package_name" >/dev/null
    )
    sha256sum "$zip_archive" >"$zip_archive.sha256"

    echo "Windows release: $archive"
    cat "$archive.sha256"
    echo "Windows release: $zip_archive"
    cat "$zip_archive.sha256"
done
