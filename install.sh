#!/bin/sh
# Installs the latest demogod release:
#
#   curl -fsSL https://raw.githubusercontent.com/izelnakri/demogod/main/install.sh | sh
#
# DEMOGOD_VERSION=v0.2.0 picks a release, DEMOGOD_INSTALL=/usr/local/bin a directory.
set -eu

repository="izelnakri/demogod"
directory="${DEMOGOD_INSTALL:-$HOME/.local/bin}"
version="${DEMOGOD_VERSION:-latest}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64 | Linux-amd64) target="x86_64-unknown-linux-musl" ;;
  Linux-aarch64 | Linux-arm64) target="aarch64-unknown-linux-musl" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  *) echo "demogod has no prebuilt binary for $(uname -s) $(uname -m); try: cargo install demogod" >&2; exit 1 ;;
esac

if [ "$version" = latest ]; then
  base="https://github.com/$repository/releases/latest/download"
else
  base="https://github.com/$repository/releases/download/$version"
fi
archive="demogod-$target.tar.gz"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

echo "Downloading $archive ($version)"
curl -fsSL "$base/$archive" -o "$scratch/$archive"
if curl -fsSL "$base/checksums.txt" -o "$scratch/checksums.txt" 2>/dev/null; then
  expected="$(grep " $archive\$" "$scratch/checksums.txt" | cut -d' ' -f1 || true)"
  [ -n "$expected" ] || { echo "checksums.txt has no entry for $archive" >&2; exit 1; }
  actual="$( (sha256sum "$scratch/$archive" 2>/dev/null || shasum -a 256 "$scratch/$archive") | cut -d' ' -f1)"
  [ "$expected" = "$actual" ] || { echo "checksum mismatch for $archive" >&2; exit 1; }
fi

tar -xzf "$scratch/$archive" -C "$scratch"
mkdir -p "$directory"
install -m 755 "$scratch/demogod-$target/demogod" "$directory/demogod"
echo "Installed $("$directory/demogod" --version) to $directory/demogod"
case ":$PATH:" in
  *":$directory:"*) ;;
  *) echo "Add $directory to your PATH to run it as demogod." ;;
esac
