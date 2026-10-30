#!/usr/bin/env bash
# Launch the complete desktop build, including its matching Chromium helper.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export CEF_PATH="${CEF_PATH:-${XDG_CACHE_HOME:-$HOME/.cache}/shelldeck/cef}"
if [[ "$(uname -s)" == Linux ]]; then
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/x86_64-linux-gnu/pkgconfig}"
fi
# Separate invocations keep the runtime feature out of the main GPUI binary.
cargo build --locked -p shelldeck-browser --features runtime
python3 scripts/bundle-browser-runtime.py --profile debug
cargo build --locked -p shelldeck
exec "${CARGO_TARGET_DIR:-target}/debug/shelldeck" "$@"
