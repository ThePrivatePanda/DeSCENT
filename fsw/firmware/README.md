# firmware

Built images kept at a stable path, for anything that needs one without building it first. The ground
software embeds flash_dump.bin at its own build time so it can put the dump sketch on a board without
a compiler.

* flash_dump.bin - bench/flash_dump, the sketch that prints the flash log. Flashes at 0x08000000
* flash_dump.elf - the same build with symbols, for reading addresses
* flight.bin, flight.elf - the flight software, built with the defaults in config.h: id 255 meaning
  nobody configured it, LE module, SF7, +14 dBm. A board flashed with this announces itself as
  unconfigured rather than impersonating a real one. Per-board settings still come from a -D build
  until the firmware carries a settings block
* radio.json - what the firmware will actually accept: the spreading factor range, and per module the
  transmit powers and the PA bytes each one programs. Generated from constants.h by
  tools/emit_radio_json.sh, so the table exists once. The ground UI reads this rather than keeping a
  copy, because a second copy of that table is how a board ends up configured for an amplifier it
  does not have

Rebuild both with, from V2_6_X:

```
./build.sh build --sketch flash_dump
cp build/chipsat.bin firmware/flash_dump.bin
cp build/chipsat.elf firmware/flash_dump.elf
./build.sh build            # put the tree back on the flight software
```

build/ is wiped and rewritten by every build, which is why these are copied out rather than pointed at.
