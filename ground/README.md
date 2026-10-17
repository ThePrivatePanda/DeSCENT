# DeSCENT ChipSat

The ground software for the DeSCENT ChipSat: it receives telemetry from the LoRa receivers,
configures and flashes ChipSats over an ST-Link, and pulls a board's flash log. Run it on the
laptop at the bench or in the field. The same page is published to GitHub Pages for watching
telemetry, but flashing and pulling need the app, because they need a probe and a serial port.

* app - the Rust server. `cargo run --release` serves the page on 127.0.0.1:8765 and opens a browser
* web - the page itself: plain HTML, CSS and JavaScript, no framework and no build step
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

A flight has three phases and the board tells them apart itself from its altitude: **waiting**
from switch-on, on the ground or mounted on the drone, however it is handled; **in the air** once
it has climbed 15 m (30 m on GPS altitude, which stands in when the barometer is dead), through
the release and the fall; **landed** after 30 s at rest below half of what it climbed. A release
is not detected, and a drone hovering dead still is not landed. Landed re-arms, so the next climb
is the air again. The **Phases** block sets, one card per phase, how often it logs to the chip (a
gap between records in seconds) and how often it transmits: a gap after each packet in seconds, or
**follow the battery** with that phase's own table, five bands for a charge above 75 %, down to
50 %, 35 %, 20 %, and below that. Thresholds have to fall from one band to the next, and the
phase's gap then applies only while the gauge cannot be read. A fresh image logs every 50 ms in
the air and every second on the ground, so a board can sit switched on for hours without filling
its chip; with neither barometer nor GPS it logs at the blind gap, 200 ms, whatever the phase.
Every setting goes into the image's settings block and the board reads it at boot.

**Clear flash** erases the log chip and nothing else: the flight software is saved, the dump
firmware erases the chip, and the flight software goes back on with its settings as they were.
It is the last click before launch.

With the flight software beside the app it compiles the exact image for that board's module,
about 13 seconds, then writes the settings into its block. Without it, it writes the image the app
carries with the settings patched into it, about two seconds. The pane says which it is going to
do. The module is never patched: to change it, flash the other image.

Pulling a log takes minutes and flashes the board twice. It saves what is on the board first and
puts it back afterwards.

## Building it yourself

`cd app && cargo build --release`. The page is compiled into the binary, so a change under web/
needs a rebuild before it is served. So are the ChipSat images in `fsw/firmware` and the receiver
image in `receiver/firmware`: that is what a downloaded app flashes with nothing else installed. For the build-and-flash button, the app looks for the flight
software in `fsw/` beside it, or wherever `CHIPSAT_FSW` points.

## Known issues

* nothing in the ChipSats screen has been run against a probe in anger; the flash and pull paths
  have, the patch path has not
* a read attaches over SWD, which leaves the debug domain powered until the board is power-cycled:
  do not measure sleep current on a board that has been under the clip
