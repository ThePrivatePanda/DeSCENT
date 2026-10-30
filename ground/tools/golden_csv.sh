#!/bin/sh
# Writes every log under ~/DeSCENT (or $1) as CSV with exportcsv.js and with
# app/src/csv.rs and diffs the two file by file. Same files as golden.sh.
# sh tools/golden_csv.sh [data folder] [work folder]
set -e
here=$(cd "$(dirname "$0")/.." && pwd)
data=${1:-$HOME/DeSCENT}
work=${2:-$(mktemp -d)}
list=$work/files.txt

{
  find "$data" -type f \( -name '*.log' -o -path '*/pulls/*/dump.txt' -o -path '*/clears/*/dump.txt' \)
  find "$here/test/data" -type f -name '*.txt'
  [ -d "${DESCENT_LAB_LOGS_RAW:-/media/Data/Research/SSDS/logs}" ] &&
    find "${DESCENT_LAB_LOGS_RAW:-/media/Data/Research/SSDS/logs}" -type f -name '*.log'
} | sort > "$list"
echo "$(wc -l < "$list") files, list in $list"

nice -n 19 node "$here/tools/golden_csv.js" "$list" "$work"
DG_GOLDEN_LIST=$list DG_GOLDEN_CSV_OUT=$work \
  nice -n 19 cargo test -j 2 \
  --manifest-path "$here/app/Cargo.toml" csv::tests::golden -- --exact
n=0; bad=0; rows=0
while read -r p; do
  if ! cmp -s "$work/$n.js.csv" "$work/$n.rust.csv"; then
    bad=$((bad + 1)); echo "differs: $p"; diff "$work/$n.js.csv" "$work/$n.rust.csv" | head -5
  fi
  rows=$((rows + $(wc -l < "$work/$n.js.csv") - 1))
  n=$((n + 1))
done < "$list"
echo "$n files, $rows rows, $bad differ"
[ "$bad" -eq 0 ]
