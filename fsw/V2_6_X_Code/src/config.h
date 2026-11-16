#pragma once
// Everything you would realistically change: which board this is, which drivers to build, and the
// timings and thresholds of the flight cycle.
// Values fixed by the board, a datasheet or a protocol live in constants.h

#include <Arduino.h>

// Which implementation gets compiled for each part
#define CHIPSAT_DRIVER_LIBRARY 1
#define CHIPSAT_DRIVER_OWN     2

// Each one can be overridden with -DCHIPSAT_x_DRIVER=n. System, I2C and UART are library only, no own version
#ifndef CHIPSAT_SYSTEM_DRIVER
#define CHIPSAT_SYSTEM_DRIVER CHIPSAT_DRIVER_LIBRARY  // arduino core + ST HAL
#endif
#ifndef CHIPSAT_I2C_DRIVER
#define CHIPSAT_I2C_DRIVER    CHIPSAT_DRIVER_LIBRARY  // Wire
#endif
#ifndef CHIPSAT_UART_DRIVER
#define CHIPSAT_UART_DRIVER   CHIPSAT_DRIVER_LIBRARY  // HardwareSerial
#endif
#ifndef CHIPSAT_IMU_DRIVER
#define CHIPSAT_IMU_DRIVER    CHIPSAT_DRIVER_OWN      // ours, or SparkFun BNO08x 1.0.6
#endif
#ifndef CHIPSAT_GPS_DRIVER
#define CHIPSAT_GPS_DRIVER    CHIPSAT_DRIVER_OWN      // ours, or SparkFun u-blox GNSS 2.2.28
#endif
#ifndef CHIPSAT_ENV_DRIVER
#define CHIPSAT_ENV_DRIVER    CHIPSAT_DRIVER_OWN      // ours, or Adafruit BME280 2.3.0
#endif
#ifndef CHIPSAT_GAUGE_DRIVER
#define CHIPSAT_GAUGE_DRIVER  CHIPSAT_DRIVER_OWN      // ours, or Adafruit MAX1704X 1.0.3
#endif
#ifndef CHIPSAT_RADIO_DRIVER
#define CHIPSAT_RADIO_DRIVER  CHIPSAT_DRIVER_OWN      // ours, or RadioLib 7.1.2
#endif

// Which Wio-E5 is on the board, from the module label: "Wio-E5-LE" or plain "Wio-E5" (HP)
#define CHIPSAT_RADIO_MODULE_LE 1
#define CHIPSAT_RADIO_MODULE_HP 2
#ifndef CHIPSAT_RADIO_MODULE
#define CHIPSAT_RADIO_MODULE    CHIPSAT_RADIO_MODULE_LE
#endif

// Console log level: 0 off, 1 error, 2 warn, 3 info, 4 debug
#ifndef CHIPSAT_LOG_LEVEL
#define CHIPSAT_LOG_LEVEL 4
#endif

