#!/bin/sh
# Replays every log under ~/DeSCENT (or $1) through fleet.js and through
# app/src/fleet.rs, as Open log replays one, and compares rows, receivers, the summary
# and every run's history. Same files as golden.sh.
# sh tools/golden_fleet.sh [data folder] [work folder]
set -e
here=$(cd "$(dirname "$0")/.." && pwd)
data=${1:-$HOME/DeSCENT}
work=${2:-$(mktemp -d)}
# The JS the port is held to, as it was in 1.3.0 (da42787) before 1.4 took it out of web/
mkdir -p "$work/ref"
git -C "$here/.." archive da42787 ground/web/js | tar -x --strip-components=1 -C "$work/ref"
export DG_REF_JS=$work/ref/web/js
list=$work/files.txt

{
  find "$data" -type f \( -name '*.log' -o -path '*/pulls/*/dump.txt' -o -path '*/clears/*/dump.txt' \)
  find "$here/test/data" -type f -name '*.txt'
  [ -d "${DESCENT_LAB_LOGS_RAW:-/media/Data/Research/SSDS/logs}" ] &&
    find "${DESCENT_LAB_LOGS_RAW:-/media/Data/Research/SSDS/logs}" -type f -name '*.log'
} | sort > "$list"
echo "$(wc -l < "$list") files, list in $list"

nice -n 19 node "$here/tools/golden_fleet.js" dump "$list" "$work/fleet_js.jsonl"
DG_GOLDEN_LIST=$list DG_GOLDEN_FLEET_OUT=$work/fleet_rust.jsonl \
  nice -n 19 cargo test -j 2 \
  --manifest-path "$here/app/Cargo.toml" fleet::tests::golden -- --exact
nice -n 19 node "$here/tools/golden_fleet.js" compare "$work/fleet_js.jsonl" "$work/fleet_rust.jsonl"
