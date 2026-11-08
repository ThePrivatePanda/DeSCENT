#pragma once

#include <stdint.h>

// Sensor recovery, the I2C bus watchdog, phase changes, radio and flash trouble, all as one kind of
// flash log record (kLogEventSystem, app/FlashLog.h) so they can be pooled and read together. Every
// one of them also has its console line, written by the caller as before.
//
// The numbers are what goes on the chip, so a code keeps its meaning once a board has flown with it.
// A new one gets the next free number here and in ground/app/src/dump.rs. A source or what is never 0,
// a part or reason of 0 is none

namespace ChipSatLog
{

// source
constexpr uint8_t kSourceSensor = 1;   // Health.cpp, a sensor's recovery ladder
constexpr uint8_t kSourceBus = 2;      // platform/I2cWatch.cpp
constexpr uint8_t kSourcePhase = 3;    // Phase.cpp, and FlightController for asleep and awake
constexpr uint8_t kSourceRadio = 4;    // FlightController::transmit
constexpr uint8_t kSourceFlash = 5;    // FlashLog.cpp

// part: what it happened to, 0 when nothing in particular
constexpr uint8_t kPartNone = 0;
constexpr uint8_t kPartImu = 1;
constexpr uint8_t kPartEnv = 2;     // BME280, also the barometer for the phase
constexpr uint8_t kPartGauge = 3;
constexpr uint8_t kPartGps = 4;
constexpr uint8_t kPartRadio = 5;
constexpr uint8_t kPartFlash = 6;
constexpr uint8_t kPartBusOther = 7;   // an I2C address that is none of the three above

// what, sensor. detail is the last one's of the run
constexpr uint8_t kSensorRetry = 1;     // went bad, the driver only re-reads. detail: IMU silent reports, bit 0 accel to bit 3 quat
constexpr uint8_t kSensorDefer = 2;     // reset held in the air, the part still partly works. detail as retry
constexpr uint8_t kSensorReset = 3;     // first reset: IMU soft reset, BME280 or gauge restart, GPS port. detail 1 if the driver said ok
constexpr uint8_t kSensorRestart = 4;   // second reset: IMU pin reset, GPS hot start. detail as reset
constexpr uint8_t kSensorAside = 5;     // resets didn't help, left alone. detail: backoff ms
constexpr uint8_t kSensorOk = 6;        // a good reading again. detail: ms it was bad, reason 0
constexpr uint8_t kSensorPinReset = 7;  // IMU NRST pulsed, whoever asked. reason: kWhyBoot, kWhyBusStuck, the restart's failure or the ladder's reason. detail: failed restarts in a row before it
constexpr uint8_t kSensorDriverRestart = 8;   // the IMU driver's own background restart ended. detail 1 ok, 0 failed. reason: kWhyNoAck, kWhyNoReset, kWhyNoEnable, 0 when ok
// reason, sensor
constexpr uint8_t kWhyAsleep = 1;     // IMU
constexpr uint8_t kWhySilent = 2;     // IMU, every report stopped
constexpr uint8_t kWhyPartial = 3;    // IMU, some reports stopped
constexpr uint8_t kWhyStale = 4;      // BME280 or gauge, ready but no fresh reading
constexpr uint8_t kWhyNotReady = 5;   // BME280 or gauge, the driver isn't up
constexpr uint8_t kWhyNoPvt = 6;      // GPS, bytes but no NAV-PVT
constexpr uint8_t kWhyNoBytes = 7;    // GPS, nothing at all
constexpr uint8_t kWhyBoot = 8;       // IMU pin reset at boot, before the first start
constexpr uint8_t kWhyBusStuck = 9;   // IMU pin reset before a bus clear the IMU is blamed for
constexpr uint8_t kWhyNoAck = 10;     // IMU restart, the probe or the reset write got no ACK
constexpr uint8_t kWhyNoReset = 11;   // IMU restart, no reset complete in time
constexpr uint8_t kWhyNoEnable = 12;  // IMU restart, a Set Feature write failed

// what, bus. part is whose address, kPartNone for the clear
constexpr uint8_t kBusHeld = 1;         // a line low before a transfer. reason: lines. detail: address
constexpr uint8_t kBusJam = 2;          // a line low right after one, blamed on that address. reason: lines. detail: address
constexpr uint8_t kBusTimeout = 3;      // timeouts in a row, part held back. detail: hold ms
constexpr uint8_t kBusFault = 4;        // two parts timing out together, the bus is cleared. detail: ms the transfer took
constexpr uint8_t kBusClear = 5;        // a clear ran. reason: kWhyClear*. detail: clock pulses it took
constexpr uint8_t kBusQuarantine = 6;   // jammed kBusJamsToQuarantine times. detail: quarantine ms
constexpr uint8_t kBusRelease = 7;      // out of quarantine, on probation. detail: address
// reason, bus: held and jam say which lines were low as bits, the clear how it ended
constexpr uint8_t kWhySdaLow = 1;
constexpr uint8_t kWhySclLow = 2;
constexpr uint8_t kWhyClearOk = 1;        // both lines high after it
constexpr uint8_t kWhyClearLow = 2;       // SDA still low after the pulses
constexpr uint8_t kWhyClearSclHeld = 3;   // SCL never came up, clocking can't help

// what, phase. part is what decided: the IMU, or the altitude source (env or GPS). detail 0 unless it says
constexpr uint8_t kPhaseToAir = 1;
constexpr uint8_t kPhaseToGround = 2;
constexpr uint8_t kPhaseDescent = 3;       // the barometer started falling in the air, which holds it there
constexpr uint8_t kPhaseBootAirEnd = 4;    // the log goes from boot-air to the ground, the radio phase doesn't move
constexpr uint8_t kPhaseImuFast = 5;       // the IMU to 20 Hz on the ground. reason kWhyMoving
constexpr uint8_t kPhaseImuSlow = 6;       // back to 1 Hz on the ground without going up. reason kWhyQuiet
constexpr uint8_t kPhaseAsleep = 7;        // landed with a fix held: GPS standby, Stop2. part GPS, reason kWhyGoodFix, detail hAcc mm
constexpr uint8_t kPhaseAwake = 8;         // out of it, everything on again. reason kWhyMoving, or kWhyImuDead for the air on altitude
// reason, phase
constexpr uint8_t kWhyImuMotion = 1;       // most of 10 s of IMU readings moved
constexpr uint8_t kWhyQuiet = 2;           // no moving reading for kLandedMs (30 s)
constexpr uint8_t kWhyQuietNoAlt = 3;      // kGroundQuietMs (3 min), no altitude live all flight
constexpr uint8_t kWhyBaroDescent = 4;
constexpr uint8_t kWhyImuDead = 5;         // IMU missing past kImuDeadMs, the altitude's climb or rest decided
constexpr uint8_t kWhyBootAirLimit = 6;    // kBootAirMaxMs of uptime
constexpr uint8_t kWhyMotionHeld = 7;      // the motion window kMotionHeldMs with the altitude live and no climb
constexpr uint8_t kWhyQuietHigh = 8;       // quiet and flat kLandedHighMs, above half the climb
constexpr uint8_t kWhyMoving = 9;          // kWakeMoving of the last kWakeReadings at 1 Hz moved
constexpr uint8_t kWhyGoodFix = 10;        // kFixRun fixes in a row under the hAcc limit

// what, radio. detail is the radio's last code (int16, sign extended) unless it says otherwise
constexpr uint8_t kRadioTxFail = 1;      // reason: kWhyStartRefused or kWhyFinishFailed
constexpr uint8_t kRadioTxTimeout = 2;   // not done within time on air + 100 ms. detail: ms waited
constexpr uint8_t kRadioResetup = 3;     // configRadio again. reason: kWhySetupOk or kWhySetupFailed
// reason, radio
constexpr uint8_t kWhyStartRefused = 1;
constexpr uint8_t kWhyFinishFailed = 2;
constexpr uint8_t kWhySetupOk = 1;
constexpr uint8_t kWhySetupFailed = 2;

// what, flash. Only a write failure can reach this boot's log: the other two end it
constexpr uint8_t kFlashWriteFail = 1;    // reason: ChipSatPlatform::Status as a number. detail: index of the next record
constexpr uint8_t kFlashEraseStuck = 2;   // past kFlashEraseTimeoutMs. detail: sector
constexpr uint8_t kFlashStopped = 3;      // logging over for this boot. detail: records on the chip

struct SystemEvent
{
  uint8_t source;
  uint8_t what;
  uint8_t part;
  uint8_t reason;
  uint32_t detail;
  uint32_t ms;
};

// The controller points this at the flash log, the way the console gets set for the log lines. A
// test of one module that sets nothing just doesn't keep them
using SystemSink = void (*)(void *context, const SystemEvent &event);
inline SystemSink systemSink = nullptr;
inline void *systemSinkContext = nullptr;

inline void setSystemSink(SystemSink sink, void *context)
{
  systemSink = sink;
  systemSinkContext = context;
}

inline void noteSystem(uint8_t source, uint8_t what, uint8_t part, uint8_t reason, uint32_t detail, uint32_t nowMs)
{
  if (systemSink != nullptr) {
    systemSink(systemSinkContext, SystemEvent{source, what, part, reason, detail, nowMs});
  }
}

} // namespace ChipSatLog
