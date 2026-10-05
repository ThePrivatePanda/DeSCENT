# Logs, runs and replay

Reference for the log the app writes, how a run is worked out from it, and how a flash
dump is replayed. Moved out of the README, which is for getting started.

## Data

The **Data** screen is one table of everything the app has written: live sessions, pulls, and
firmware images read off boards, with the folder they are in at the top. A log or a pull can be
opened in replay or saved as a log or a CSV; a pull's raw dump and an image can be saved; and
**Write to a board…** takes an image to the ChipSats screen.

Picking a pull shows its report: one row per boot with how many records it holds, how long it ran,
when it started (in UTC, or "not dated" for a boot that never got GPS time), how many GPS anchors
were used to date it, and how fast the board's clock ran. Beside it is how the transfer went.

## Recording

The app writes every line from every receiver to a log file on its own, browser open or not. The
top bar names that file. **Save a copy** downloads it and **New file** starts another.

| Control | Does |
|---|---|
| **Open a file…** | Replays a saved file, or an old lab log from the CSV receiver. In the bar and on Data. |
| **Save log** | With a log open: downloads it. A log the app wrote is the file itself; any other is written back out in the log format. |
| **Save CSV** | With a log open: downloads the decoded packets as a table. |
| **Save raw** | With a pull open: downloads its `dump.txt`. |
| Autosave | Everything is also kept in the browser every 5 s. After a crash, the next visit offers to open, download or discard it. |

The page without the app has nothing writing to disk for it, so there it keeps its recording
controls: **Start** asks where to save and writes every line from every connected receiver to that
file, **Pause / Resume** and **Stop** do what they say, and **Save session** downloads everything
since the page opened. In browsers without the save-file picker, the recording is kept in memory
and downloaded on **Stop**.

The CSV has one row per packet that passes its CRC: `time_ms`, `time_utc`, the receiver or boot it
came from, `uptime_ms` where the log gives it, every packet field, then RSSI, SNR and frequency
error. A field whose validity bit is clear is an empty cell, not a number, and `time_utc` is empty
for a boot that never got GPS time.

**Settings** (graph time span, when old graph data is dropped, status thresholds, saving) are remembered per browser.

## Clearing

The **Clear** box at the bottom left holds one tick per thing: all units, only the unit shown,
receiver counts, hidden units, and starting a new log file. Tick what should go and press Clear.
Clearing receiver counts does not disconnect anything, and nothing already written to a log file is
touched unless you ask for a new one.

## Replaying a log without clicking

`?log=<url>` on the address opens a log on load and goes straight into replay:

```
http://127.0.0.1:8765/?log=http://127.0.0.1:44333/log.log
```

Repeat the parameter to merge several files, as opening several at once does. Percent-encode the
inner URL if it contains an `&`. A file that will not load leaves a working page and says why.

Note that the hosted site at ground.privatepanda.co **cannot** do this for a log served from your
own machine: a page loaded over https is not allowed to fetch `http://127.0.0.1`, and the browser
blocks it before any request is made. Use the app's own address for that, as above.

## Replaying a flash dump

**Open a file…** also takes a log written from a ChipSat's flash, exported by
`V2_6_X/tools/flash_to_ground.py` on the flight-software side. Such a file names itself on its
second line:

```
#DG,SRC,v1,kind=flash,from=CS64_2026-09-24.bin,boot=7,packets=120,anchor_uptime_ms=41230,anchor_utc=2026-09-24T11:02:03.500Z,anchor_tacc_ns=35000000
```

A boot that never got a GPS fix carries `time_source=start` instead of an anchor, meaning its
times were chosen by a human. The dashboard says so on screen, because once both are epoch
milliseconds a guessed time axis looks exactly like a real one.

A dump that names its boots with `#DG,BOOT,v1,n=<n>` is split into one run per boot, which the
counter alone could never do. Otherwise a chip log is not split into runs at all, nothing in
it is discarded as a duplicate reception, and the columns that are read off the transmission
counter — missed, restarts, Rx %, interval — show nothing rather than something wrong. The board
logs at 20 Hz while the counter moves once per transmission, and every boot in a dump starts at the
same time, so the counter tells you nothing about runs here.

A replay says so on the line under its bar, with how many boots it holds and how many of them have
GPS time, so a replayed dump cannot be mistaken for live reception. Records off a chip carry no
RSSI or SNR, so those columns stay blank and the weak
RF warning never fires: an absence, not a fault.

