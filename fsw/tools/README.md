# tools

Python, standard library only unless a tool says otherwise. Run them from here.

Pulling a board's flash log is not here any more: the ground software does it from the ChipSats
screen, with nothing installed, and replays or exports what it pulled from its Data screen.

* tbeam.py - sets a T-Beam receiver's radio settings and name, and `--flash` writes an image from
  ground/receiver/firmware, keeping the settings across it. pyserial, and esptool for a flash, go
  into tools/.venv on first use
* descent_decode.py - decodes the 55 byte packet from hex, whether it came from a console log, a flash
  dump or the T-Beam. `--table`, `--csv`, `--detail`
* serial_log.py - timestamped console capture. Writes a .log with timestamps and a .raw without, so the
  decoder can read the raw one
* rig_run.py - drives the INA260 power rig and the ChipSat console for unattended power runs
* imu_power_analyze.py - works out the per-phase current from a rig run
* emit_firmware.sh - rebuilds everything ../firmware holds, which CI checks against the source on
  every change. emit_radio_json.sh is the part of it that turns constants.h into radio.json
* test_descent_decode.py, test_imu_power_analyze.py - their tests, `python3 test_*.py`

Older copies of the decoder, logger, rig and analysis scripts sit in ../V2_6_X_Tests/tools, from when
they lived with the tests. These are the ones kept up to date.
