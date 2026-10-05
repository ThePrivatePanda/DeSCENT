#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_GAUGE_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include <math.h>
#include <Adafruit_MAX1704X.h>
#include "../FuelGauge.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

static Adafruit_MAX17048 gauge;

FuelGauge::FuelGauge(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system)
  : bus_(bus), system_(system)
{
}

Status FuelGauge::begin()
{
  LOG_I(Soc, "start");
  const uint32_t startMs = system_.nowMs();

  for (uint8_t attempt = 0; attempt < kInitAttempts; ++attempt) {
    if (gauge.begin(&bus_.arduinoWire())) {
      ready_ = true;
      LOG_I(Soc, "init") {
        line.field("ok", true);
        line.field("attempts", attempt + 1);
        line.field("ms", system_.nowMs() - startMs);
      }
      return Status::Ok;
    }

    system_.waitMs(kInitRetryDelayMs);
  }

  ready_ = false;
  LOG_E(Soc, "init") {
    line.field("ok", false);
    line.field("attempts", kInitAttempts);
    line.field("retry", "never");
  }
  return Status::NoAck;
}

Status FuelGauge::read()
{
  if (!ready_) {
    data_.valid = false;
    LOG_W(Soc, "refused") {
      line.field("call", "read");
      line.field("ready", false);
    }
    return Status::NotReady;
  }

  const float percentage = gauge.cellPercent();
  rawPercent_ = percentage;

  if (!isfinite(percentage)) {
    data_.valid = false;
    LOG_W(Soc, "read") {
      line.field("ok", false);
      line.field("reason", "nan");
    }
    return Status::BadData;
  }

  // constrain is a macro that evaluates its args twice, keep it on a local
  data_.cellPercentage = constrain(percentage, 0.0f, 100.0f);
  data_.updatedMs = system_.nowMs();
  data_.valid = true;
  return Status::Ok;
}

} // namespace ChipSatDevices

#endif
