#!/usr/bin/env bash
# Compila el plugin oliv4600-pack
set -e
source "$HOME/.cargo/env"
cd "$(dirname "$0")/plugins/oliv4600-pack"
echo "→ trunk build (plugin)…"
trunk build
echo "✓ Plugin compilado → plugins/oliv4600-pack/app/"
