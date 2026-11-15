#!/usr/bin/env bash
# Makes strategies/<name>/ from templates/strategy: a strategy that builds, tests and installs.
#
# Usage: scripts/new-strategy.sh <name>   (lowercase letters, digits, - and _; at most 48)
set -euo pipefail
cd "$(dirname "$0")/.."
name=${1:-}
if ! [[ $name =~ ^[a-z0-9][a-z0-9_-]{0,47}$ ]]; then
  echo "A name is lowercase letters, digits, - and _ (at most 48), starting with a letter or digit." >&2
  exit 1
fi
dir="strategies/$name"
[ -e "$dir" ] && { echo "$dir already exists" >&2; exit 1; }
# CamelCase type name: my-cool_bot -> MyCoolBot.
type=$(echo "$name" | awk -F'[-_]' '{for (i=1;i<=NF;i++) printf "%s", toupper(substr($i,1,1)) substr($i,2)}')
case $type in [0-9]*) type="S$type" ;; esac
mkdir -p "$dir/src"
for f in Cargo.toml src/lib.rs src/lib_tests.rs; do
  sed -e "s/__NAME__/$name/g" -e "s/__TYPE__/$type/g" "templates/strategy/$f" > "$dir/$f"
done
# Add it to Cargo.lock (only the new crate: every dependency keeps its pinned version).
cargo update --workspace --offline --quiet 2>/dev/null || cargo update --workspace --quiet
# The template is formatted for its own name: format the new one for yours.
cargo fmt -p "bb-strategy-$name" 2>/dev/null || true
echo "Made $dir. Next: edit $dir/src/lib.rs, then: just test && just build $name"
