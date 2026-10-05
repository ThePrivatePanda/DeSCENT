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
#include "../devices/Flash.h"
#include "FlashLog.h"
#include "Landing.h"
#include "Telemetry.h"

namespace ChipSatApp
{

// Boot sequence and the telemetry cycle
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
  uint32_t configuredTxIntervalMs();   // the gap after a transmission, not including it
  void sampleSensors();     // each sensor at its own pace, nothing waits on the radio
  void logIfDue();
  void anchorUtc(uint32_t nowMs);
  void buildPacket(ChipSatTelemetry::TelemetryPacket &into, bool &allFresh, uint16_t counter);
  bool transmitDue() const;
  void runCycle();
  void transmit();
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
  Landing landing_;
  uint32_t envStartedMs_ = 0;
  bool envMeasuring_ = false;
  uint32_t gpsReadMs_ = 0;
  uint32_t gaugeReadMs_ = 0;
  uint32_t logWrittenMs_ = 0;
  uint32_t utcAnchorMs_ = 0;
  bool utcAnchored_ = false;
  bool logFull_ = false;
  uint16_t packetCounter_ = 0;
  uint16_t sentCounter_ = 0;   // the counter on the last packet that actually went out
  uint32_t previousReadMs_ = 0;
  uint32_t txIntervalMs_;
  uint32_t cycle_ = 0;   // for logging
  bool radioReady_ = false;
  uint32_t lastTxMs_ = 0;

  // IMU stuck check
  uint32_t imuCountsLastCycle_[4] = {0, 0, 0, 0};
  uint8_t imuStuckCycles_ = 0;
  uint8_t imuSoftResets_ = 0;
};

} // namespace ChipSatApp
