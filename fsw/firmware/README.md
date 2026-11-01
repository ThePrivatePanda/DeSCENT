# firmware

Built images kept at a stable path, for anything that needs one without building it first. The ground
app's build reads this folder and embeds them, so a downloaded app can put the dump sketch or a
flight image on a board without a compiler.

* flash_dump.bin - bench/flash_dump, the sketch that prints the flash log. Flashes at 0x08000000
* flash_dump.elf - the same build with symbols, for reading addresses
* flight-le.bin, flight-hp.bin, and their .elf - the flight software for each radio module, built with
  the defaults: id 255 meaning nobody configured it, SF7, +14 dBm. A board flashed with one announces
  itself as unconfigured rather than impersonating a real one. Each image carries a settings block,
  which the ground software rewrites with the board's own id, spreading factor, power and transmit
  and log rates before flashing. It only rewrites an image at the block layout it writes itself. The block names the module but a tool never changes it: the other module
  means the other image
* bench-le.bin, bench-hp.bin, and their .elf - bench/sensor_bench for each radio module: every sensor
  read and printed on the console, the flight packet back to back as CSID 254 at the module's lowest
  power. Not flight software, and no settings block to configure. Flashes at 0x08000000
* flight.json - which images these are, their sizes and sha256, and the defaults they were built
  with. The ground software reads it rather than being told
* radio.json - what the firmware will actually accept: the spreading factor range, and per module the
  transmit powers and the PA bytes each one programs. Generated from constants.h by
  tools/emit_radio_json.sh, so the table exists once. The ground UI reads this rather than keeping a
  copy, because a second copy of that table is how a board ends up configured for an amplifier it
  does not have

Rebuild all of them from fsw/ with `./tools/emit_firmware.sh`, then rebuild the app so it carries
the new ones; its build watches each file here. The build is reproducible, and CI runs the same
script on every change under fsw/ and fails when the result is not what is committed, so an image
that has fallen behind its source cannot ship unnoticed. A change to the flight software is
therefore not finished until this folder has been regenerated and committed with it.

build/ is wiped and rewritten by every build, which is why these are copied out rather than pointed at.

A ground app with this fsw/ tree beside it can build its own flight, dump and bench images and
radio.json from the tree instead, in fsw/build, and keeps them in its data folder. It never writes
here, so this folder is still only what emit_firmware.sh made.
