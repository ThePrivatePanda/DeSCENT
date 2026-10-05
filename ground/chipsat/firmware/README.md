# Carried firmware

Copies of what fsw/firmware publishes, compiled into the app so a download can flash a board with
nothing else installed. Generated - do not edit. Replace them by running
`fsw/tools/emit_flight_images.sh` and copying the folder across.

* flash_dump.bin - the sketch that reads a board's flash log out over the console
* flight-le.bin - flight software for the LE module, 66576 bytes
* flight-hp.bin - flight software for the HP module, 66576 bytes
* flight.json, radio.json - what the images are, and the powers each module has rows for

The app reads these to recognise what a board is running, and writes one when there is no flight
software tree to build from, with the board's settings patched into its block first. The build
writes the block; the app only rewrites it.

Check the hashes against fsw/firmware before a release. build.rs watches each file, so a replaced
image reaches the binary - it used to watch the folder only, and a folder's timestamp does not
change when a file is overwritten in place.
