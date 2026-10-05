# V2_6_X_Code

Flight software for the DeSCENT ChipSats (Wio-E5 boards)

## Before flashing
Configure src/config.h - most importantly, look at the id `kChipSatId` (different on every board) and radio module `CHIPSAT_RADIO_MODULE`

## Build
From V2_6_X, with only CMake installed:
```
./build.sh                    # build.cmd on Windows
./build.sh flash
./build.sh probes             # lists the ST-Links plugged in
./build.sh flash --probe SN   # when more than one is

./build.sh flash --sketch flash_dump    # a bench sketch instead of the flight software
./build.sh flash                        # and back to the flight software
```
The first run downloads everything into V2_6_X/deps, pinned and checked by sha256: the Arduino STM32
core 2.12.0 and CMSIS sources, the compiler, ninja and OpenOCD. Deleting deps/ and build/ is always safe

Arduino IDE still works: Generic WLE5JCIx, upload method SWD. It's needed for the library drivers

- STM32 core 2.12.0
- fly at log level 3 (`CHIPSAT_LOG_LEVEL`), level 4 prints enough to overflow the GPS port buffer

## Notes

- our own drivers for all five parts by default. The library versions still build, pick them per part in config.h (SparkFun BNO08x 1.0.6, SparkFun u-blox GNSS 2.2.28, Adafruit BME280 2.3.0, Adafruit MAX1704X 1.0.3, RadioLib 7.1.2)
- packets go out back to back, not timed off the battery
- a missing or dead sensor never stops setup or the packets, its data just goes out marked invalid
- IMU data older than 3 s goes out marked invalid
- a report quiet for 3 cycles gets the IMU soft reset, 3 times a boot at most
- the watchdog resets the board if nothing feeds it for 10 s
- packets print as `I PKT` lines too and descent_decode.py reads them
