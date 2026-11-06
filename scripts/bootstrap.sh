#!/bin/sh
set -eu
PROJECT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export RUSTUP_HOME="$PROJECT_ROOT/.tools/rustup"
export CARGO_HOME="$PROJECT_ROOT/.tools/cargo"
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    INSTALLER=$(mktemp -t rust-explorer-rustup)
    trap 'rm -f "$INSTALLER"' EXIT
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$INSTALLER"
    sh "$INSTALLER" -y --no-modify-path --profile minimal --default-toolchain none
fi
cd "$PROJECT_ROOT"
TOOLCHAIN_VERSION=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
"$CARGO_HOME/bin/rustup" toolchain install "$TOOLCHAIN_VERSION" --profile minimal --component rustfmt --component clippy
