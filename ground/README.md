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
transmit power and the gaps for each phase, and press the button.

A flight has two phases and the board tells them apart itself from its altitude: the **ground**
from switch-on, on the bench or mounted on the drone, however it is handled; the **air** once it
has climbed 15 m (30 m on GPS altitude, which stands in when the barometer is dead), through the
release and the fall. It is the ground again after 30 s at rest below half of what it climbed, or
3 minutes with the IMU quiet at any height (a roof, a hill), and the next climb is the air again.
A release is not detected, and a drone hovering dead still stays in the air. The **Phases** block
has a card for each: how often it logs to the chip (a gap between records in seconds) and how often
it transmits, a gap after each packet in seconds or **follow the battery** with that phase's own
table, five bands for a charge above 75 %, down to 50 %, 35 %, 20 %, and below that. Thresholds
have to fall from one band to the next, and the phase's gap then applies only while the gauge
cannot be read. A fresh image transmits every 60 s on the ground and every second in the air, logs
every 50 ms in the air and keeps one record per 10 s on the ground, about two days of ground before
the chip goes round. A ground log gap of 0 keeps nothing from the ground. The board holds the last
30 s in RAM, so a take-off writes the 30 s before it too. Every setting goes into the image's
settings block and the board reads it at boot.

**Clear flash** erases the log chip and nothing else: the flight software is read off, the dump
firmware erases the chip, and the flight software goes back on with its settings as they were.
Nothing is saved to disk, and the confirm says when the board was last pulled. It is the last
click before launch. A full chip no longer stops logging, it overwrites its oldest sector.

With the flight software beside the app and **Use this computer's flight software** ticked, the
app builds the flight, dump and bench images from it, about 13 seconds a part, keeps them by
source hash and names each by its commit and the files changed since. **Rebuild** on the ChipSats
screen builds whatever is not built yet. A tree that does not build refuses to write a flight or
bench image rather than write the old one, and Pull and Clear use the dump firmware the app
carries until it does. Unticked, or with no tree, it writes the image the app carries with the
settings patched in, about two seconds. The pane says which. The module is never patched: to
change it, flash the other image. Writing an image saves what was on the board only when that box
is ticked.

Pulling a log takes minutes and flashes the board twice. It first checks that the console port is
this board's, saves the flight software and puts it back afterwards, and **Stop** keeps what has
arrived. Each pull gets a folder in the data folder under the day, like `2026-10-18/pulls/1432-board9`.

## Bench

The Bench screen puts the sensor bench on a ChipSat and shows its console part by part: a card
each for the IMU, barometer, GPS, gauge, I2C bus, flash and radio with what the board said and how
long it has been quiet, every start of the board with its reset cause, and the raw console. The
bench sends the flight packet as board 254 once it has run 30 s, and R on its console turns the
radio on or off. It never writes the flash chip. Not run on a board yet.

## Building it yourself

`cd app && cargo build --release`. The page is compiled into the binary, so a change under web/
needs a rebuild before it is served. So are the ChipSat images in `fsw/firmware` and the receiver
image in `receiver/firmware`: that is what a downloaded app flashes with nothing else installed.
For its own builds the app looks for the flight software in `fsw/` beside it, or wherever
`CHIPSAT_FSW` points. The decoders for packets, flash records and the settings block are compiled
into the app, so a change to one of those formats in the tree needs the app rebuilt too.

## Known issues

* nothing in the ChipSats screen has been run against a probe in anger; the flash and pull paths
  have, the patch path has not
* the Bench screen and the builds from this computer's tree have not been used on a board yet
* a read attaches over SWD, which leaves the debug domain powered until the board is power-cycled:
  do not measure sleep current on a board that has been under the clip
