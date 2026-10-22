# DeSCENT

Flight software and ground software for the Cornell SSDS DeSCENT ChipSat, a 20 g satellite deployed
in hundreds from a sounding rocket near 100 km and transmitting over LoRa during its fall. The two
halves live together because they are used together, and either builds without the other.

* fsw - the flight software that runs on the ChipSat. `cd fsw && ./build.sh setup` once, then
  `./build.sh flash` over an ST-Link. Per-board settings go on the build command, so
  `./build.sh flash -DCHIPSAT_ID=12 -DCHIPSAT_SPREADING_FACTOR=9` is a board of its own
* ground - the dashboard: receives telemetry, configures and flashes ChipSats and T-Beams, pulls a
  board's flash log and runs the sensor bench on a board. `cd ground/app && cargo run --release`

A board built with no `-DCHIPSAT_ID` reports 255, which is not a board, so an unconfigured one says
so instead of impersonating a real one.

Pulled logs and session logs are written outside the repo, under `~/DeSCENT`, one folder per day.

## Licence

MIT, see LICENSE. The flight software began as part of the SSDS DeSCENT project; see NOTICE.

## Releases

Tagging `ground-v<version>` builds the dashboard for Windows, Linux and macOS and attaches them to
the release. Bump `ground/app/Cargo.toml` to the same version, or the app cannot tell that a newer
one is out. The flight software is not released as a binary: it is built for the board it is going on.
