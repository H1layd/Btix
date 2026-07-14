#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${PREFIX:-$HOME/.local/bin}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Building release version..."
cargo build --release

echo "==> Installing into $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
install -m 755 target/release/btix "$INSTALL_DIR/btix"

echo "==> Configuring PATH ($INSTALL_DIR)"
if command -v fish >/dev/null 2>&1; then
    fish -c "fish_add_path -U '$INSTALL_DIR'" || true
    echo "    (fish) fish_user_paths updated"
fi
case "${SHELL:-}" in
    *zsh*) rc="$HOME/.zshrc" ;;
    *bash*) rc="$HOME/.bashrc" ;;
    *) rc="" ;;
esac
if [ -n "$rc" ] && ! grep -qs "$INSTALL_DIR" "$rc"; then
    printf '\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$rc"
    echo "    added to $rc"
fi

echo "==> Done."
echo "    Activate PATH in the CURRENT session with one command:"
echo "      fish:      fish_add_path $INSTALL_DIR"
echo "      bash/zsh:  export PATH=\"$INSTALL_DIR:\$PATH\""
echo "    Or just open a new terminal. Then run: btix"
