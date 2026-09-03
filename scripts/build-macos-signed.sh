#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/target/macos-release}"
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
export MACOSX_DEPLOYMENT_TARGET

for name in \
  APPLE_APPLICATION_CERTIFICATE_P12_BASE64 \
  APPLE_INSTALLER_CERTIFICATE_P12_BASE64 \
  APPLE_CERTIFICATE_PASSWORD \
  APPLE_DEVELOPER_ID_APPLICATION \
  APPLE_DEVELOPER_ID_INSTALLER \
  APPLE_ID \
  APPLE_APP_SPECIFIC_PASSWORD \
  APPLE_TEAM_ID; do
  [[ -n "${!name:-}" ]] || { echo "Missing required environment variable: $name" >&2; exit 1; }
done

for command in cargo cargo-truce codesign ditto lipo pkgbuild pkgutil python3 security shasum spctl uuidgen xcrun; do
  command -v "$command" >/dev/null || { echo "Missing required command: $command" >&2; exit 1; }
done

version="$(python3 - "$ROOT_DIR/Cargo.toml" <<'PY'
import pathlib
import sys
import tomllib

print(tomllib.loads(pathlib.Path(sys.argv[1]).read_text())["package"]["version"])
PY
)"

build_bundle() {
  local target="$1"
  local target_dir="$ROOT_DIR/target/macos-$target"
  rm -rf -- "$target_dir"
  mkdir -p -- "$target_dir"
  (
    cd "$ROOT_DIR"
    CARGO_TARGET_DIR="$target_dir" cargo +1.97.1 truce build \
      --clap --vst3 \
      -p pure_va_dispersion_core \
      --target "$target" \
      --target-cpu baseline
  )
  for bundle in KURV.clap KURV.vst3; do
    [[ -d "$target_dir/bundles/$target/$bundle" ]] || {
      echo "Missing $target $bundle bundle" >&2
      exit 1
    }
    [[ -f "$target_dir/bundles/$target/$bundle/Contents/MacOS/KURV" ]] || {
      echo "Missing $target $bundle binary" >&2
      exit 1
    }
  done
}

build_bundle aarch64-apple-darwin
build_bundle x86_64-apple-darwin

work_dir="$(mktemp -d)"
keychain="$work_dir/signing.keychain-db"
keychain_password="$(uuidgen)"
cleanup() {
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

bundles="$work_dir/bundled"
arm_bundles="$ROOT_DIR/target/macos-aarch64-apple-darwin/bundles/aarch64-apple-darwin"
x86_bundles="$ROOT_DIR/target/macos-x86_64-apple-darwin/bundles/x86_64-apple-darwin"
mkdir -p -- "$bundles"
for bundle in KURV.clap KURV.vst3; do
  ditto "$arm_bundles/$bundle" "$bundles/$bundle"
  universal_binary="$bundles/$bundle/Contents/MacOS/KURV"
  lipo -create \
    "$arm_bundles/$bundle/Contents/MacOS/KURV" \
    "$x86_bundles/$bundle/Contents/MacOS/KURV" \
    -output "$universal_binary.tmp"
  mv -- "$universal_binary.tmp" "$universal_binary"
  lipo "$universal_binary" -verify_arch arm64 x86_64
done

mkdir -p -- "$OUTPUT_DIR"
APPLICATION_P12="$work_dir/application.p12" INSTALLER_P12="$work_dir/installer.p12" python3 - <<'PY'
import base64
import os
import pathlib

pathlib.Path(os.environ["APPLICATION_P12"]).write_bytes(
    base64.b64decode(os.environ["APPLE_APPLICATION_CERTIFICATE_P12_BASE64"])
)
pathlib.Path(os.environ["INSTALLER_P12"]).write_bytes(
    base64.b64decode(os.environ["APPLE_INSTALLER_CERTIFICATE_P12_BASE64"])
)
PY

security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$work_dir/application.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12
security import "$work_dir/installer.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12
security list-keychains -d user -s "$keychain"
security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain"

for bundle in KURV.clap KURV.vst3; do
  codesign --force --sign "$APPLE_DEVELOPER_ID_APPLICATION" \
    --keychain "$keychain" --options runtime --timestamp "$bundles/$bundle"
  codesign --verify --deep --strict --verbose=2 "$bundles/$bundle"
done

pkgroot="$work_dir/pkgroot"
mkdir -p \
  "$pkgroot/Library/Audio/Plug-Ins/CLAP" \
  "$pkgroot/Library/Audio/Plug-Ins/VST3"
ditto "$bundles/KURV.clap" "$pkgroot/Library/Audio/Plug-Ins/CLAP/KURV.clap"
ditto "$bundles/KURV.vst3" "$pkgroot/Library/Audio/Plug-Ins/VST3/KURV.vst3"

pkg="$OUTPUT_DIR/KURV-${version}-macos.pkg"
pkgbuild \
  --root "$pkgroot" \
  --identifier com.prototypelab.kurv.pkg \
  --version "$version" \
  --install-location / \
  --sign "$APPLE_DEVELOPER_ID_INSTALLER" \
  --keychain "$keychain" \
  "$pkg"

notary_log="$OUTPUT_DIR/KURV-${version}-notary.json"
xcrun notarytool submit "$pkg" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_APP_SPECIFIC_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait \
  --output-format json > "$notary_log"
xcrun stapler staple "$pkg"
xcrun stapler validate "$pkg"
pkgutil --check-signature "$pkg"
spctl --assess --type install --verbose=4 "$pkg"
shasum -a 256 "$pkg" > "$pkg.sha256"

echo "$pkg"
