#!/bin/sh
# Decodes every log under ~/DeSCENT (or $1) with lines.js and with app/src/lines.rs
# and compares the two line by line. Lab captures and test/data go in too when found,
# since only they have CSV-receiver and console hex-dump lines.
# sh tools/golden.sh [data folder] [work folder]
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

nice -n 19 node "$here/tools/golden_decode.js" "$list" "$work/js.jsonl"
DG_GOLDEN_LIST=$list DG_GOLDEN_OUT=$work/rust.jsonl \
  nice -n 19 cargo test -j 2 \
  --manifest-path "$here/app/Cargo.toml" lines::tests::golden -- --exact
nice -n 19 node "$here/tools/golden_compare.js" "$work/js.jsonl" "$work/rust.jsonl"
