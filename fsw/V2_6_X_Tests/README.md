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

- test_telemetry and test_v25_telemetry each write out 20000 packets. diff the two files, they come out identical, so the ground station can still decode us
- test_imu_validity - the stale rule, edge cases and the millis wrap
- test_env_compensation - our BME280 maths against Adafruit's on the same raw readings, they agree
- test_radio_bytes - our radio command bytes and time on air against RadioLib's maths, they agree
- test_log - log lines at every level, then descent_decode.py reads the packet line back
- imu_sleep_hang - give the IMU library a failing I2C write and modeSleep() never returns, it just keeps retrying
- test_flash_log - the flash log against a chip kept in RAM: finding the end after a reset, cut off records, a full chip, erasing ahead

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
