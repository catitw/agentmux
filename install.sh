#!/bin/sh
# agentmux installer: cargo install + desktop launcher entry, optionally
# with hook integration (--with-hooks).
set -eu

cd "$(dirname "$0")"

WITH_HOOKS=0
for arg in "$@"; do
    case "$arg" in
        --with-hooks) WITH_HOOKS=1 ;;
        *)
            echo "error: unknown argument: $arg (supported: --with-hooks)" >&2
            exit 1
            ;;
    esac
done

echo "==> checking cargo..."
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found." >&2
    echo "       Install the Rust toolchain first: https://rustup.rs" >&2
    exit 1
fi

echo "==> building and installing agentmux (cargo install --path . --locked)..."
cargo install --path . --locked

BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
BIN="$BIN_DIR/agentmux"
if [ ! -x "$BIN" ]; then
    echo "error: installed binary not found at $BIN" >&2
    exit 1
fi

echo "==> installing desktop launcher entry..."
"$BIN" --install-desktop-entry

if [ "$WITH_HOOKS" = 1 ]; then
    echo "==> installing hooks (claude/omp state reporting)..."
    "$BIN" --install-hooks
fi

echo
echo "done."
echo "  binary:   $BIN"
echo "  launcher: installed — find agentmux in your application menu"
echo "  start:    via the launcher, or run: agentmux"
if [ "$WITH_HOOKS" = 0 ]; then
    echo "  hooks:    not installed — re-run with --with-hooks to enable agent state reporting"
fi
