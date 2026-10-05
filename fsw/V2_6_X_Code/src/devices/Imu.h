#pragma once

#include <stdint.h>
#include "../platform/I2cBus.h"
#include "../platform/Status.h"
#include "../platform/System.h"
#include "SensorData.h"

namespace ChipSatDevices
{

// One bit per report
enum ImuReport : uint8_t
{
  kImuLinearAcceleration = 1U << 0,
  kImuGyroscope = 1U << 1,
  kImuMagnetometer = 1U << 2,
  kImuOrientation = 1U << 3,
  kImuAllReports = 0x0F
};

// BNO085. Which driver is behind this is picked by CHIPSAT_IMU_DRIVER in config.h.
// All four reports stream all the time, it only sleeps around TX on the HP module
class Imu
{
public:
  Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system);

  ChipSatPlatform::Status begin(uint16_t reportIntervalMs);
  void service();   // call every loop pass
  ChipSatPlatform::Status waitForFresh(uint16_t timeoutMs);
  ChipSatPlatform::Status sleep();   // Ok if already asleep
  ChipSatPlatform::Status wake();    // Ok if it wasn't asleep

  // for when reports stop without any error. Only sends the soft reset. The reports come back on
  // in update() (library) or in the background from service() (own driver)
  ChipSatPlatform::Status resetHub();

  bool ready() const { return ready_; }
  bool sleeping() const { return sleeping_; }
  const ChipSatSensors::IMUData &data() const { return data_; }

  const uint32_t *eventCounts() const { return events_; }   // reports received since boot

private:
  bool configureReports(uint16_t startupDelayMs);
  void restartStep();   // own driver only
  void lose(bool resetSent);   // own driver only
  bool update();
  void invalidate();
  void updateOverallValidity();

  ChipSatPlatform::I2cBus &bus_;
  ChipSatPlatform::System &system_;
  ChipSatSensors::IMUData data_;
  uint16_t reportIntervalMs_ = 50;
  bool ready_ = false;
  bool sleeping_ = false;

  uint8_t missing_ = 0;
  uint8_t tries_[4] = {0, 0, 0, 0};
  uint32_t events_[4] = {0, 0, 0, 0};
  bool firstResetSeen_ = false;
  uint8_t emptyWaits_ = 0;
  bool refusedLogged_ = false;
};

} // namespace ChipSatDevices
