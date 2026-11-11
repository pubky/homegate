#!/bin/bash

# Sets the homegate version in Cargo.toml, Cargo.lock and openapi.yaml.
# The release workflow fails if these do not match the tag.

set -euo pipefail

NEW_VERSION=${1:-}
if [ -z "$NEW_VERSION" ]; then
  echo "Error: New version not specified."
  echo "Usage: $0 <new_version>"
  exit 1
fi

SEMVER_REGEX="^([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9A-Za-z.-]+))?$"
if [[ ! "$NEW_VERSION" =~ $SEMVER_REGEX ]]; then
  echo "Error: Version '$NEW_VERSION' is not in semver format (e.g., 1.2.3, 1.0.0-rc.1)."
  exit 1
fi

cd "$(dirname "$0")/.."

# `-i.bak` is the only in-place form that both GNU and BSD sed accept.
# The range stops at the first `version` line, which is the package's own.
sed -i.bak "1,/^version = /s/^version = .*/version = \"$NEW_VERSION\"/" Cargo.toml
sed -i.bak "s/^  version: .*/  version: $NEW_VERSION/" openapi.yaml
rm Cargo.toml.bak openapi.yaml.bak

# Writes the new version into Cargo.lock without touching any dependency.
cargo update --workspace

echo "Version set to $NEW_VERSION"
