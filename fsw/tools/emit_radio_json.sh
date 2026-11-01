#!/bin/sh
# Prints what the firmware accepts, from the real headers, as JSON. The ground UI reads this instead
# of keeping a second copy of the LE/HP power table, which already exists once in constants.h.
# Run from fsw/:  ./tools/emit_radio_json.sh > firmware/radio.json
set -e
here=$(dirname "$0")/..
src="$here/V2_6_X_Code/src"
test="$here/V2_6_X_Tests"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
echo "{"
echo '  "spreadingFactor": {"min": 5, "max": 12},'
# the block layout and the rate slots a fresh image carries, from the headers, so the dashboard
# prefills the same numbers and can tell a tree that writes another layout
g++ -std=gnu++17 -I"$test/test/mock" -I"$src" "$test/test/emit_battery_json.cpp" -o "$out/battery"
"$out/battery"
echo ","
first=1
for module in 1 2; do
  g++ -std=gnu++17 -I"$test/test/mock" -I"$src" -DCHIPSAT_RADIO_MODULE=$module \
      "$test/test/emit_radio_table.cpp" -o "$out/emit"
  [ $first -eq 1 ] || echo ","
  "$out/emit"
  first=0
done
echo
echo "}"
