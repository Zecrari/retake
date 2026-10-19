#!/usr/bin/env bash
# Installs retake on a GitHub Actions runner. Used by action.yml.
set -euo pipefail

repo="Zecrari/retake"
version="${RETAKE_VERSION:-latest}"

case "${RUNNER_OS}-${RUNNER_ARCH}" in
  Linux-X64) target="x86_64-unknown-linux-musl" ;;
  Linux-ARM64) target="aarch64-unknown-linux-musl" ;;
  macOS-X64) target="x86_64-apple-darwin" ;;
  macOS-ARM64) target="aarch64-apple-darwin" ;;
  Windows-X64) target="x86_64-pc-windows-msvc" ;;
  *) echo "::error::retake has no prebuilt binary for ${RUNNER_OS}-${RUNNER_ARCH}"; exit 1 ;;
esac

if [ "$version" = "latest" ]; then
  version=$(gh release view --repo "$repo" --json tagName --jq .tagName)
fi

ext="tar.gz"
[ "$RUNNER_OS" = "Windows" ] && ext="zip"
asset="retake-${target}.${ext}"

workdir="${RUNNER_TEMP}/retake-${version}"
mkdir -p "$workdir"
gh release download "$version" --repo "$repo" --pattern "$asset" --pattern "${asset}.sha256" --dir "$workdir" --clobber

cd "$workdir"
expected=$(cut -d ' ' -f1 "${asset}.sha256")
if command -v sha256sum >/dev/null; then
  actual=$(sha256sum "$asset" | cut -d ' ' -f1)
else
  actual=$(shasum -a 256 "$asset" | cut -d ' ' -f1)
fi
if [ "$expected" != "$actual" ]; then
  echo "::error::Checksum mismatch for $asset"
  exit 1
fi

if [ "$ext" = "zip" ]; then
  unzip -oq "$asset"
else
  tar -xzf "$asset"
fi

echo "${workdir}/retake-${target}" >> "$GITHUB_PATH"
echo "version=${version}" >> "$GITHUB_OUTPUT"
echo "Installed retake ${version} (${target})"
