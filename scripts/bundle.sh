#!/bin/sh
set -eu
PROJECT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD_PYTHON=${RUST_EXPLORER_PYTHON:-python3}
if [ -z "${RUST_EXPLORER_PYTHON:-}" ] && [ -x "$HOME/.local/bin/python3.11" ]; then
    BUILD_PYTHON="$HOME/.local/bin/python3.11"
fi
exec "$BUILD_PYTHON" "$PROJECT_ROOT/scripts/build-bundle.py" "$@"
