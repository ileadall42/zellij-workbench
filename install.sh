#!/usr/bin/env bash
# Installs the zw binary by building it with cargo.
#
#   curl -fsSL https://raw.githubusercontent.com/LeON-Nie-code/zellij-workbench/main/install.sh | bash
#
# Override the install directory with ZELLIJ_WORKBENCH_INSTALL_DIR.
set -euo pipefail

REPO_URL="https://github.com/LeON-Nie-code/zellij-workbench"
INSTALL_DIR="${ZELLIJ_WORKBENCH_INSTALL_DIR:-$HOME/.local/bin}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to install zw (see https://rustup.rs)" >&2
  exit 1
fi

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

echo "Cloning $REPO_URL..."
git clone --depth 1 "$REPO_URL" "$workdir/zellij-workbench"

echo "Building zw (release)..."
(cd "$workdir/zellij-workbench" && cargo build --release --bin zw)

mkdir -p "$INSTALL_DIR"
install -m 755 "$workdir/zellij-workbench/target/release/zw" "$INSTALL_DIR/zw"

echo "Installed zw to $INSTALL_DIR/zw"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "note: $INSTALL_DIR is not on your PATH yet" ;;
esac
