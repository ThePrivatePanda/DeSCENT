# V2_6_X_Tests

Tests we ran while writing V2_6_X_Code. Logs are in data/, one folder per test.

* bench - sketches that go on the board
* test - laptop tests, the build line is at the top of each file
* tools - serial logger, power rig, power analysis, packet decoder
* data - logs and captures

Board is V2.5.x hardware, CSID 9 with the LE radio unless it says otherwise.

## Laptop

Copy the g++ line out of the file you want and run it. The python ones just run

```
python3 tools/test_descent_decode.py
python3 tools/test_imu_power_analyze.py
```

- test_telemetry and test_v25_telemetry each write out 20000 packets. diff the two files: since 2026-10-17 they differ only in byte 51 (V2.6 sends the cell voltage there, V2.5 sent percent) and the extra float on each line, so the ground station still decodes everything else
- test_imu_validity - the stale rule, edge cases and the millis wrap
- test_env_compensation - our BME280 maths against Adafruit's on the same raw readings, they agree
- test_radio_bytes - our radio command bytes and time on air against RadioLib's maths, they agree
- test_log - log lines at every level, then descent_decode.py reads the packet line back
- imu_sleep_hang - give the IMU library a failing I2C write and modeSleep() never returns, it just keeps retrying
- test_flash_log - the flash log against a chip kept in RAM: finding the end after a reset, cut off records, a full chip going round and the seam after it, power cuts mid-write and mid-erase, the 30 s RAM ring written in order on take-off, the ground trickle, the boot and dropped events, a slow erase and the HP pause dropping nothing. passes
- test_phase - the two phases on the barometer, on GPS when it is dead, and blind on the IMU: a climb is the air, 30 s still below half the climb is the ground, a hover never is, 3 min IMU quiet on a roof or a hill is the ground too, and a board takes off again from where it lies. Also the boot-air after a reset and its 5 min limit. Ends by replaying the real drops of 2026-10-13 (board 13) and 2026-10-17 (board 9) from test/data, the air at 184 s and 84 s on board 9 where the board's own log said about 180 and 84. passes
- test_settings - the settings block: every field, the CRC, the log rates, what is refused, and test/data/settings_layout4.inc, the same block settings.rs checks on the ground side
- test_pa_settings - the PA row for each module and power against the SX1261-2 datasheet table, a power the module has no row for fails to compile
- test_i2c_watch - the bus clear against a simulated stuck part, then the watchdog's rules: a held line, a part that jams twice is quarantined, timeouts in a row hold a part back, two parts timing out together is the bus. passes
- test_health - the sensor recovery ladders with fake drivers: retry, reset, set aside with a backoff that stops at 5 s in the air, one step a pass, the bus before any sensor, GPS first on the ground, a partly working IMU left alone in the air. passes
- test_env_recovery - a BME280 missing at boot comes back once it answers, paced like the flight loop. it never did before. passes
- test_imu_wake - the HP wake after a transmission against a simulated BNO085 that takes writes and never answers, it gives up within its 1.5 s budget instead of about 9.7 s. passes
- test_flight_loop - FlightController with fake drivers and a chip in RAM, once per module: a phase change inside the TX wait still sends the next packet at once (it waited a whole gap before 2026-10-28), every record from 28 s before take-off reaches the chip with nothing dropped through the TX waits, the log gaps follow the battery in the air, on the ground and in boot-air and fall back to the fixed gap with the gauge unread, the ring stays on the air's fixed gap whenever the board isn't logging as air, and a stuck erase says `stopped` once. passes

## Board

- imu_power_test - streaming the IMU draws about 14 mA more than asleep, waking it every 10 s costs under 0.5 mA
- imu_report_rates_test - 10 ms to 1 s comes out exact, then it clamps around 2.4 s
- imu_sleep_tilt_test - tilt is fine after a sleep. heading isn't, a quarter turn read 31 deg
- v25_heading - heading is shaky with the IMU streaming too, quarter turns read -39 to -102 deg
- power cut sweep - every cut booted clean. we saw it get stuck twice by hand but couldn't make it happen again
- bench runs - before the IMU fixes three of the four reports froze after about 8 min and the packets still said valid
- arrival firmware - board turned up running V2.5, sending every 30 s, packets decode
- roof drop, about 15 m - about two thirds of the packets decoded, GPS fix, no reboot. it took the landing fine

Driver sketches, all run on the bench and passed, logs not kept. Each _library one runs the same thing through the old library so the two can be compared on one board

- env_sensor_test, env_sensor_test_library - BME280 once a second
- fuel_gauge_test - charge and cell voltage once a second
- gps_test, gps_test_library - MAX-M10S fix, polled once a second
- gps_stream_test - our GPS driver the way it flies, NAV-PVT streaming. new, not run yet
- imu_test - the BNO085 set up the way our driver does it, plus the report lengths from its advertisement
- radio_test, radio_test_library - a flight-format packet a second plus a register dump
- flash_log_test - our flash driver and log, 300 records then finds the end again. new, not run yet
- flash_dump - prints the flash log for descent_decode.py, E then Y erases the whole chip. ground only, not run yet
- sensor_bench - every sensor at its own rate with each reading on the console as the driver gave it, an I2C scan at boot, a beat a second, and the flight packet back to back as CSID 254 at the module's lowest power. radio off for the first 30 s, R toggles it. writes nothing to the flash chip. one per module, -DCHIPSAT_RADIO_MODULE=1 or 2. new, not run yet
