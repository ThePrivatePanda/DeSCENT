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
// why only the air is logged at that rate. Records are made at the air's gap in every phase and go
// into a RAM ring first; on the ground only one per ground gap reaches the chip, so a take-off still
// writes the 30 s before it. The gaps are in the settings block, these are what a fresh image carries.
// Only slots 0 (ground) and 1 (air) are read, the rest stay so old blocks still parse
// ms. A ground gap of 0 writes nothing on the ground (before 2026-10-22 it meant every IMU reading),
// an air gap of 0 still means every IMU reading
constexpr uint32_t kLogIntervalMsDefault[4] = {10000, 50, 50, 10000};   // ground, air, then two unused
constexpr uint32_t kLogBlindIntervalMs = 200;    // unused since 2026-10-22, kept in the block

// 30 s at 50 ms, 38.4 KB of static RAM. A full chip overwrites its oldest sector, so a board nobody
// cleared still logs (decided 2026-10-19, was stop-when-full)
constexpr uint32_t kLogRingRecords = 600;
constexpr uint8_t  kLogDrainPerCall = 4;    // per call, so sampling and the radio keep going
constexpr uint32_t kLogRingSlackRecords = 20;   // 1 s of margin on the ground for an erase

// Flight phases (app/Phase.h): the ground, and the air from a climb until it is at rest
// again. Falling is not detected: on the 2026-10-13 drops the IMU's linear acceleration read 0.3
// to 28 m/s2 on the drone and the same during the release (motor vibration, and a flat sat is at
// terminal speed in half a second), and the drone itself descended 53 m before letting go, so
// neither a rate nor a drop from the peak tells a release from the drone
//
// What decides the phase
constexpr uint32_t kPhaseRateWindowMs = 800;    // the vertical rate is over at least this long
constexpr float    kPhaseBaselineRateMps = 0.3f; // the ground baseline only follows a sat this still
constexpr float    kPhaseBaselineGain = 0.02f;   // per altitude reading, so about 10 s to settle on the barometer
constexpr float    kRiseGainM = 15.0f;          // climbed this far above the baseline on the barometer: the air
constexpr float    kRiseGainGpsM = 30.0f;       // on GPS altitude: it drifted 13 m on the ground on 2026-10-13
constexpr uint32_t kAltitudeStaleMs = 3000;     // an altitude source older than this is not live
constexpr float    kQuietAccelMps2 = 2.0f;      // at rest: lying in grass in some wind is not 0.5
constexpr float    kQuietGyroRadPerSec = 0.35f; // about 20 degrees a second
constexpr float    kQuietAltitudeM = 4.0f;      // about half a hectopascal
constexpr float    kQuietAltitudeGpsM = 8.0f;   // GPS altitude wanders this much lying still
constexpr uint32_t kLandedMs = 30000;           // this long at rest, below half of what it climbed, is the ground
constexpr uint32_t kLandedNoImuMs = 120000;     // on altitude alone, with the IMU dead
constexpr uint32_t kGroundQuietMs = 180000;     // the IMU quiet this long is the ground at any height
constexpr uint32_t kMotionWindowMs = 10000;     // no altitude source: the IMU alone tells the air from the ground
constexpr float    kMotionFraction = 0.8f;      // this much of the window not quiet is a running drone
constexpr uint32_t kBlindLandedMs = 30000;      // and this long quiet after it is the ground
constexpr uint32_t kBootAirMaxMs = 300000;      // ms of uptime, boot-air logging ends here at the latest

// How often a packet goes out: the gap after each, per flight phase (app/Phase.h), as a fresh image
// carries them in its settings block; the dashboard writes a board's own. Each phase has a fixed gap
// and may follow the battery instead, with its own table: one gap per state-of-charge band, so a
// board running down spends its last charge on fewer packets. Off by default in every phase. A
// phase that follows the battery still uses its fixed gap while the gauge cannot be read.
// Battery data is read and sent every packet either way.
// Order: ground, air, then two unused slots (were falling and landed).
// The air is the whole flight and its gap is the flight rate; CHIPSAT_TX_INTERVAL_MS sets it on the
// build line, 0 sends back to back
#ifndef CHIPSAT_TX_INTERVAL_MS
#define CHIPSAT_TX_INTERVAL_MS 1000
#endif
constexpr uint32_t kRateFixedMsDefault[4] = {60000, CHIPSAT_TX_INTERVAL_MS, CHIPSAT_TX_INTERVAL_MS, 60000};
constexpr bool     kRateFromSocDefault[4] = {false, false, false, false};
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
constexpr float    kRadioFrequencyMhz = 915.0f;
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

// HP IMU wake after TX, ms, plus up to 325 ms for a try already started. Unbounded it could take
// 9.7 s with a hub that takes writes and never answers. A guess, not measured
constexpr uint32_t kImuWakeBudgetMs = 1500;

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
