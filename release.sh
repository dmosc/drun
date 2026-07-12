#!/usr/bin/env bash
# release.sh — bump versions, tag, and push to trigger CI release builds.
#
# Usage:
#   ./release.sh <new-tag>
#   ./release.sh v0.2.0
#
# What this does:
#   1. Detects the current version from crates/drun-core/Cargo.toml
#   2. Bumps [package] version and drun-core dependency pins in all three
#      Cargo.toml files (drun-core, drun-py, drun-mcp)
#   3. Commits the version bump
#   4. Creates a signed git tag on that commit
#   5. Pushes the commit + tag to origin
#
# The tag push triggers release.yml, which:
#   • builds drun-mcp binaries (macOS arm64/x86_64, Linux x86_64)
#   • builds and publishes the drun-sandbox Python wheels to PyPI
#     (build-wheels + publish-pypi jobs)
#
# Prerequisites:
#   • Working tree must be clean (no uncommitted or staged changes)
#   • You must be on the branch you want tagged (typically main)
#   • You must have push access to origin

set -euo pipefail

# ── argument validation ────────────────────────────────────────────────────────

NEW_TAG="${1:-}"
if [[ -z "$NEW_TAG" ]]; then
    echo "Usage: $0 <new-tag>   (e.g. $0 v0.2.0)"
    exit 1
fi

if ! [[ "$NEW_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: tag must match vMAJOR.MINOR.PATCH (got: $NEW_TAG)"
    exit 1
fi

NEW_VERSION="${NEW_TAG#v}"

# ── pre-flight checks ──────────────────────────────────────────────────────────

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Error: working tree is not clean — commit or stash changes first."
    exit 1
fi

if git rev-parse "$NEW_TAG" &>/dev/null; then
    echo "Error: tag $NEW_TAG already exists."
    exit 1
fi

# ── detect current version ─────────────────────────────────────────────────────

CURRENT_VERSION=$(grep -m1 '^version = ' crates/drun-core/Cargo.toml | sed 's/version = "//;s/"//')

echo "Current version : $CURRENT_VERSION"
echo "New version     : $NEW_VERSION"
echo "New tag         : $NEW_TAG"
echo ""

if [[ "$CURRENT_VERSION" == "$NEW_VERSION" ]]; then
    echo "Error: new version is the same as the current version."
    exit 1
fi

# ── in-place sed (macOS vs Linux) ─────────────────────────────────────────────

sedi() {
    if [[ "$(uname)" == "Darwin" ]]; then
        sed -i '' "$@"
    else
        sed -i "$@"
    fi
}

# ── bump versions in all Cargo.toml files ─────────────────────────────────────

CARGO_FILES=(
    "crates/drun-core/Cargo.toml"
    "crates/drun-py/Cargo.toml"
    "crates/drun-mcp/Cargo.toml"
)

echo "Bumping versions..."
for f in "${CARGO_FILES[@]}"; do
    # [package] version line
    sedi "s/^version = \"${CURRENT_VERSION}\"$/version = \"${NEW_VERSION}\"/" "$f"
    # drun-core dependency pin (present in drun-py and drun-mcp)
    sedi "s/drun-core = { version = \"${CURRENT_VERSION}\"/drun-core = { version = \"${NEW_VERSION}\"/" "$f"
    echo "  $f"
done

echo ""

# Regenerate Cargo.lock so the version bump is reflected before committing.
echo "Updating Cargo.lock..."
cargo metadata --format-version 1 -q > /dev/null

echo ""

# ── commit, tag, push ──────────────────────────────────────────────────────────

echo "Committing version bump..."
git add "${CARGO_FILES[@]}" Cargo.lock
git commit -m "chore: bump version to ${NEW_TAG}"

echo "Creating tag ${NEW_TAG}..."
git tag "${NEW_TAG}"

echo "Pushing commit and tag..."
git push origin HEAD
git push origin "${NEW_TAG}"

echo ""
echo "Released ${NEW_TAG}. Workflows now running:"
echo "  release.yml → drun-mcp binaries (macOS arm64/x86_64, Linux x86_64)"
echo "  pypi.yml    → Python wheels published to PyPI"
