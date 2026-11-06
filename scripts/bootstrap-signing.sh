#!/bin/sh
set -eu
SIGNING_PROJECT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SIGNING_PYTHON=${RUST_EXPLORER_PYTHON:-python3}
if [ -z "${RUST_EXPLORER_PYTHON:-}" ] && [ -x "$HOME/.local/bin/python3.11" ]; then SIGNING_PYTHON="$HOME/.local/bin/python3.11"; fi
if [ -x "$SIGNING_PROJECT/.tools/signing/bin/rcodesign" ] && cmp -s "$SIGNING_PROJECT/scripts/signing-tool.lock" "$SIGNING_PROJECT/.tools/signing/installed.lock" && "$SIGNING_PYTHON" "$SIGNING_PROJECT/scripts/verify-signing-tool.py"; then
    exit 0
fi
sh "$SIGNING_PROJECT/scripts/cargo.sh" info apple-codesign@0.29.0 > /dev/null
"$SIGNING_PYTHON" "$SIGNING_PROJECT/scripts/prepare-signing-tool.py"
sh "$SIGNING_PROJECT/scripts/cargo.sh" install --path "$SIGNING_PROJECT/.tools/signing-source" --locked --no-default-features --root "$SIGNING_PROJECT/.tools/signing" --force
cp "$SIGNING_PROJECT/scripts/signing-tool.lock" "$SIGNING_PROJECT/.tools/signing/installed.lock"

"$SIGNING_PYTHON" "$SIGNING_PROJECT/scripts/verify-signing-tool.py" --record-after-build
