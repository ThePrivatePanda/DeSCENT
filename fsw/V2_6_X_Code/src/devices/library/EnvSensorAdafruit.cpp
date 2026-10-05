#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_ENV_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include <math.h>
#include <Adafruit_BME280.h>
#include "../EnvSensor.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

static Adafruit_BME280 bme;

EnvSensor::EnvSensor(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system)
  : bus_(bus), system_(system)
{
}

Status EnvSensor::begin()
{
  LOG_I(Env, "start");
  const uint32_t startMs = system_.nowMs();

  for (uint8_t attempt = 0; attempt < kInitAttempts; ++attempt) {
    if (bme.begin(kEnvAddress, &bus_.arduinoWire())) {
      ready_ = true;
      LOG_I(Env, "init") {
        line.field("ok", true);
        line.field("attempts", attempt + 1);
        line.field("ms", system_.nowMs() - startMs);
      }
      return Status::Ok;
    }

    system_.waitMs(kInitRetryDelayMs);
  }

  ready_ = false;
  LOG_E(Env, "init") {
    line.field("ok", false);
    line.field("attempts", kInitAttempts);
    line.field("retry", "never");
  }
  return Status::NoAck;
}

// library runs the chip in normal mode
Status EnvSensor::startMeasurement()
{
  return ready_ ? Status::Ok : Status::NotReady;
}

Status EnvSensor::read()
{
  if (!ready_) {
    data_.valid = false;
    LOG_W(Env, "refused") {
      line.field("call", "read");
      line.field("ready", false);
    }
    return Status::NotReady;
  }

  // Each call reads the sensor again
  const float temperatureC = bme.readTemperature();
  const float pressurePa = bme.readPressure();
  const float humidityPercent = bme.readHumidity();
  const float altitudeM = bme.readAltitude(kSeaLevelPressureHpa);

  const bool readingsValid =
    isfinite(temperatureC) && isfinite(pressurePa) && isfinite(humidityPercent) && isfinite(altitudeM);

  if (!readingsValid) {
    data_.valid = false;
    LOG_W(Env, "read") {
      line.field("ok", false);
      line.field("reason", "nonfinite");
    }
    return Status::BadData;
  }

  data_.temperatureC = temperatureC;
  data_.pressurePa = pressurePa;
  data_.humidityPercent = humidityPercent;
  data_.altitudeM = altitudeM;
  data_.updatedMs = system_.nowMs();
  data_.valid = true;
  return Status::Ok;
}

} // namespace ChipSatDevices

#endif
