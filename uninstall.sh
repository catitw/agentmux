#!/bin/sh
# agentmux uninstaller: desktop launcher entry first (while the binary still
# exists), then cargo uninstall. --purge also removes the config directory.
set -eu

cd "$(dirname "$0")"

PURGE=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        *)
            echo "error: unknown argument: $arg (supported: --purge)" >&2
            exit 1
            ;;
    esac
done

BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
BIN="$BIN_DIR/agentmux"

echo "==> removing desktop launcher entry..."
if [ -x "$BIN" ]; then
    "$BIN" --uninstall-desktop-entry
else
    echo "    binary not found at $BIN — skipping launcher cleanup (any leftover desktop files are harmless)"
fi

echo "==> uninstalling agentmux (cargo uninstall)..."
if ! cargo uninstall agentmux 2>/dev/null; then
    echo "    agentmux was not installed via cargo — nothing to uninstall"
fi

if [ "$PURGE" = 1 ]; then
    echo "==> purging config (~/.config/agentmux)..."
    rm -rf "$HOME/.config/agentmux"
    echo "    removed ~/.config/agentmux"
else
    echo "note: config kept at ~/.config/agentmux (sessions.json etc.)"
    echo "      re-run with --purge to remove it"
fi

echo "done."
