#!/bin/sh
# Rebuilds everything firmware/ holds from the tree it sits in: the dump sketch, one default flight
# image per radio module with flight.json describing them, and radio.json from the real headers.
# Run from fsw/:  ./tools/emit_firmware.sh [OUT]       OUT is firmware/ unless given
# CI runs this and fails when the result differs from what is committed, so an image that has
# fallen behind the source cannot ship. The build is reproducible: same source, same bytes.
set -e
cd "$(dirname "$0")/.."
out=${1:-firmware}
mkdir -p "$out"

./build.sh build --sketch flash_dump >/dev/null
cp build/chipsat.bin "$out/flash_dump.bin"
cp build/chipsat.elf "$out/flash_dump.elf"

# The defaults a board gets before anyone configures it: id 255, which is not a board, so an
# unconfigured one says so instead of impersonating a real one.
sf=7; dbm=14; id=255
printf '{\n  "settingsBlock": true,\n  "sf": %s,\n  "dbm": %s,\n  "csid": %s,\n  "images": [\n' "$sf" "$dbm" "$id" > "$out/flight.json"
first=1
for pair in "le:1" "hp:2"; do
  name=${pair%%:*}; module=${pair##*:}
  ./build.sh build -DCHIPSAT_RADIO_MODULE=$module -DCHIPSAT_ID=$id \
                   -DCHIPSAT_SPREADING_FACTOR=$sf -DCHIPSAT_TARGET_DBM=$dbm >/dev/null
  cp build/chipsat.bin "$out/flight-$name.bin"
  cp build/chipsat.elf "$out/flight-$name.elf"
  sum=$(sha256sum "$out/flight-$name.bin" | cut -d' ' -f1)
  size=$(stat -c%s "$out/flight-$name.bin")
  [ $first -eq 1 ] || printf ',\n' >> "$out/flight.json"
  printf '    {"file": "flight-%s.bin", "module": "%s", "bytes": %s, "sha256": "%s"}' \
         "$name" "$name" "$size" "$sum" >> "$out/flight.json"
  first=0
done
printf '\n  ]\n}\n' >> "$out/flight.json"

./tools/emit_radio_json.sh > "$out/radio.json"

# Leave the tree configured for the flight software with its own defaults, as a build after a
# checkout would be, so the next ./build.sh flash is not a bench sketch.
./build.sh build >/dev/null
echo "wrote $out: flash_dump, flight-le, flight-hp, flight.json, radio.json"
