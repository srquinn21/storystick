#!/usr/bin/env bash
# One-time dev setup: run this once after `git clone`.
#
# It builds storystick and symlinks the binary onto your PATH, so from
# then on a plain `cargo build --manifest-path cli/Cargo.toml` (run from
# anywhere) keeps the `storystick` command current -- no `cargo install`,
# no `--force` reinstall dance. It also seeds a starter stock catalog at
# ~/.config/storystick/stock.yaml if you don't already have one.
#
# Safe to re-run: an existing stock catalog is left untouched, and the
# symlink is just repointed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
config_dir="$HOME/.config/storystick"
stock_path="$config_dir/stock.yaml"

echo "==> building storystick"
cargo build --manifest-path "$repo_root/cli/Cargo.toml"

mkdir -p "$cargo_bin_dir"
ln -sf "$repo_root/target/debug/storystick" "$cargo_bin_dir/storystick"
echo "==> linked $cargo_bin_dir/storystick -> target/debug/storystick"

if [ -e "$stock_path" ]; then
    echo "==> stock catalog already exists at $stock_path, leaving it alone"
else
    mkdir -p "$config_dir"
    cp "$repo_root/scripts/stock.example.yaml" "$stock_path"
    echo "==> seeded a starter stock catalog at $stock_path -- edit it to match your shop"
fi

case ":$PATH:" in
    *":$cargo_bin_dir:"*) ;;
    *) echo "warning: $cargo_bin_dir is not on your PATH -- add it in your shell profile (rustup normally does this for you)." ;;
esac

echo
echo "done. 'storystick' is on your PATH. After editing code, just run:"
echo "  cargo build --manifest-path \"$repo_root/cli/Cargo.toml\""
echo "and the installed 'storystick' command picks up the change automatically."
