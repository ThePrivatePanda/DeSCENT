#pragma once

#include <stdint.h>
#include "../devices/EnvSensor.h"
#include "../devices/FuelGauge.h"
#include "../devices/Gps.h"
#include "../devices/Imu.h"
#include "../devices/Led.h"
#include "../devices/Radio.h"
#include "../devices/SensorData.h"
#include "../platform/I2cBus.h"
#include "../platform/System.h"
#include "../platform/Uart.h"
#include "../Settings.h"
#include "../devices/Flash.h"
#include "FlashLog.h"
#include "Health.h"
#include "Phase.h"
#include "Telemetry.h"

namespace ChipSatApp
{

// Boot sequence and the telemetry cycle. Landed after a flight it first puts the fall on the chip and
// looks for a fix to hold (kFixRun in a row under the hAcc limit), then sleeps: the IMU, GPS, radio
// and flash chip off, Stop2 (kDeepSleepWhenLanded). Once a ground transmit gap it wakes for one
// packet and its record, and a board turned or carried up stays awake (docs/phases.md)
class FlightController
{
public:
  FlightController(ChipSatPlatform::System &system,
                   ChipSatPlatform::I2cBus &i2c,
                   ChipSatPlatform::Uart &console,
                   ChipSatPlatform::Uart &gpsPort,
                   ChipSatDevices::Imu &imu,
                   ChipSatDevices::Gps &gps,
                   ChipSatDevices::EnvSensor &env,
                   ChipSatDevices::FuelGauge &gauge,
                   ChipSatDevices::Radio &radio,
                   ChipSatDevices::Led &led,
                   ChipSatDevices::Flash &flash,
                   FlashLog &flashLog);

  void setup();
  void loop();

private:
  struct CycleReads
  {
    uint32_t gateMs = 0;
    bool imuOkay = false;
    bool gpsOkay = false;
    bool socOkay = false;
    bool envOkay = false;
    uint32_t imuMs = 0;
    uint32_t gpsMs = 0;
    uint32_t socMs = 0;
    uint32_t envMs = 0;
  };

  void logResetCause();
  void beginSensors();
  void sendStatusPacket();
  void keepAlive();
  bool configRadio();                  // false if any step failed
  void sampleSensors();     // each sensor at its own pace, nothing waits on the radio
  void followPhase();       // hands fresh readings to the phase and acts on a change
  void followLanding();     // the fix to hold once landed, and asleep
  void goAsleep(uint32_t nowMs);
  void wakeUp(uint8_t part, uint8_t why, uint32_t detail);
  void cycleAsleep();       // one wake while asleep, the packet and its record
  bool wakeImu();           // asleep, for one wake. false with no orientation after a pin reset
  bool wakeIfMoved();       // asleep, a climb or a tilt wakes it for good
  bool fixGood(uint32_t nowMs) const;   // the NAV-PVT last seen is a fix to hold
  void readForPacket();     // asleep: the gauge and one barometer reading
  void restUntilDue();      // asleep between wakes
  uint32_t imuMaxAgeMs() const;
  const ChipSatSettings::Rate &transmitRate() const;
  uint32_t phaseTxIntervalMs(bool socOkay);
  void updateLogGaps(bool socOkay);
  void logIfDue();
  void anchorUtc(uint32_t nowMs);
  void buildPacket(ChipSatTelemetry::TelemetryPacket &into, bool &allFresh, uint16_t counter);
  bool transmitDue() const;
  void runCycle();
  void transmit();
  void noteResetup();   // after configRadio() outside setup
  void logReads(const CycleReads &reads) const;

  ChipSatPlatform::System &system_;
  ChipSatPlatform::I2cBus &i2c_;
  ChipSatPlatform::Uart &console_;
  ChipSatPlatform::Uart &gpsPort_;
  ChipSatDevices::Imu &imu_;
  ChipSatDevices::Gps &gps_;
  ChipSatDevices::EnvSensor &env_;
  ChipSatDevices::FuelGauge &gauge_;
  ChipSatDevices::Radio &radio_;
  ChipSatDevices::Led &led_;
  ChipSatDevices::Flash &flash_;
  FlashLog &flashLog_;

  ChipSatSensors::SensorData data_;
  ChipSatTelemetry::TelemetryPacket packet_{};
  ChipSatTelemetry::TelemetryPacket logPacket_{};   // kept apart, the radio has the other one
  FlightPhase phase_;
  Health health_;
  uint32_t imuSeenMs_ = 0;   // the IMU reading last handed to the phase
  uint32_t envSeenMs_ = 0;   // and the barometer reading
  uint32_t gpsSeenMs_ = 0;
  uint32_t envStartedMs_ = 0;
  bool envMeasuring_ = false;
  uint32_t gpsReadMs_ = 0;
  uint32_t gaugeReadMs_ = 0;
  uint32_t logWrittenMs_ = 0;
  uint32_t logImuMs_ = 0;      // the IMU reading the last record carried, for a gap of 0
  uint32_t utcAnchorMs_ = 0;
  bool utcAnchored_ = false;
  bool logStopped_ = false;
  uint16_t packetCounter_ = 0;
  uint16_t sentCounter_ = 0;   // the counter on the last packet handed to the radio, sent or not
  uint32_t previousReadMs_ = 0;
  bool phaseDue_ = false;   // a phase change wants the next packet now
  uint32_t txIntervalMs_;
  uint32_t logGapMs_[ChipSatSettings::kPhases] = {};   // what each log rate gives for the charge now
  uint32_t resetCsr_ = 0;
  uint32_t cycle_ = 0;   // for logging
  bool radioReady_ = false;
  bool transmitting_ = false;
  uint32_t txBeganMs_ = 0;
  uint32_t lastTxMs_ = 0;   // when the last transmission ended

  bool asleep_ = false;
  bool airWrittenSaid_ = false;   // landed: the console has said the fall is on the chip
  uint32_t lookingSinceMs_ = 0;   // landed, or last moved since
  uint32_t pvtSeenMs_ = 0;        // the last NAV-PVT the fix rule looked at
  uint8_t fixRun_ = 0;
  ChipSatSensors::GPSData bestFix_;   // of the run so far
  ChipSatSensors::GPSData heldFix_;   // what the packets carry asleep
  uint32_t jitter_ = 0;   // xorshift state for the landed transmit gap
  uint32_t wakes_ = 0;    // asleep, for the longer fix search every kAsleepFixLongEvery
  ChipSatSensors::Vector3f restUp_;   // which way was up in the IMU's frame as it fell asleep
  bool restUpKnown_ = false;
};

} // namespace ChipSatApp
