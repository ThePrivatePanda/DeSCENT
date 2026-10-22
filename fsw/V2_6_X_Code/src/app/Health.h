#pragma once

#include <stdint.h>
#include "../devices/EnvSensor.h"
#include "../devices/FuelGauge.h"
#include "../devices/Gps.h"
#include "../devices/Imu.h"
#include "../platform/I2cBus.h"
#include "../platform/System.h"
#include "SensorWatch.h"

namespace ChipSatApp
{

// Per-sensor and bus watchdogs. One step a pass at most, bus first, no sensor step while the bus
// is faulted. GPS goes first on the ground since its position is what finds the sat.
// Worst cost per step, from reading the code, not measured: bus clear <12 ms, baro restart ~70 ms,
// gauge a few ms, IMU and GPS steps only queue work
class Health
{
public:
  Health(ChipSatPlatform::System &system, ChipSatPlatform::I2cBus &bus, ChipSatDevices::Imu &imu,
         ChipSatDevices::Gps &gps, ChipSatDevices::EnvSensor &env, ChipSatDevices::FuelGauge &gauge);

  void begin();   // after the sensors' begin(), limits count from here

  // never from inside the TX wait. air includes the boot-air
  void service(bool air);

  void imuWoke();   // HP: reports get kImuGraceMs to come back

  const SensorWatch &imuWatch() const { return imu_; }
  const SensorWatch &envWatch() const { return env_; }
  const SensorWatch &gaugeWatch() const { return gauge_; }
  const SensorWatch &gpsWatch() const { return gps_; }

private:
  enum class Part : uint8_t { Imu, Env, Gauge, Gps };

  Seen judgeImu(uint32_t nowMs, bool &partial);
  Seen judgeEnv(uint32_t nowMs) const;
  Seen judgeGauge(uint32_t nowMs) const;
  Seen judgeGps(uint32_t nowMs) const;
  void report(Part part, WatchChange change, uint32_t nowMs);
  bool act(Part part, uint32_t nowMs);
  SensorWatch &watch(Part part);
  const char *reason(Part part, uint32_t nowMs) const;

  ChipSatPlatform::System &system_;
  ChipSatPlatform::I2cBus &bus_;
  ChipSatDevices::Imu &imuPart_;
  ChipSatDevices::Gps &gpsPart_;
  ChipSatDevices::EnvSensor &envPart_;
  ChipSatDevices::FuelGauge &gaugePart_;

  SensorWatch imu_;
  SensorWatch env_;
  SensorWatch gauge_;
  SensorWatch gps_;

  uint32_t beganMs_ = 0;
  uint32_t lastActionMs_ = 0;
  bool actedAny_ = false;

  bool imuUp_ = false;
  uint32_t imuUpMs_ = 0;
  uint32_t imuCounts_[4] = {0, 0, 0, 0};
  uint32_t imuMovedMs_[4] = {0, 0, 0, 0};
  uint8_t imuMoved_ = 0;    // since it came up
  uint8_t imuSilent_ = 0;
};

} // namespace ChipSatApp
