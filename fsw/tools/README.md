# tools

Python, standard library only unless a tool says otherwise. Run them from here.

* flash_replay.py - the one command for getting a log off a board.
  `./flash_replay.py --port /dev/ttyUSB0` puts flash_dump on the board, reads the log, puts the
  flight software back, and opens the log in the ground software. Everything lands in
  V2_6_X/data/<date>/ as dump.txt, records.csv and ground.log. `--no-flash` and `--no-restore` skip
  either half of the flashing, `--no-browser` writes the files and stops, `--local` serves a checked
  out copy of the ground page, `--plot` is the old matplotlib window
* flash_to_ground.py - turns a flash dump into a "# descent-ground log v1" file, which the ground
  software opens with its own Open log button and replays like a live flight. Times come from the log's
  time anchors, so they are real UTC. A log with no anchor, which is what an indoor run gives, is
  dated from 1970 instead, so the spacing is right and the date is obviously not; `--start` sets a real one.
  RSSI and SNR go in empty, since nothing on the chip was received over the air. It also writes a
  `#DG,SRC` line per boot, which the dashboard shows as "Flash dump" with the anchor the times rest on,
  so a replay is never mistaken for live reception
* descent_decode.py - decodes the 55 byte packet from hex, whether it came from a console log, a flash
  dump or the T-Beam. `--table`, `--csv`, `--detail`
* serial_log.py - timestamped console capture. Writes a .log with timestamps and a .raw without, so the
  decoder can read the raw one
* rig_run.py - drives the INA260 power rig and the ChipSat console for unattended power runs
* imu_power_analyze.py - works out the per-phase current from a rig run
* test_descent_decode.py, test_imu_power_analyze.py - their tests, `python3 test_*.py`

Copies of the same tools live in the repo root's tools/ folder, which is where they started. These are
the V2_6_X ones and the only ones kept up to date
