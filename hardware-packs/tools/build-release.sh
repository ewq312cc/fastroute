#!/bin/sh
# Builds the release of the packs in this tree: verifies every pack and the set, checks index.yaml (the lock), and writes
# dist/hcp-packs/ (release.yaml, SHA256SUMS, one deterministic archive per pack). No network is used.
#
#   hardware-packs/tools/build-release.sh [--regenerate-index] [OUT_DIR]
#
# After you change a pack: `hcp-manager release seal <pack dir>` (digests), then --regenerate-index, and review the diff of index.yaml:
# it is what a software release ships.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${HCP_MANAGER_BIN:-}
if [ -z "$bin" ]; then
  root=$(cd "$here/.." && pwd)
  for c in "$root/target/release/hcp-manager" "$root/target/debug/hcp-manager"; do [ -x "$c" ] && bin=$c && break; done
fi
[ -n "$bin" ] || { echo "hcp-manager not found: cargo build --release -p hcp-manager, or set HCP_MANAGER_BIN" >&2; exit 6; }
regen=0
if [ "${1:-}" = "--regenerate-index" ]; then regen=1; shift; fi
out=${1:-"$here/../dist/hcp-packs"}
if [ "$regen" = 1 ]; then "$bin" release index --packs "$here"; else "$bin" release index --packs "$here" --check; fi
"$bin" release build --packs "$here" --out "$out" --force
"$bin" packs builtin --release "$out" verify
