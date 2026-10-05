# Field card

One page to print. The app needs no internet.

## Before leaving

- set the spreading factor you are flying and flash both receivers to it (**T-Beams** → **Flash the receiver firmware**)
- check every ChipSat on the **ChipSats** screen: module set, and none still running the dump firmware
- flash a spare T-Beam the same way, so a dead receiver is a swap
- check the laptop clock, because logs are timestamped from it
- check free disk space; a long flight is a few MB, a chip dump about 5 MB
- close the Arduino IDE, including its Serial Monitor

## Starting up

1. plug the receiver in
2. open the app, go to **T-Beams**
3. pick the board and press **Read this one**; a board cannot be flashed before that
4. check its spreading factor on its sheet; change it there, or flash it if it needs the firmware
5. go to **Live** and wait for packets in the fleet table

## During the flight

- a unit that stops being heard beeps three times
- the log is written continuously, browser open or not; the top bar names the file

## After landing

- for the on-board log: clip the board on the ST-Link, **ChipSats** → **Read the board** → **Pull the flight log**. It takes a few minutes and flashes twice
- the flight software is saved before the dump and written back after; if the pull fails, the sheet says whether the board is still on the dump firmware
- the pull lands in its own `descent-pull-…` folder; **Data** lists it and shows the folder
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
- **SF6 does not link** — the ChipSat sends an explicit header, which the receiver cannot read at
  SF6. Fly SF7 or above.

## Numbers

| | |
|---|---|
| Frequency | 915.0 MHz |
| Bandwidth | 125 kHz |
| Coding rate | 4/7 |
| Sync word | 0x12 |
| Preamble | 8 |
| Packet | 55 bytes, CRC-16/CCITT-FALSE over the first 53 |
