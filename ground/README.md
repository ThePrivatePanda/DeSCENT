# DeSCENT ChipSat

The ground software for the DeSCENT ChipSat: it receives telemetry from the LoRa receivers,
configures and flashes ChipSats over an ST-Link, and pulls a board's flash log. Run it on the
laptop at the bench or in the field. The same page is published to GitHub Pages for watching
telemetry, but flashing and pulling need the app, because they need a probe and a serial port.

* app - the Rust server. `cargo run --release` serves the page on 127.0.0.1:8765 and opens a browser
* web - the page itself: plain HTML, CSS and JavaScript, no framework and no build step
* chipsat - the firmware the app carries, so a fresh download can flash a board
* receiver - the sketch that runs on a T-Beam, and its built images
* test - `node --test test/*.test.js` for the page, `cargo test` in app/ for the server
* docs - the log format, replay, the design guide and the field card

## T-Beams

Plug one in and the T-Beams screen lists it. Name it, set its spreading factor to match the
ChipSats, and its packets appear on Live. A receiver remembers its own settings, so this is once
per board, not once per session.

## ChipSats

Clip a board to the ST-Link and the ChipSats screen reads it by itself. Set the radio module from
the label on the part - Wio-E5 is HP, Wio-E5-LE is LE - then set the board id, spreading factor,
transmit power and interval, and press the button.

With the flight software beside the app it compiles the exact image for that board, about 13
seconds. Without it, it writes the image the app carries with those settings patched into it, about
two seconds. The pane says which it is going to do. The module is never patched: to change it,
flash the other image.

Pulling a log takes minutes and flashes the board twice. It saves what is on the board first and
puts it back afterwards.

## Building it yourself

`cd app && cargo build --release`. The page is compiled into the binary, so a change under web/
needs a rebuild before it is served. For the build-and-flash button, the app looks for the flight
software in `fsw/` beside it, or wherever `CHIPSAT_FSW` points.

## Known issues

* nothing in the ChipSats screen has been run against a probe in anger; the flash and pull paths
  have, the patch path has not
* a read attaches over SWD, which leaves the debug domain powered until the board is power-cycled:
  do not measure sleep current on a board that has been under the clip
