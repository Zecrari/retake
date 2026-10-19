#!/bin/sh
# Installs retake: https://github.com/Zecrari/retake
#
#   curl -fsSL https://zecrari.github.io/retake/install.sh | sh
#
# Environment variables:
#   RETAKE_VERSION      release tag to install, e.g. v0.1.0 (default: latest)
#   RETAKE_INSTALL_DIR  where to put the binary (default: $HOME/.local/bin)
set -eu

repo="Zecrari/retake"
version="${RETAKE_VERSION:-latest}"
install_dir="${RETAKE_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'retake-install: %s\n' "$1"; }
fail() { printf 'retake-install: error: %s\n' "$1" >&2; exit 1; }

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) fail "unsupported OS $(uname -s). On Windows, use install.ps1." ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) fail "unsupported CPU architecture $(uname -m)" ;;
esac
target="${arch}-${os}"
asset="retake-${target}.tar.gz"

if [ "$version" = "latest" ]; then
  base="https://github.com/${repo}/releases/latest/download"
else
  base="https://github.com/${repo}/releases/download/${version}"
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
else
  fail "curl or wget is required"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading ${asset} (${version})"
fetch "${base}/${asset}" "${tmp}/${asset}" || fail "download failed: ${base}/${asset}"
fetch "${base}/${asset}.sha256" "${tmp}/${asset}.sha256" || fail "checksum download failed"

expected="$(cut -d ' ' -f1 "${tmp}/${asset}.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmp}/${asset}" | cut -d ' ' -f1)"
else
  actual="$(shasum -a 256 "${tmp}/${asset}" | cut -d ' ' -f1)"
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for ${asset}"

tar -xzf "${tmp}/${asset}" -C "$tmp"
mkdir -p "$install_dir"
install -m 755 "${tmp}/retake-${target}/retake" "${install_dir}/retake" 2>/dev/null \
  || { cp "${tmp}/retake-${target}/retake" "${install_dir}/retake" && chmod 755 "${install_dir}/retake"; }

say "installed $("${install_dir}/retake" --version) to ${install_dir}/retake"
case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) say "add ${install_dir} to your PATH, e.g.: export PATH=\"${install_dir}:\$PATH\"" ;;
esac
