# V2_6_X_Tests

Tests we ran while writing V2_6_X_Code. Logs are in data/, one folder per test.

* bench - sketches that go on the board, `./build.sh flash --sketch NAME` from fsw/ or the IDE. the RadioLib ones build that way too
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
- test_flash_log - the flash log against a chip kept in RAM: finding the end after a reset, cut off records, a full chip going round and the seam after it, power cuts mid-write and mid-erase, the 30 s RAM ring written in order on take-off, the ground trickle, landed every record at once whatever the gap, the boot and dropped events, a slow erase and the HP pause dropping nothing. A ground gap of 0 keeps every record. System events: a retry every 300 ms for 2 min is 4 records whose counts add up to all 400, two events never pool together, a lone one goes after 5 s quiet, a full table writes its oldest at once, kept on the ground at gap 0 and 10 s, a failed write is its own event. passes
- test_phase - the phase on the IMU and the altitude. The air at the slightest hint: 20 moving readings in 2 s, a tumble or 250 ms near 0 g at 20 Hz, or a 15 m climb (30 m on GPS) with nothing from the IMU, through a brown-out, bad readings or the IMU dead; a few stray moving readings never. Handling is the air and the ground 30 s after it stops. Asleep, an hour of wakes a minute apart with one barometer reading each is no movement, one reading 16 m up is the climb, and a tilt found at a wake is 20 Hz. The ground at any height: 30 s quiet and flat (a roof, a tree, a still hover), a minute quiet whatever the barometer does (a still board whose barometer jumps every second), 3 min with no altitude all flight, and the same with the IMU dead since no reading is quiet (boot-air with no IMU ends at 30 s); a barometer fall and the orientation turning 15 deg in 5 s hold it off, a 70 deg heading snap only delays it. A tumble from 1 Hz is the air 2.0 s after a release at worst. Ends by replaying the real drops of 2026-10-13 (board 13) and 2026-10-17 (board 9) from test/data in time order, each sparse record held up to 1 s: in the air by the climb, through each release and fall, the ground within 45 s of touchdown, and up again on the handling around them. passes
- test_settings - the settings block: every field, the CRC, the log rates, what is refused, and test/data/settings_layout4.inc, the same block settings.rs checks on the ground side
- test_pa_settings - the PA row for each module and power against the SX1261-2 datasheet table, a power the module has no row for fails to compile
- test_i2c_watch - the bus clear against a simulated stuck part, then the watchdog's rules: a held line, a part that jams twice is quarantined, timeouts in a row hold a part back, two parts timing out together is the bus, and the same steps as system events with their codes. passes
- test_health - the sensor recovery ladders with fake drivers: retry, reset, set aside with a backoff that stops at 5 s in the air, one step a pass, the bus before any sensor, GPS first on the ground, a partly working IMU left alone in the air, an IMU asleep after landing and the GPS in standby left alone and judged again once awake, and each step as a system event with its codes. passes
- test_env_recovery - a BME280 missing at boot comes back once it answers, paced like the flight loop. it never did before. passes
- test_imu_wake - Imu::wake() (no longer used around a transmission) against a simulated BNO085 that takes writes and never answers, it gives up within its 1.5 s budget instead of about 9.7 s.. The 1 Hz and 20 Hz switch returns at once and sends one Set Feature a service() call, 20 Hz readings from 30 ms after it with none lost. passes
- test_gps - GpsMaxM10s against a fake receiver: hAcc out of NAV-PVT, standby is one UBX-RXM-PMREQ and nothing written after it for a minute, and the wake gets an ack before the settings go again. passes
- test_flight_loop - FlightController with fake drivers and a chip in RAM, once per module, the take-off being the fake IMU reading a running drone: a phase change inside the TX wait still sends the next packet at once (it waited a whole gap before 2026-10-28), every record from 28 s before take-off reaches the chip with nothing dropped through the TX waits, the log gaps follow the battery in the air, on the ground and in boot-air and fall back to the fixed gap with the gauge unread, the ring stays on the air's fixed gap whenever the board isn't logging as air, boot-air transmits at the air's gap and the ground's from bootairend on (or the air's still, flown out of it), and a stuck erase says `stopped` once., and back to back at SF12 in the air every 50 ms record reaches the chip with the IMU read through every transmission (only 445 of 3000 records had it with the old HP sleep) passes. Also each source's system event reaching the chip end to end (IMU ladder, phase, radio refusing to start), and off the air a record at the faster of the ground gap and the air's fixed gap: ground 20 ms with air 100 s is 6000 records in 2 min, ground 0 one per IMU reading. The console is the core's TX buffer at 115200 baud, so a cycle's lines hold the loop as they do on a board: with 64 bytes 8 checks fail with 120-135 ms gaps around each packet as on 2026-11-08, with 2048 none. Barometer readings taken on air (11 m high, as board 18's) never reach the phase: the landing is the 30 s one, at 156 s, where handing them over lands on the IMU's minute at 180 s. A desk board knocked every 3 s with its barometer 40 m up through every SF12 packet never goes to the air (handing those readings over takes it up). Asleep a wake each ground gap, one packet and one record each, Stop2 4 s at a time with the watchdog fed and the IMU, GPS and chip off; with no fix the GPS looks 10 s a wake and 60 s every tenth and the held fix goes out, a fix again replaces it and its time goes on the chip; turned about the vertical it sleeps on, tipped 30 deg it wakes (reason tilted), a 30 m climb wakes it into the air (reason climb), a dead IMU gets one pin reset a wake and the packets go on, and a ground rate of 0 runs the wakes back to back. The first line at boot says the log level and the frequency, also built with -DCHIPSAT_RADIO_FREQUENCY_MHZ=903.5

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
