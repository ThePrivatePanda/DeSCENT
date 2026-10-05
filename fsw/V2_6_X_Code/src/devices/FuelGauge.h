#pragma once

#include <stdint.h>
#include "../platform/I2cBus.h"
#include "../platform/Status.h"
#include "../platform/System.h"
#include "SensorData.h"

namespace ChipSatDevices
{

// MAX17048. Which driver is behind this is picked by CHIPSAT_GAUGE_DRIVER in config.h
class FuelGauge
{
public:
  FuelGauge(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system);

  ChipSatPlatform::Status begin();
  ChipSatPlatform::Status read();    // clamped to 0-100. A failed read leaves the old value, marked invalid

  bool ready() const { return ready_; }
  const ChipSatSensors::StateOfChargeData &data() const { return data_; }

  float rawPercent() const { return rawPercent_; }   // before the clamp, for logging

private:
  ChipSatPlatform::Status read16(uint8_t reg, uint16_t &value);
  ChipSatPlatform::Status write16(uint8_t reg, uint16_t value);
  bool start();

  ChipSatPlatform::I2cBus &bus_;
  ChipSatPlatform::System &system_;
  ChipSatSensors::StateOfChargeData data_;
  bool ready_ = false;
  float rawPercent_ = 0.0f;
  uint32_t lastStartMs_ = 0;
};

} // namespace ChipSatDevices
