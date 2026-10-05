#!/bin/sh
# Builds a default flight image per radio module and writes firmware/flight.json describing them.
# The dashboard reads the json rather than being told what the images are.
# Run from V2_6_X:  ./tools/emit_flight_images.sh
set -e
cd "$(dirname "$0")/.."
sf=7; dbm=14; id=255
printf '{\n  "settingsBlock": true,\n  "sf": %s,\n  "dbm": %s,\n  "csid": %s,\n  "images": [\n' "$sf" "$dbm" "$id" > firmware/flight.json
first=1
for pair in "le:1" "hp:2"; do
  name=${pair%%:*}; module=${pair##*:}
  ./build.sh build -DCHIPSAT_RADIO_MODULE=$module -DCHIPSAT_ID=$id \
                   -DCHIPSAT_SPREADING_FACTOR=$sf -DCHIPSAT_TARGET_DBM=$dbm >/dev/null
  cp build/chipsat.bin "firmware/flight-$name.bin"
  cp build/chipsat.elf "firmware/flight-$name.elf"
  sum=$(sha256sum "firmware/flight-$name.bin" | cut -d' ' -f1)
  size=$(stat -c%s "firmware/flight-$name.bin")
  [ $first -eq 1 ] || printf ',\n' >> firmware/flight.json
  printf '    {"file": "flight-%s.bin", "module": "%s", "bytes": %s, "sha256": "%s"}' \
         "$name" "$name" "$size" "$sum" >> firmware/flight.json
  first=0
done
printf '\n  ]\n}\n' >> firmware/flight.json
