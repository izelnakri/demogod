#!/usr/bin/env bash
# Prepares a release: make release VERSION=0.2.0
#
# Sets the version in Cargo.toml, Cargo.lock and npm/package.json, writes the CHANGELOG with
# git-cliff, commits and tags. Pushing the tag is what publishes: CI builds every binary, then
# releases them on GitHub, crates.io and npm.
set -euo pipefail

VERSION="${1:?usage: scripts/release.sh <version>, e.g. 0.2.0}"
VERSION="${VERSION#v}"
cd "$(dirname "$0")/.."

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not a version: $VERSION" >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo "commit or stash your changes first" >&2; exit 1; }
git rev-parse "v$VERSION" >/dev/null 2>&1 && { echo "v$VERSION is already tagged" >&2; exit 1; }
command -v git-cliff >/dev/null || { echo "git-cliff is needed: cargo install git-cliff (it is in the nix devShell)" >&2; exit 1; }

# Node, rather than sed, so it reads the same with GNU's and BSD's.
# shellcheck disable=SC2016 # the ${…} below is JavaScript's, not the shell's
node -e '
  const fs = require("fs");
  const version = process.argv[1];
  const cargo = fs.readFileSync("Cargo.toml", "utf8");
  fs.writeFileSync("Cargo.toml", cargo.replace(/^version = ".*"$/m, `version = "${version}"`));
  const pkg = JSON.parse(fs.readFileSync("npm/package.json", "utf8"));
  pkg.version = version;
  for (const name of Object.keys(pkg.optionalDependencies)) pkg.optionalDependencies[name] = version;
  fs.writeFileSync("npm/package.json", JSON.stringify(pkg, null, 2) + "\n");
' "$VERSION"
cargo update --workspace --quiet
git-cliff --tag "v$VERSION" --output CHANGELOG.md

git add Cargo.toml Cargo.lock npm/package.json CHANGELOG.md
git commit --quiet -m "chore(release): v$VERSION"
git tag -a "v$VERSION" -m "v$VERSION"

echo "Tagged v$VERSION. Publish it with:"
echo
echo "  git push origin main v$VERSION"
