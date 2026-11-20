#!/usr/bin/env bash
# Build and install Statehound from source: the everystate engine (upstream at a pinned commit plus this
# repository's patch) and the statehound binary. Pure Rust: no Java is needed.
#
#   scripts/install.sh [--prefix DIR] [--claude] [--codex] [--pipeline CHECKOUT]
#
#   --prefix DIR         install the binaries into DIR (default: ~/.local/bin)
#   --claude             install the statehound skill for Claude Code (~/.claude/skills/statehound)
#   --codex              install the statehound skill for Codex (~/.codex/skills/statehound)
#   --pipeline CHECKOUT  also patch a checkout of the TLA+ bug-finding pipeline
#                        (github.com/specular-org/Specular at the pinned commit) to use Statehound
set -euo pipefail

EVERYSTATE_URL="https://github.com/mattzcarey/everystate"
EVERYSTATE_COMMIT="28fd7c5f9069a021b7a9702007e2f2b054a789fb"   # everystate 0.4.1
PIPELINE_COMMIT="6079cd135122cd0d9d86b3fca5aa4002ae435844"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="$HOME/.local/bin"
CLAUDE=false
CODEX=false
PIPELINE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --claude) CLAUDE=true; shift ;;
        --codex) CODEX=true; shift ;;
        --pipeline) PIPELINE="$2"; shift 2 ;;
        -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

command -v cargo >/dev/null || { echo "cargo not found: install Rust from https://rustup.rs" >&2; exit 1; }
command -v git >/dev/null || { echo "git not found" >&2; exit 1; }

BUILD="${STATEHOUND_BUILD_DIR:-$REPO/target/build}"
mkdir -p "$BUILD" "$PREFIX"

echo "==> everystate $EVERYSTATE_COMMIT + statehound patch"
ES="$BUILD/everystate"
if [ ! -d "$ES/.git" ]; then
    git clone --quiet "$EVERYSTATE_URL" "$ES"
fi
git -C "$ES" fetch --quiet origin
git -C "$ES" checkout --quiet --force "$EVERYSTATE_COMMIT"
git -C "$ES" clean --quiet -fdx -e target
for p in "$REPO"/patches/everystate/*.patch; do
    git -C "$ES" apply --whitespace=nowarn "$p"
done
(cd "$ES" && cargo build --release --quiet)
install -m 0755 "$ES/target/release/everystate" "$PREFIX/everystate"

echo "==> statehound"
(cd "$REPO" && cargo build --release --quiet)
for bin in statehound tla-cfa; do
    if [ -x "$REPO/target/release/$bin" ]; then
        install -m 0755 "$REPO/target/release/$bin" "$PREFIX/$bin"
    fi
done

if $CLAUDE; then
    mkdir -p "$HOME/.claude/skills"
    rm -rf "$HOME/.claude/skills/statehound"
    cp -R "$REPO/skills/statehound" "$HOME/.claude/skills/statehound"
    echo "==> skill installed for Claude Code: ~/.claude/skills/statehound"
fi
if $CODEX; then
    mkdir -p "$HOME/.codex/skills"
    rm -rf "$HOME/.codex/skills/statehound"
    cp -R "$REPO/skills/statehound" "$HOME/.codex/skills/statehound"
    echo "==> skill installed for Codex: ~/.codex/skills/statehound"
fi

if [ -n "$PIPELINE" ]; then
    echo "==> patching pipeline checkout $PIPELINE"
    head="$(git -C "$PIPELINE" rev-parse HEAD)"
    if [ "$head" != "$PIPELINE_COMMIT" ]; then
        echo "warning: $PIPELINE is at $head; the patches were made against $PIPELINE_COMMIT" >&2
    fi
    for p in "$REPO"/patches/pipeline/*.patch; do
        git -C "$PIPELINE" apply --whitespace=nowarn "$p"
    done
    mkdir -p "$PIPELINE/bin"
    install -m 0755 "$PREFIX/everystate" "$PIPELINE/bin/everystate"
    install -m 0755 "$PREFIX/statehound" "$PIPELINE/bin/statehound"
    [ -x "$PREFIX/tla-cfa" ] && install -m 0755 "$PREFIX/tla-cfa" "$PIPELINE/bin/tla-cfa"
fi

echo
"$PREFIX/everystate" -version
echo "statehound installed in $PREFIX"
case ":$PATH:" in *":$PREFIX:"*) ;; *) echo "note: add $PREFIX to your PATH" ;; esac
