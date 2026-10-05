# DescentRawReceiver

The sketch that runs on a LilyGO T-Beam and turns it into a receiver: it listens for ChipSat
packets and forwards each one over USB as a line of hex. It decodes nothing - the dashboard does
that - so a receiver only has to agree with the ChipSats about the radio parameters.

* DescentRawReceiver - the sketch, with boards.h and utilities.h from LilyGO
* firmware - built images, whole-flash, written at 0x0

## Flash it

`./tbeam.py --flash ../../ground/receiver/firmware/descent-raw-receiver-1.2.1.bin` from fsw/tools,
then `--name 01` to label it. Or build it yourself with arduino-cli for `esp32:esp32:t-beam`;
firmware/README.md has the command.

Settings live in the board's own flash and survive a power cycle, but not a flash: the images are
whole-flash images and the span they cover includes the settings partition. `tbeam.py --flash`
reads the settings off the board first and puts them back.

## Serial protocol

115200 baud, ASCII, one line per event, `\n` terminated. `boards.h` prints a few start-up lines at boot; the dashboard ignores them.

| Line | When | Fields |
|---|---|---|
| `#DG,RX,v1,id=<ID>,f=915.0,bw=125.0,sf=9,cr=7,sync=0x12,pre=8` | boot, every 30 s, on `#GET`, after an accepted `#SET` | `ID` = last 3 bytes of ESP32 eFuse MAC, 6 uppercase hex |
| `PKT,<len>,<hex>,<rssi>,<snr>,<freqErr>` | good radio read | byte count; payload uppercase hex, no spaces; RSSI dBm (1 dp); SNR dB (2 dp); frequency error Hz (integer) |
| `ERR,<code>,<rssi>,<snr>` | bad radio read | RadioLib code (`-7` PHY CRC mismatch, `-24` header damaged); RSSI; SNR |
| `HB,<millis>,<ok>,<err>` | every 5 s | uptime ms; good packets since boot; errors since boot |
| `#DG,FATAL,<code>` | setup failure | RadioLib code; repeated every 5 s, board halted |
| `#GET` | sent to the board | no fields; reprints the `#DG,RX` header at once |
| `#SET,<key>,<value>` | sent to the board | `freq` 137-1020 MHz; `bw` 7.8 / 10.4 / 15.6 / 20.8 / 31.25 / 41.7 / 62.5 / 125 / 250 / 500 kHz; `sf` 6-12; `cr` 5-8; `sync` 0x00-0xFF; `pre` 6-65535 |
| `#DG,ERR,<key>` | command rejected | the key that would not take; nothing changed |

Example: `PKT,55,0000…0923,-50.0,13.75,-1234`

A failed `startReceive()` is reported as `ERR,<code>,0.0,0.00`.

Settings changed with `#SET` are kept in flash (NVS namespace `dgrx`) and come back after a reboot.

## Versions

* 1.2.1 - the header ends `,fw=<version>`, so a receiver says which build it is running instead of
  being guessed at from which keys it carries. A board that sends no `fw=` is 1.1.0 or older
* 1.2.0 - a name can be bound to the board's id, kept in NVS, shown on the screen's top line. The
  header carries `name=` always, empty when unset. Not shipped: 1.2.1 replaced it before anyone
  flashed it, so that one name never means two sets of bytes
* 1.1.0, 1.0.0 - no name, no version; the header ends at `pre=`

`kFirmwareVersion` in the sketch and the version in the file name are the same three numbers, and
nothing enforces that but this sentence. Change both.
