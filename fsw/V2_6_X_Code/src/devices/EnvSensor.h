#pragma once

#include <stddef.h>
#include <stdint.h>
#include "../platform/I2cBus.h"
#include "../platform/Status.h"
#include "../platform/System.h"
#include "SensorData.h"

namespace ChipSatDevices
{

// BME280. Which driver is behind this is picked by CHIPSAT_ENV_DRIVER in config.h
class EnvSensor
{
public:
  EnvSensor(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system);

  ChipSatPlatform::Status begin();
  ChipSatPlatform::Status startMeasurement();   // read() collects it, waiting only for what's left
  ChipSatPlatform::Status read();    // a failed or out of range reading leaves the old values, marked invalid

  bool ready() const { return ready_; }
  const ChipSatSensors::EnvironmentalData &data() const { return data_; }

private:
  // only the own driver uses these, the library one leaves them undefined
  ChipSatPlatform::Status readRegisters(uint8_t reg, uint8_t *buffer, size_t length);
  ChipSatPlatform::Status writeRegister(uint8_t reg, uint8_t value);
  ChipSatPlatform::Status waitForStatus(uint8_t mask, uint16_t timeoutMs);
  static uint8_t measControl(uint8_t mode);
  bool start();

  ChipSatPlatform::I2cBus &bus_;
  ChipSatPlatform::System &system_;
  ChipSatSensors::EnvironmentalData data_;
  bool ready_ = false;
  bool measuring_ = false;
  uint32_t measureStartMs_ = 0;
  uint32_t lastStartMs_ = 0;
};

} // namespace ChipSatDevices
