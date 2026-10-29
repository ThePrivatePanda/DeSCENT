# V2_6_X_Code

Flight software for the DeSCENT ChipSats (Wio-E5 boards)

## Before flashing
Configure src/config.h - most importantly, look at the id `kChipSatId` (different on every board) and radio module `CHIPSAT_RADIO_MODULE`. The ground app's ChipSats screen sets the id, spreading factor, power and the per-phase gaps in the image's settings block instead, with no compiler

## Build
From fsw/, with only CMake installed:
```
./build.sh                    # build.cmd on Windows
./build.sh flash
./build.sh probes             # lists the ST-Links plugged in
./build.sh flash --probe SN   # when more than one is

./build.sh flash --sketch flash_dump    # a bench sketch instead of the flight software
./build.sh flash                        # and back to the flight software
```
The first run downloads everything into fsw/deps, pinned and checked by sha256: the Arduino STM32
core 2.12.0 and CMSIS sources, the compiler, ninja and OpenOCD. Deleting deps/ and build/ is always safe

Arduino IDE still works: Generic WLE5JCIx, upload method SWD. It's needed for the library drivers

- STM32 core 2.12.0
- fly at log level 3 (`CHIPSAT_LOG_LEVEL`), level 4 prints enough to overflow the GPS port buffer

## Notes

- our own drivers for all five parts by default. The library versions still build, pick them per part in config.h (SparkFun BNO08x 1.0.6, SparkFun u-blox GNSS 2.2.28, Adafruit BME280 2.3.0, Adafruit MAX1704X 1.0.3, RadioLib 7.1.2)
- two phases, ground and air (app/Phase.h). Air is a 15 m climb on the barometer, 30 m on GPS when the barometer is dead, or the IMU alone with neither. Ground again after 30 s at rest below half the climb, or 3 min with the IMU quiet at any height. A release is not detected
- a packet every 60 s on the ground and every 1 s in the air, as shipped
- after any reset it logs as in the air but transmits at the ground rate until it really climbs, for 5 min at most, so a reboot mid-fall keeps the fall
- the flash log is a ring over the whole chip, a full chip overwrites its oldest sector. The last 30 s sit in RAM, so a take-off writes the 30 s before it. On the ground one record per 10 s reaches the chip. It only stops, for the rest of a boot, after an erase that times out or 3 failed writes in a row, and says `W FLASH stopped` once
- every boot writes its reset cause to the log, and how many records it dropped when that grows
- a missing or dead sensor never stops setup or the packets, its data just goes out marked invalid
- IMU data older than 500 ms goes out marked invalid
- each sensor and the I2C bus has its own watchdog (app/Health, platform/I2cWatch): retry, reset the part, set it aside and try again later. In the air only a part that is fully dead gets reset. Host tested, not run on a board yet
- the board's watchdog resets it if nothing feeds it for 5 s
- packets print as `I PKT` lines too and descent_decode.py reads them
- the Cyc end line carries `stackfree=`, the least free stack since boot. Not looked at on a board yet
