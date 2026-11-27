#!/usr/bin/env bash
set -euo pipefail

# Builds an ad-hoc signed Haven.app:
#   scripts/bundle-macos.sh            # release build
#   PROFILE=debug scripts/bundle-macos.sh

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${PROFILE:-release}"
BUNDLE_DIR="${ROOT_DIR}/target/bundle/Haven.app"
MACOS_DIR="${BUNDLE_DIR}/Contents/MacOS"

if [[ "${PROFILE}" == "release" ]]; then
    cargo build --release -p haven --manifest-path "${ROOT_DIR}/Cargo.toml" "$@"
else
    cargo build -p haven --manifest-path "${ROOT_DIR}/Cargo.toml" "$@"
fi

rm -rf "${BUNDLE_DIR}"
mkdir -p "${MACOS_DIR}" "${BUNDLE_DIR}/Contents/Resources"
cp "${ROOT_DIR}/target/${PROFILE}/haven" "${MACOS_DIR}/haven"
cp "${ROOT_DIR}/assets/Info.plist.template" "${BUNDLE_DIR}/Contents/Info.plist"
cp "${ROOT_DIR}/assets/AppIcon.icns" "${BUNDLE_DIR}/Contents/Resources/AppIcon.icns"
chmod +x "${MACOS_DIR}/haven"

if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "${BUNDLE_DIR}" >/dev/null 2>&1 || true
fi

echo "Built ${BUNDLE_DIR}"
echo "  open \"${BUNDLE_DIR}\""
