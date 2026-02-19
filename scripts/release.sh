#!/bin/bash
set -euo pipefail

# ShellDeck release script
# Usage: ./scripts/release.sh [patch|minor|major]
# Default: patch

cd "$(git rev-parse --show-toplevel)"

BUMP_TYPE="${1:-patch}"
CARGO_TOML="Cargo.toml"

# ─── Read current version ─────────────────────────────────────────────────────
CURRENT_VERSION=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/')
if [[ -z "$CURRENT_VERSION" ]]; then
    echo "error: Could not read version from $CARGO_TOML" >&2
    exit 1
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

# ─── Compute new version ──────────────────────────────────────────────────────
case "$BUMP_TYPE" in
    patch) PATCH=$((PATCH + 1)) ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    *)
        echo "error: Invalid bump type '$BUMP_TYPE'. Use: patch, minor, or major" >&2
        exit 1
        ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEW_VERSION}"

echo "==> Current version: $CURRENT_VERSION"
echo "==> New version:     $NEW_VERSION ($BUMP_TYPE bump)"
echo ""

# ─── Safety checks ────────────────────────────────────────────────────────────

# Check for uncommitted changes (excluding Cargo.toml/Cargo.lock which we'll modify)
if ! git diff --quiet -- ':!Cargo.toml' ':!Cargo.lock'; then
    echo "error: You have uncommitted changes. Commit or stash them first." >&2
    git diff --stat -- ':!Cargo.toml' ':!Cargo.lock'
    exit 1
fi

if ! git diff --cached --quiet; then
    echo "error: You have staged changes. Commit or unstage them first." >&2
    git diff --cached --stat
    exit 1
fi

# Check local tag doesn't exist
if git tag -l "$TAG" | grep -q "$TAG"; then
    echo "error: Local tag $TAG already exists." >&2
    echo "       Delete it with: git tag -d $TAG" >&2
    exit 1
fi

# Check remote tag doesn't exist
git fetch --tags --quiet 2>/dev/null || true
if git ls-remote --tags origin "refs/tags/$TAG" 2>/dev/null | grep -q "$TAG"; then
    echo "error: Remote tag $TAG already exists on origin." >&2
    echo "       This version has already been released." >&2
    exit 1
fi

# Check we're on main branch
BRANCH=$(git branch --show-current)
if [[ "$BRANCH" != "main" ]]; then
    echo "warning: You're on branch '$BRANCH', not 'main'."
    read -rp "Continue anyway? [y/N] " CONFIRM
    [[ "$CONFIRM" =~ ^[Yy]$ ]] || exit 1
fi

# Check we're up to date with remote
git fetch --quiet
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/main 2>/dev/null || echo "")
if [[ -n "$REMOTE" && "$LOCAL" != "$REMOTE" ]]; then
    BASE=$(git merge-base HEAD origin/main 2>/dev/null || echo "")
    if [[ "$BASE" != "$LOCAL" ]]; then
        echo "error: Your branch is behind origin/main. Run 'git pull --rebase' first." >&2
        exit 1
    fi
fi

# ─── Update version ──────────────────────────────────────────────────────────
echo "==> Updating version in $CARGO_TOML..."
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"

# ─── Build check ──────────────────────────────────────────────────────────────
echo "==> Running cargo check..."
if [[ "$(uname -s)" == "Linux" ]]; then
    PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/x86_64-linux-gnu/pkgconfig}" cargo check --quiet 2>&1
else
    cargo check --quiet 2>&1
fi

# ─── Prompt for commit message ────────────────────────────────────────────────
echo ""
echo "==> Recent commits since last tag:"
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if [[ -n "$LAST_TAG" ]]; then
    git log --oneline "$LAST_TAG"..HEAD | head -10
else
    git log --oneline -10
fi

echo ""
DEFAULT_MSG="$TAG: "
read -rp "Commit message [$DEFAULT_MSG]: " USER_MSG
COMMIT_MSG="${USER_MSG:-$DEFAULT_MSG}"

# Ensure message starts with version tag
if [[ ! "$COMMIT_MSG" =~ ^v[0-9] ]]; then
    COMMIT_MSG="$TAG: $COMMIT_MSG"
fi

# ─── Commit, tag, push ───────────────────────────────────────────────────────
echo ""
echo "==> Committing and tagging..."
git add Cargo.toml Cargo.lock
git commit -m "$COMMIT_MSG"
git tag "$TAG"

echo "==> Pushing to origin..."
git push
git push origin "$TAG"

echo ""
echo "=== Release $TAG pushed ==="
echo "  GitHub Actions will now build release binaries."
echo "  Monitor: https://github.com/$(git remote get-url origin | sed 's/.*github.com[:/]\(.*\)\.git/\1/')/actions"
