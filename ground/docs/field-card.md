# Field card

One page to print. The app needs no internet.

## Before leaving

- set the spreading factor you are flying and flash both receivers to it (**T-Beams** → the **Receiver firmware** card → **Flash**)
- check every ChipSat on the **ChipSats** screen: module set, and none still running the dump
  firmware or the sensor bench
- set each board's **Phases**: for the ground and the air, the gap between flash log records and
  the gap between packets, and whether the packets follow the battery with that phase's own table.
  A board flashed before the phases has none and keeps its one fixed interval whatever is set
- if the app builds from this laptop's flight software, check the write confirm names the commit
  you mean to fly
- last of all, **Clear flash** on each board with **Save the flight software that is on it and put
  it back** ticked: the log chip is wiped, the flight software and its settings stay, nothing is
  saved. Unticked (the default) the board is left on the dump firmware. Pull it first if the chip
  holds anything you want
- flash a spare T-Beam the same way, so a dead receiver is a swap
- check the laptop clock, because logs are timestamped from it
- check free disk space; a long flight is a few MB, a chip dump about 5 MB
- close the Arduino IDE, including its Serial Monitor

## Starting up

1. plug the receiver in
2. open the app, go to **T-Beams**
3. pick the board and press **Read this one**; a board cannot be flashed before that
4. check its spreading factor on its sheet; change it there, or flash it if it needs the firmware
5. go to **Live** and wait for a unit card to appear in the Units list

## During the flight

- a unit that stops being heard beeps three times
- the log is written continuously, browser open or not; the top bar names the file

## After landing

- for the on-board log: clip the board on the ST-Link, **ChipSats** → **Read the board** → **Pull the flight log**. It takes a few minutes and flashes twice
- the flight software is saved before the dump and written back after only with that box ticked (off by default); if the pull fails, the sheet says whether the board is still on the dump firmware
- **Stop** ends a pull and keeps what has arrived
- the pull lands in the day's folder, `<day>/pulls/<time>-<board>`; **Data** lists it and shows the folder
- **Save log**, **Save CSV** and **Save raw** are on the pull's result, on its row in **Data**, and above the replayed log

## When something looks broken

- **"Device or resource busy"** — something else holds the port. Usually the Arduino IDE's Serial
  Monitor. Close it and try again.
- **busy for the first few seconds after plugging in** — ModemManager is probing the port. Wait
  fifteen seconds.
- **receiver firmware "not offered for flashing"** — the board has not been claimed with **Read this
  one** yet.
- **"no data"** on a receiver — wrong spreading factor, or it is a different board than you think.
  **Listen** on its T-Beams sheet says what a board is.
- **ChipSat buttons greyed out** — the board has not been read since it was clipped on. Press **Read
  the board**.
- **"did not build, so nothing was written"** — the flight software beside the app does not
  compile. The compiler's words are under the button. Fix it, or restart the app with
  `CHIPSAT_FSW=none` to write the copy it carries.
- **SF6 does not link, SF5 is not offered** — the T-Beam's SX1276 has no SF5, and its SF6 is the
  first generation format, which the ChipSat's SX1262 cannot produce: tested 2026-10-17 with both
  sides headerless, every packet heard, every CRC failed (`ERR,-7` in the log). Fly SF7 or above.

## Numbers

| | |
|---|---|
| Frequency | 915.0 MHz |
| Bandwidth | 125 kHz |
| Coding rate | 4/7 |
| Sync word | 0x12 |
| Preamble | 8 |
| Packet | 55 bytes, CRC-16/CCITT-FALSE over the first 53 |
