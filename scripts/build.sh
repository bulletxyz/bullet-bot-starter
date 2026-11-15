#!/usr/bin/env bash
# Builds every strategy in strategies/ into a WASM component (wasm32-wasip2, release) and puts it
# in dist/ as <name>.wasm, with <name>.wasm.sha256 and dist/SHA256SUMS. The folder name is the
# strategy's name: what the bot shows and what the owner signs.
#
# Usage: scripts/build.sh [name ...]   (no names: every strategy)
set -euo pipefail
cd "$(dirname "$0")/.."

target=wasm32-wasip2
sysroot=$(rustc --print sysroot)
if [ ! -d "$sysroot/lib/rustlib/$target" ]; then
  echo "The $target target is missing. Install it with: rustup target add $target" >&2
  exit 1
fi

# Only crates with a cdylib are plugins; the rest are the bot's built-ins.
if [ $# -gt 0 ]; then
  names=("$@")
else
  names=()
  for d in strategies/*/; do grep -q cdylib "$d/Cargo.toml" && names+=("$(basename "$d")"); done
fi

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
field() {
  local v
  v=$(sed -n "s/^$1 = \"\(.*\)\"/\1/p" "$2" | head -1)
  [ -n "$v" ] || { echo "$2 has no $1 = \"...\" line" >&2; exit 1; }
  echo "$v"
}

packages=()
for name in "${names[@]}"; do
  manifest="strategies/$name/Cargo.toml"
  [ -f "$manifest" ] || { echo "no strategy named $name (strategies/$name)" >&2; exit 1; }
  grep -q cdylib "$manifest" || { echo "$name is a built-in, not a plugin" >&2; exit 1; }
  packages+=(-p "$(field name "$manifest")")
done

# No local paths inside the file, so the same source and toolchain give the same bytes in any
# folder. CARGO_ENCODED_RUSTFLAGS (flags split by 0x1f) keeps a path with spaces whole.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
sep=$(printf '\037')
flags="--remap-path-prefix=$PWD=/starter${sep}--remap-path-prefix=$cargo_home=/cargo"
if [ -n "${RUSTFLAGS:-}" ]; then flags="$(echo "$RUSTFLAGS" | tr ' ' "$sep")${sep}$flags"; fi
export CARGO_ENCODED_RUSTFLAGS="$flags"
unset RUSTFLAGS
cargo build --locked --release --target "$target" "${packages[@]}"

mkdir -p dist
# Written again once every plugin passed, so a refused build leaves none that is wrong.
rm -f dist/SHA256SUMS
printf '\n%-20s %-9s %s\n' name version sha256
for name in "${names[@]}"; do
  manifest="strategies/$name/Cargo.toml"
  crate=$(field name "$manifest" | tr - _)
  version=$(field version "$manifest")
  cp "${CARGO_TARGET_DIR:-target}/$target/release/$crate.wasm" "dist/$name.wasm"
  # The bot reads the plugin's name and version from its bb-strategy-meta section (`name\nversion`): they must be
  # the folder's name and the Cargo.toml version.
  if ! tr '\n' '|' < "dist/$name.wasm" | grep -a -F "bb-strategy-meta$name|$version" >/dev/null; then
    echo "dist/$name.wasm is not labelled $name $version:" \
      "Describe::NAME must be \"$name\" and VERSION its Cargo.toml's" >&2
    rm -f "dist/$name.wasm" "dist/$name.wasm.sha256"
    exit 1
  fi
  hash=$(sha "dist/$name.wasm")
  echo "$hash  $name.wasm" > "dist/$name.wasm.sha256"
  printf '%-20s %-9s %s\n' "$name" "$version" "$hash"
done
cat dist/*.wasm.sha256 > dist/SHA256SUMS
echo
echo "Install it on your bot: docs/INSTALL.md"