## Log format

```
# descent-ground log v1 started 2026-09-26T14:00:00.000Z
<unix ms>	<receiver>	<line exactly as received>
```

## Runs

A ChipSat's counter starts again from 0 every time it reboots, so a reflash would otherwise mix old
data into new. Each run is kept separately: the one transmitting now keeps the plain CSID, and
earlier runs become `64a`, `64b` and so on, oldest first. The fleet list shows one row per board
with a `+3` button to unfold them, so a board that restarted thirty times is still one row.

Runs are managed in the **Runs** tab of the history panel, beside Overview and IMU: every run of the
selected board, what it holds, and a button to join it into the next one or throw it away. The fleet
list carries the same actions in miniature.

If a restart was an accident — a knocked cable on the bench — press **Join into the next run** and
the earlier run goes back into the current one, history and all. The restart is still counted, so the
board does not read as one that never rebooted. Individual runs can be deleted instead.

## Screen

| Area | Shows |
|---|---|
| Left | Every field of the selected unit's latest packet. Fields whose validity bit is clear show `—` and the last trusted value. |
| Right top | One row per CSID: state, age, counter, packets, missed, Rx %, resets, battery, validity bits, best RSSI/SNR, packet interval, receivers that heard it. |
| Right bottom | History of the selected unit: Overview, IMU, Environment, GPS, Radio (per receiver). The time axis spans the data received. |

This is the Live screen, and Replay is the same screen over a log.

## Rules

| Item | Rule |
|---|---|
| CRC | Checked in the browser. A bad-CRC packet is counted and never updates a unit. |
| CSV receiver | Old `V2_5_X_T_BEAM_DECODE_CSV` T-Beams work too. The packet is rebuilt from the printed values to check the CRC. |
| Missed | Counter gaps, modulo 65536. |
| Reset | Counter drops, or 0 twice in a row. |
| Stale | No packet for 3× the unit's own measured interval (min 5 s). |
| Lost | No packet for 10 min. A unit that was being heard and goes lost beeps three times; the beep can be turned off in Settings. |
| Thresholds | All rules below are defaults; change them in Settings. |
| Several receivers | Identical bytes within 1.5 s = one packet. RSSI/SNR kept per receiver; the fleet list shows the best. |
| Saturation | ▲sat when valid acceleration reaches ±75 m/s² (BNO085 limit is ±8 g). |
| Low battery | Below 20 %. |
| Weak RF | Best RSSI below −115 dBm. |

## Receiver

`receiver/DescentRawReceiver/` is the T-Beam sketch that sends raw packets. See `receiver/README.md`.

## Files

| Path | What it does |
|---|---|
| `Open DeSCENT Ground.html` | Double-click launcher; opens `web/index.html`. |
| `web/index.html` | The page. |
| `web/js/packet.js` | Packet layout, CRC, decode. Mirrors `Telemetry.h` in the flight code. |
| `web/js/lines.js` | Turns one serial line (raw, CSV, hex dump) into an event. |
| `web/js/fleet.js` | Per-unit state: missed, resets, interval, stale/lost, history, dedupe. |
| `web/js/serial.js` | Web Serial ports. |
| `web/js/recorder.js` | Recording, log files, replay parsing. |
| `web/js/exportcsv.js` | An open log as CSV, one row per good packet. |
| `web/js/store.js` | Settings and crash autosave (browser storage). |
| `web/js/charts.js` | History charts (uPlot). |
| `web/js/app.js` | Wires it together and draws Live and Replay. |
| `web/js/screens.js` | The tabs, and the strip in the top bar. |
| `web/js/tbeams.js` | The T-Beams screen. |
| `web/js/chipsats.js` | The ChipSats screen. |
| `web/js/datascreen.js` | The Data screen. |
| `web/js/boardmodel.js`, `web/js/datamodel.js` | What those screens say, as plain functions with tests. |
| `web/js/serial_native.js` | Talks to the app when the page came from it. |
| `web/sw.js` | Offline copy for the hosted site. |
| `test/` | `node --test test/*.test.js`. Runs from a fresh clone on the small logs in `test/data`. The checks against the lab's full captures run when `DESCENT_LAB_LOGS` is the folder holding `V2_5_X` and `V2_6_X`, and are skipped with a line saying so when it is not. |
| `docs/design-guide.md` | Colours, type, layout rules. |