namespace ChipSatConfig
{

// 255 is "nobody set this". A default has to be something, because the id is compiled in, and the
// old default of 9 was a real board - so any build without -DCHIPSAT_ID impersonated board 9 and two
// unconfigured boards merged into one on the ground. 255 is not a board, so it shows up as unset
#ifndef CHIPSAT_ID
#define CHIPSAT_ID 255
#endif
constexpr uint8_t kChipSatId = CHIPSAT_ID;   // per board, -DCHIPSAT_ID=12
static_assert(kChipSatId != 0, "0 reads as a board that exists; 255 is the unset value");

constexpr uint32_t kConsoleBaud = 115200;
// The core's TX buffer, SERIAL_TX_BUFFER_SIZE, set in CMakeLists.txt and build_opt.h. A packet's
// cycle prints about 900 bytes at log level 4 before the radio starts, and with the core's 64 bytes
// the loop waited about 70 ms for them to go out, losing one or two IMU readings each transmission
// (2026-11-08 pulls, gaps of 100-160 ms in the 20 Hz records). Holding a whole cycle, nothing waits
constexpr size_t   kConsoleTxBufferBytes = 2048;

// LED pin levels. Nobody has checked which one actually lights it
constexpr uint8_t kLedLevelAtBoot = LOW;
constexpr uint8_t kLedLevelDuringTx = LOW;
constexpr uint8_t kLedLevelAfterTx = HIGH;

// How the loop runs. Sensors keep their own pace, a packet takes the newest reading each one has as
// long as it is younger than its limit here, and anything older goes out with its validity bit clear
constexpr uint32_t kImuMaxAgeMs = 500;      // reports arrive every 50 ms
constexpr uint32_t kEnvMaxAgeMs = 2000;
constexpr uint32_t kGaugeMaxAgeMs = 30000;

// How often each sensor is asked, when nothing else is waiting on it
constexpr uint32_t kEnvIntervalMs = 200;
constexpr uint32_t kEnvConversionMs = 120;   // x16 takes 113, see kEnvOversampling
constexpr uint32_t kGpsIntervalMs = 500;     // the receiver only has a new one each second
constexpr uint32_t kGaugeIntervalMs = 1000;

// How often the uptime is tied to GPS time in the log and on the console, once the receiver has it
constexpr uint32_t kUtcAnchorIntervalMs = 10000;

// Flash log. Every record is 64 bytes, so 20 Hz fills the 2 MB chip in about 27 minutes, which is
// why only the air is logged at that rate by default. Records go into a RAM ring first; on the ground
// only one per ground gap reaches the chip, so a take-off still writes the 30 s before it. The ring
// only moves at 580 pending on the ground, so while the board doesn't log as air the records come at
// the faster of the ground gap and the air's FIXED gap, whatever the air's bands say.
// The gaps are in the settings block, these are what a fresh image carries, flat: the bands equal
// the fixed gap, so a fresh image never moves with the battery. ms, ground then air. A gap of 0 is
// every IMU reading (20 Hz) in either phase, all of it kept. On the ground that fills the 2 MB chip
// in about 27 minutes and then overwrites the oldest sectors, older flights included (decided
// 2026-11-06; from 2026-10-22 a ground 0 wrote nothing, before that it was every reading too).
// The ground's is the ground transmit gap: the page writes one ground rate to both (2026-11-16)
constexpr uint32_t kLogFixedMsDefault[2] = {60000, 50};

// 30 s at 50 ms, 38.4 KB of static RAM. A full chip overwrites its oldest sector, so a board nobody
// cleared still logs (decided 2026-10-19, was stop-when-full)
constexpr uint32_t kLogRingRecords = 600;
constexpr uint8_t  kLogDrainPerCall = 4;    // per call, so sampling and the radio keep going
constexpr uint32_t kLogRingSlackRecords = 20;   // 1 s of margin on the ground for an erase

// System events (log/SystemEvent.h) pool in RAM, one run per source, what, part and reason. A repeat
// within kLogSystemQuietMs of the last one is counted, not written again. A run is written once it
// has been quiet that long, or kLogSystemRunMs after it began, so a storm that never stops still
// reaches the chip and a reset loses at most that much. Board 15's IMU retrying every 300 ms for
// minutes is then 2 records a minute where it would be 200. Both picked, not measured
constexpr uint32_t kLogSystemQuietMs = 5000;
constexpr uint32_t kLogSystemRunMs = 30000;
constexpr uint8_t  kLogSystemRuns = 8;   // runs open at once, 20 bytes each. Full, the oldest is written early

// Flight phases (app/Phase.h): the ground, and the air from the IMU moving until it is quiet again.
// The IMU decides both ways, and while an altitude is live it has to agree: a climb to go up, flat
// to come down. The barometer alone never puts a board in the air. On 2026-11-01 barometer readings taken during 22 dBm transmits read about 46 m high, on
// the ground, which is above the old 15 m climb: desk boards went to the air, and one stayed there
// at least 46 minutes and wrapped its chip. Falling is not detected as its own phase: on the
// 2026-10-13 drops the IMU read 0.3 to 28 m/s2 on the drone and the same during the release
//
// What decides the phase
constexpr float    kQuietAccelMps2 = 2.0f;      // a reading under this and kQuietGyro is quiet: lying in grass in some wind is not 0.5
constexpr float    kQuietGyroRadPerSec = 0.35f; // about 20 degrees a second
// The air at the slightest hint, the ground only after a long quiet (decided 2026-11-09): the old
// 10 s window, 80 % moving and 75 % filled, was lost to every brown-out, IMU gap and run of bad
// readings. Half the readings of 2 s at 20 Hz moving is a hint; a few bad readings are not, and a
// gap of up to a second still leaves room for it. Replayed on the 2026-11-07/08 pulls, 1-4 s windows
// and 10-40 readings put the same flights up within 1.5 s of each other and kept the boards in the
// air on handling alone just as long (fsw/tools/phase_replay.py)
constexpr uint32_t kMotionWindowMs = 2000;
constexpr uint32_t kMotionSlotMs = 50;          // one slot per IMU report
constexpr uint16_t kHintMoving = 20;            // moving readings in the window that are the air
constexpr uint32_t kLandedMs = 30000;           // no moving reading this long, flat with an altitude live, is the ground at any height
// No moving reading this long is the ground whatever the altitude says. Boards lying still on
// 2026-11-08 read under 0.14 m/s2 and 2 deg/s while their barometer jumped too much to read flat, and
// board 17 transmitted back to back until its battery browned out 8.5 min after touchdown. A running
// drone reads moving within seconds, a fall the barometer sees resets the count (kDescentStepM), and
// a tumble takes a board back to the air at once (kTumbleRadPerSec), so a minute costs little air
// time and strands a board for a minute at most
constexpr uint32_t kLandedImuMs = 60000;
constexpr uint32_t kGroundQuietMs = 180000;     // instead of kLandedMs when no altitude was live all flight
constexpr uint32_t kDescentBinMs = 1000;        // the barometer is falling when kDescentBins of these in a row
constexpr uint8_t  kDescentBins = 4;            // each have their lowest and highest reading kDescentStepM
constexpr float    kDescentStepM = 5.0f;        // under the bin before: 5 m/s for 3 s
constexpr uint32_t kBootAirMaxMs = 300000;      // ms of uptime, boot-air logging ends here at the latest
constexpr float    kFlatAltitudeM = 3.0f;       // flat: each second's lowest and highest barometer reading both stay within this
constexpr float    kFlatAltitudeGpsM = 15.0f;   // the same on GPS altitude
constexpr uint32_t kWakeQuietMs = 30000;        // 20 Hz on the ground this long without a moving reading is 1 Hz again
// A fall at 20 Hz is the air at once, from the ground or boot-air, no climb needed. Every drop pulled
// so far (boards 9, 17, 18, 2026-10-18 and 11-08) tumbled at a median 448-663 deg/s for 7 s and
// more, handling never had a median over 188 deg/s and only in bursts. Some releases also read near
// 0 g for 250-370 ms before the tumble: the felt acceleration, gravity put back from the quaternion
constexpr float    kTumbleRadPerSec = 5.236f;   // 300 deg/s
constexpr uint32_t kTumbleMs = 1000;            // held this long
constexpr float    kFreeFallMps2 = 3.0f;        // felt acceleration under this
constexpr uint32_t kFreeFallMs = 150;           // held this long
constexpr uint32_t kSpellGapMs = 200;           // either goes on across a gap in the readings this short, a tumble across readings under its line too
// The orientation turned this much against kTurnSpanMs before counts as movement for the quiet
// timers, never for the air: a fall nearly always turns, if slowly. On 2026-11-08 boards lying still
// turned 0.03-0.6 deg in 5 s at the median and at most 7.3 deg at the 99th percentile, falls 74-131
// deg at the median and 8-31 deg in their slowest 5 s. One 70.6 deg jump on a still board (board 17
// boot 4, the magnetometer's heading coming in) is why it can't take a board to the air
constexpr float    kTurnedRad = 0.2618f;        // 15 deg, twice the still boards' 99th percentile
constexpr uint32_t kTurnSpanMs = 5000;

// Landed after a flight, the GPS looks for a fix to hold, then everything sleeps and wakes once a
// ground transmit gap for one packet (FlightController.h). The receiver's own hAcc on a trusted 3D
// fix, kFixRun in a row
constexpr uint32_t kFixGoodMm = 10000;
constexpr uint32_t kFixFairMm = 20000;          // what will do once kFixRelaxMs have passed since landing
constexpr uint32_t kFixRelaxMs = 300000;
constexpr uint8_t  kFixRun = 5;
constexpr uint32_t kFixAfterLandingMs = 30000;  // the first of the run no sooner than this after landing
constexpr uint32_t kLandedJitterMs = 2000;      // each transmit gap asleep moves up to this either way, so boards that landed together don't keep colliding
// Asleep, each wake looks this long for a fresh fix, longer every kAsleepFixLongEvery-th, and sends
// the held one when none is good enough (the user, 2026-11-16)
constexpr uint32_t kAsleepFixMs = 10000;
constexpr uint32_t kAsleepFixLongMs = 60000;
constexpr uint32_t kAsleepFixLongEvery = 10;
// Gravity this far from where it lay when it fell asleep is a board that was moved: awake again. The
// same 15 deg as kTurnedRad, picked, not measured. Gravity only, so the heading the magnetometer
// corrects on a still board (70.6 deg once on board 17) can't wake it
constexpr float    kAsleepTiltRad = 0.2618f;

// Altitude: the vertical rate and height in the Phase log line, the climb, and the flat the IMU's
// landing needs
constexpr uint32_t kPhaseRateWindowMs = 800;    // the vertical rate is over at least this long
constexpr float    kPhaseBaselineRateMps = 0.3f; // the ground baseline only follows a sat this still
constexpr float    kPhaseBaselineGain = 0.02f;   // per altitude reading, so about 10 s to settle on the barometer
constexpr float    kRiseGainM = 15.0f;          // climbed this far above the baseline on the barometer, whatever the IMU says, is the air
constexpr float    kRiseGainGpsM = 30.0f;       // on GPS altitude: it drifted 13 m on the ground on 2026-10-13
constexpr uint32_t kAltitudeStaleMs = 3000;     // an altitude source older than this is not live

// How often a packet goes out: the gap after each, per flight phase (app/Phase.h), as a fresh image
// carries them in its settings block; the dashboard writes a board's own. Each phase has a fixed gap
// and may follow the battery instead, with its own table: one gap per state-of-charge band, so a
// board running down spends its last charge on fewer packets. Off by default in every phase. A
// phase that follows the battery still uses its fixed gap while the gauge cannot be read.
// Battery data is read and sent every packet either way.
// Order: ground, air.
// The air is the whole flight and its gap is the flight rate; CHIPSAT_TX_INTERVAL_MS sets it on the
// build line, 0 sends back to back
#ifndef CHIPSAT_TX_INTERVAL_MS
#define CHIPSAT_TX_INTERVAL_MS 1000
#endif
constexpr uint32_t kTransmitFixedMsDefault[2] = {60000, CHIPSAT_TX_INTERVAL_MS};
constexpr bool     kRateFromSocDefault[2] = {false, false};
constexpr uint8_t  kSocPercentDefault[4] = {75, 50, 35, 20};
constexpr uint32_t kSocIntervalMsDefault[5] = {10000, 30000, 60000, 120000, 180000};
// The flight gap on its own, for the parts of the build that only know that one
constexpr uint32_t kTxIntervalMs = CHIPSAT_TX_INTERVAL_MS;

// Resets the board if the code hangs. Longest stretch with no wait in it is ~1.8 s (GPS setup with no
// receiver), by reading, not measured. 32 s at most on this chip
constexpr uint32_t kWatchdogTimeoutMs = 5000;
static_assert(kWatchdogTimeoutMs >= 1 && kWatchdogTimeoutMs <= 32000, "IWatchdog ignores anything outside this");

// At boot a packet goes out between sensor starts if the last one is older than this
constexpr uint32_t kBootKeepAliveMs = 1000;

// Start attempts for the library drivers. Ours try once and keep retrying in the background
constexpr uint8_t  kInitAttempts = 5;
constexpr uint16_t kInitRetryDelayMs = 500;

// IMU
constexpr uint16_t kImuReportIntervalMs = 50;   // all four reports. The library driver overflows above 65 ms
constexpr uint16_t kImuGroundIntervalMs = 1000; // the same on the ground until something moves it, own driver only
constexpr uint8_t  kImuEnableAttempts = 5;       // tries per report when turning them on
constexpr uint16_t kImuEnableSpacingMs = 50;     // pause after a report turned on
constexpr uint16_t kImuEnableRetryDelayMs = 75;  // pause after one that didn't
constexpr uint16_t kImuBootSettleMs = 1000;      // after a reset. Ours stops early when the IMU says it's up
constexpr uint32_t kImuRetryMs = 10000;   // how often a lost IMU gets another try, own driver only
constexpr uint8_t  kImuInitAttempts = 1;   // at boot, after that the background retry takes over
constexpr uint16_t kImuWakeSettleMs = 150;       // after wake, before the reports are turned on again
constexpr uint16_t kImuFreshWaitWakeMs = 3000;   // the same after a wake
constexpr uint32_t kImuFreshPollMs = 2;          // between checks while waiting

// Own driver only. It waits for the IMU's own answers instead of fixed settles
constexpr uint16_t kImuResetWaitMs = 1500;    // for reset complete. The library waited 300 + 200 + our 1000
constexpr uint16_t kImuReplyWaitMs = 250;     // product id and each enable's reply, the first took 170 ms on the bench
constexpr uint16_t kImuPollMs = 5;            // between reads while waiting for an answer

// IMU health checks
constexpr uint8_t  kImuMaxServicePasses = 16;   // I2C reads per IMU update
constexpr uint32_t kImuSoftResetSettleMs = 300;   // after a soft reset, library driver only

// GPS
constexpr uint16_t kGpsPvtWaitMs = 1200;      // library driver only, it polls
constexpr uint16_t kGpsPvtMaxAgeMs = 1500;    // older positions go out invalid
constexpr uint8_t  kGpsInitAttempts = 1;      // read() resends the settings later, so boot doesn't wait
constexpr uint32_t kGpsReconfigMs = 5000;     // settings are resent at most this often while they look lost
constexpr uint16_t kGpsNmeaLostBytes = 20;    // more non-UBX bytes between reads means NMEA is back on, settings lost
constexpr uint16_t kGpsAckWaitMs = 300;     // a config reply is 10 bytes, about 10 ms on the wire
constexpr uint8_t  kGpsDynamicModel = 8;    // airborne <4g
constexpr uint16_t kGpsMeasurementIntervalMs = 1000;   // one position a second
constexpr uint16_t kGpsNavigationRate = 1;              // a solution every measurement
constexpr uint8_t  kGpsWakeAttempts = 3;   // out of standby the first bytes may only wake it, so a setting is sent until acked

// A fix needs this many satellites before we believe the position. Three fix the position, a fourth
// fixes the clock, so below four the receiver is still converging
constexpr uint8_t  kGpsMinSatellites = 4;

// Throw away the receiver's stored position at boot, so it can never report a stale location from a
// previous run. Keeps the almanac and ephemeris, so a warm start is still fast
constexpr bool     kGpsClearStoredPosition = false;
constexpr uint16_t kGpsResetSettleMs = 500;   // how long the receiver gets to restart afterwards

// BME280
constexpr uint8_t  kEnvOversampling = 0x05;   // 0 off, 1 x1, 2 x2, 3 x4, 4 x8, 5 x16. x16 takes up to 113 ms
constexpr uint16_t kEnvMeasureTimeoutMs = 100;   // on top of that
constexpr uint16_t kEnvMeasurePollMs = 5;
constexpr uint16_t kEnvCalibrationWaitMs = 50;   // the chip copies its calibration out after a reset
constexpr uint16_t kEnvResetSettleMs = 10;
constexpr uint32_t kEnvRetryMs = 10000;   // how often read() tries again to start a BME280 that failed

// Altitude is measured against this. 1013.25 hPa is the standard atmosphere, which is what to use
// unless someone can set the day's actual sea level pressure before launch
constexpr float kSeaLevelPressureHpa = 1013.25f;

// Fuel gauge
constexpr uint32_t kGaugeRetryMs = 10000;   // how often read() tries again to start a MAX17048 that failed

// Radio
// From the build like the spreading factor below, so the ground app can put one board on another
// channel: -DCHIPSAT_RADIO_FREQUENCY_MHZ=903.5. The T-Beams have to be told the same, or hear nothing
#ifndef CHIPSAT_RADIO_FREQUENCY_MHZ
#define CHIPSAT_RADIO_FREQUENCY_MHZ 915.0
#endif
constexpr float    kRadioFrequencyMhz = CHIPSAT_RADIO_FREQUENCY_MHZ;
constexpr float    kRadioBandwidthKhz = 125.0f;
// Both of these are free of the module. Spreading factor changes time on air and how far a packet
// carries, not how much power goes out: 55 bytes at 125 kHz CR 4/7 is 143 ms at SF7, 456 ms at SF9
// and 3187 ms at SF12 - figures from timeOnAirUs in RadioSubGhzBytes.h, not from memory.
// Transmit power is what the amplifier is set up for, and the module decides
// which rows exist - an LE board has no high power amplifier to ask
// Guarded like the driver selectors, so one board's settings can come from the build command instead
// of an edit: ./build.sh flash -DCHIPSAT_SPREADING_FACTOR=12 -DCHIPSAT_TARGET_DBM=20
#ifndef CHIPSAT_SPREADING_FACTOR
#define CHIPSAT_SPREADING_FACTOR 7
#endif
#ifndef CHIPSAT_TARGET_DBM
#define CHIPSAT_TARGET_DBM 14
#endif
constexpr uint8_t  kRadioSpreadingFactor = CHIPSAT_SPREADING_FACTOR;   // 5 to 12
constexpr int8_t   kRadioTargetDbm = CHIPSAT_TARGET_DBM;   // LE: 10, 14, 15. HP: 14, 17, 20, 22
constexpr uint8_t  kRadioCodingRate = 7;   // 4/7
constexpr uint8_t  kRadioSyncWord = 0x12;
constexpr uint16_t kRadioPreambleLength = 8;
constexpr float    kRadioCurrentLimitMa = 140.0f;   // 0 to 140. applyPaConfig overwrites it, see Radio.h

// Imu::wake(), ms, plus up to 325 ms for a try already started. Only asleep after landing, once a
// wake; flight no longer sleeps the IMU for a transmission (the new boards run every sensor through
// 22 dBm SF12). Unbounded it could take 9.7 s with a hub that takes writes and never answers. A
// guess, not measured
constexpr uint32_t kImuWakeBudgetMs = 1500;

// Sleep mode (WFI) between loop passes and in every wait of 1 ms or more. Only the core stops:
// SysTick, the UARTs, I2C, SPI and the radio run on and SysTick wakes it each ms, so nothing waits
// longer than it does now. Not Stop2, which stops millis() and the GPS port. The saving is not
// measured; false builds the old spinning loop for the comparison
constexpr bool kSleepWhenIdle = true;

// Landed with a fix held, Stop2 instead of WFI between wakes; false keeps WFI, for an A/B
constexpr bool kDeepSleepWhenLanded = true;
constexpr uint32_t kSleepMaxMs = 4000;   // one Stop2 at most: the IWDG (kWatchdogTimeoutMs) keeps counting through it

// Sensor watchdogs (app/Health.h), all ms
constexpr uint32_t kHealthGapMs = 200;                 // between any two recovery steps
constexpr uint32_t kRecoverBackoffMs = 1000;           // first set-aside, doubles
constexpr uint32_t kRecoverBackoffAirMs = 5000;        // ceiling in the air, a fall is ~15 s
constexpr uint32_t kRecoverBackoffGroundMs = 30000;    // ceiling on the ground
constexpr uint32_t kImuSilentMs = 1000;        // a report not moving this long is silent
constexpr uint32_t kImuGraceMs = 2000;         // after a wake or restart
constexpr uint32_t kImuWatchRetryMs = 1000;
constexpr uint32_t kImuWatchSettleMs = 5000;   // a restart can take ~2.5 s plus four enables
constexpr uint32_t kEnvStaleMs = 2000;
constexpr uint32_t kEnvWatchRetryMs = 2000;
constexpr uint32_t kEnvWatchSettleMs = 1000;
constexpr uint32_t kGaugeStaleMs = 5000;
constexpr uint32_t kGaugeWatchRetryMs = 5000;
constexpr uint32_t kGaugeWatchSettleMs = 3000;
constexpr uint32_t kGpsSilentMs = 10000;       // no NAV-PVT at all, fix or not
constexpr uint32_t kGpsWatchRetryMs = 0;       // the driver's own poll and reconfig already ran for kGpsSilentMs
constexpr uint32_t kGpsWatchSettleMs = 5000;

// I2C bus watchdog (platform/I2cWatch.h)
constexpr uint32_t kBusSlowMs = 20;             // ms, a failed transfer this slow spun in the HAL (100 ms, twi.c I2C_TIMEOUT_TICK)
constexpr uint32_t kBusFaultWindowMs = 1000;    // ms, timeouts on two parts within this = bus fault
constexpr uint8_t  kBusTimeoutsToHold = 2;      // in a row on one part
constexpr uint32_t kBusHoldMs = 1000;           // ms, doubles
constexpr uint32_t kBusHoldMaxMs = 30000;       // ms
constexpr uint8_t  kBusJamsToQuarantine = 2;
constexpr uint32_t kBusJamWindowMs = 60000;     // ms
constexpr uint32_t kBusQuarantineMs = 30000;    // ms, doubles
constexpr uint32_t kBusQuarantineMaxMs = 300000;   // ms
constexpr uint32_t kBusClearRetryMs = 1000;     // ms after a failed clear, doubles
constexpr uint32_t kBusClearRetryMaxMs = 30000; // ms
constexpr uint32_t kBusSettleMs = 1000;         // ms, sensor steps wait this long after a clear

// Sensor retries
// A sensor that fails its first try gets tried hard for the first half minute, then backs off to its
// own interval. Measured on the bench 2026-09-24: on a healthy rail the IMU is up 2.5 s after boot,
// but on a cell it took 19 s on one board and 66 s on another, all of it spent waiting between
// 10 s retries. A four minute fall can't spend a fifth of itself on that. The slow interval is still
// what a genuinely dead sensor gets, so the loop never sits on one
constexpr uint32_t kSensorRetryFastMs = 250;
constexpr uint32_t kSensorRetryFastForMs = 30000;

constexpr uint32_t sensorRetryMs(uint32_t uptimeMs, uint32_t slowMs)
{
  return uptimeMs < kSensorRetryFastForMs ? kSensorRetryFastMs : slowMs;
}

} // namespace ChipSatConfig
