# Logs, runs and replay

Reference for the log the app writes, how the app works a run out from it, and how a flash
dump is replayed. Moved out of the README, which is for getting started.

## Data

The **Data** screen is one table of everything the app has written: live sessions, pulls, and
firmware images read off boards, with the folder they are in at the top. A log or a pull can be
opened in replay or saved as a log or a CSV; a pull's raw dump and an image can be saved; and
**Write to a board…** takes an image to the ChipSats screen.

Picking a pull shows its report: one row per boot with how many records it holds, how long it ran,
when it started (in UTC, or "not dated" for a boot that never got GPS time), how many GPS anchors
were used to date it, and how fast the board's clock ran. Firmware from 2026-10-22 on also logs
each boot's reset cause and how many records it dropped, and those get two more columns. Beside it
is how the transfer went, and a line when the log had gone round the chip and the oldest records
were written over.

Saves are filed by local day and kind under the data folder: `2026-10-18/pulls/1432-board9/`,
`2026-10-18/images/`, `2026-10-18/logs/`, and `receiver-images/` for a T-Beam's firmware, which is
never offered for a ChipSat. Saves from before that sit flat under `descent-…` names and are still
listed. Firmware read off a board only gets its `.bin` name once all of it has arrived. A save cut
short by an error is deleted, and one cut short by the app quitting stays as a `.bin.part` that
nothing lists or offers.

## Recording

The app writes every line from every receiver to a log file on its own, browser open or not. The
top bar names that file. **Save a copy** downloads it and **New file** starts another.

| Control | Does |
|---|---|
| **Open a file…** | Replays a saved file, or an old lab log from the CSV receiver. In the bar and on Data. |
| **Save log** | With a log open: downloads it. A log the app wrote is the file itself; any other is written back out in the log format. |
| **Save CSV** | With a log open: downloads the decoded packets as a table. |
| **Save raw** | With a pull open: downloads its `dump.txt`. |

The CSV has one row per packet that passes its CRC: `time_ms`, `time_utc`, the receiver or boot it
came from, `uptime_ms` where the log gives it, every packet field, then RSSI, SNR and frequency
error. A field whose validity bit is clear is an empty cell, not a number, and `time_utc` is empty
for a boot that never got GPS time.

**Settings** has two kinds. Graph time span, when old graph data is dropped and the file name prefix are
remembered per browser. The status rules (lost, stale, low battery, weak RF, accel saturation and the
window for the same packet across receivers) live in the app and are the same on every screen that
has it open.

## Clearing

The **Clear** box at the bottom left holds one tick per thing: all units, only the unit shown,
receiver counts, hidden units, and starting a new log file. Tick what should go and press Clear.
With only the unit shown ticked and no unit picked, no unit is cleared.
Clearing receiver counts does not disconnect anything, and nothing already written to a log file is
touched unless you ask for a new one.

## Replaying a log without clicking

`?log=<url>` on the address opens a log on load and goes straight into replay:

```
http://127.0.0.1:8765/?log=http://127.0.0.1:44333/log.log
```

Repeat the parameter to merge several files, as opening several at once does. Percent-encode the
inner URL if it contains an `&`. A file that will not load leaves a working page and says why.

## Replaying a flash dump

**Open a file…** also takes a log written from a ChipSat's flash: a pull saved as a log from
the Data screen. Such a file names itself on its second line:

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
logs at its own gap per phase (20 Hz in the air, one record per 10 s on the ground as shipped) while
the counter moves once per transmission, and every boot in a dump starts at the same time, so the
counter tells you nothing about runs here.

The log on the chip is a ring, so a chip that has gone round no longer starts at its first
sector. The dump firmware prints sectors oldest first, by the boot count and uptime of each
sector's first good record, and the app reads them in that order. A chip that never went round
prints in address order, as before. On a chip that has gone round the first boot in the dump is
not boot 0: the boots before it were written over.

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
with a `3 earlier` button to unfold them (`hide earlier` folds them again) and `join all` beside it,
so a board that restarted thirty times is still one row.

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
| CRC | Checked in the app. A bad-CRC packet is counted and never updates a unit. |
| CSV receiver | Old `V2_5_X_T_BEAM_DECODE_CSV` T-Beams work too. The packet is rebuilt from the printed values to check the CRC. |
| Missed | Counter gaps, modulo 65536. |
| Reset | Counter drops, or 0 twice in a row. |
| Stale | No packet for 3× the unit's own measured interval (min 5 s). |
| Lost | No packet for 10 min. A unit that was being heard and goes lost beeps three times; the beep can be turned off in Settings. |
| Thresholds | All rules below are defaults; change them in Settings (kept in the app). |
| Several receivers | Identical bytes within 1.5 s = one packet. RSSI/SNR kept per receiver; the fleet list shows the best. |
| Saturation | ▲sat when valid acceleration reaches ±75 m/s² (BNO085 limit is ±8 g). |
| Low battery | Cell under 3.5 V (set in Settings). |
| Weak RF | Best RSSI below −115 dBm. |

## Receiver

`receiver/DescentRawReceiver/` is the T-Beam sketch that sends raw packets. See `receiver/README.md`.

## Files

| Path | What it does |
|---|---|
| `Open DeSCENT Ground.html` | Double-click launcher; opens `web/index.html`. |
| `web/index.html` | The page. |
| `web/js/stream.js` | The app's stream (`/api/stream`): unit rows, receivers, last lines, graph history. |
| `web/js/replay.js` | Drives a log open in the app (`/api/replay`) and reads its own stream. |
| `web/js/store.js` | Settings: display ones in browser storage, status rules through the app. |
| `web/js/charts.js` | History charts (uPlot). |
| `web/js/app.js` | Wires it together and draws Live and Replay. |
| `web/js/screens.js` | The tabs, and the strip in the top bar. |
| `web/js/tbeams.js` | The T-Beams screen. |
| `web/js/chipsats.js` | The ChipSats screen. |
| `web/js/datascreen.js` | The Data screen. |
| `web/js/bench.js`, `web/js/benchpane.js` | Reading the sensor bench's console, and its pane on a ChipSat's sheet. |
| `web/js/boardmodel.js`, `web/js/datamodel.js` | What those screens say, as plain functions with tests. |
| `web/js/serial_native.js` | Talks to the app: ports, flashing, pulls. |
| `web/sw.js` | List of files for an offline copy of the page. Nothing registers it. |
| `test/` | `node --test test/*.test.js`. Runs from a fresh clone on the small logs in `test/data`. The checks against the lab's full captures run when `DESCENT_LAB_LOGS` is the folder holding `V2_5_X` and `V2_6_X`, and are skipped with a line saying so when it is not. |
| `docs/design-guide.md` | Colours, type, layout rules. |

