#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_GPS_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include <SparkFun_u-blox_GNSS_Arduino_Library.h>
#include "../Gps.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

static SFE_UBLOX_GNSS gnss;

Gps::Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system)
  : port_(port), system_(system)
{
}

Status Gps::begin()
{
  LOG_I(Gps, "start");
  const uint32_t startMs = system_.nowMs();

  for (uint8_t attempt = 0; attempt < kInitAttempts; attempt++) {
    if (gnss.begin(port_.arduinoStream())) {
      // UBX only
      const bool uartOkay = gnss.setUART1Output(COM_TYPE_UBX);

      // Separate bools so every step still runs if one fails
      const bool airborneOkay = configureAirborne();
      const bool normalModeOkay = configureNormalContinuous();

      ready_ = uartOkay && airborneOkay && normalModeOkay;

      if (ready_) {
        LOG_I(Gps, "init") {
          line.field("ok", true);
          line.field("attempts", attempt + 1);
          line.field("uart", uartOkay);
          line.field("dyn", airborneOkay);
          line.field("rate", rateOkay_);
          line.field("nav", navOkay_);
          line.field("mode", modeOkay_);
          line.field("dynmodel", kGpsDynamicModel);
          line.field("ms", system_.nowMs() - startMs);
        }
      } else {
        // No retry, read() refuses from here on so the GPS goes out invalid
        LOG_E(Gps, "init") {
          line.field("ok", false);
          line.field("reason", "config");
          line.field("attempts", attempt + 1);
          line.field("uart", uartOkay);
          line.field("dyn", airborneOkay);
          line.field("rate", rateOkay_);
          line.field("nav", navOkay_);
          line.field("mode", modeOkay_);
          line.field("ms", system_.nowMs() - startMs);
        }
      }
      return ready_ ? Status::Ok : Status::Failed;
    }

    system_.waitMs(kInitRetryDelayMs);
  }

  ready_ = false;
  LOG_E(Gps, "init") {
    line.field("ok", false);
    line.field("reason", "begin");
    line.field("attempts", kInitAttempts);
    line.field("ms", system_.nowMs() - startMs);
  }
  return Status::NoAck;
}

bool Gps::configureNormalContinuous()
{
  // Also clears any power save mode left over from older firmware
  rateOkay_ = gnss.setVal16(kUbxCfgRateMeas, kGpsMeasurementIntervalMs, kUbxConfigLayers);
  navOkay_ = gnss.setVal16(kUbxCfgRateNav, kGpsNavigationRate, kUbxConfigLayers);
  modeOkay_ = gnss.setVal8(kUbxCfgPmOperateMode, kGpsOperateModeFull, kUbxConfigLayers);
  return rateOkay_ && navOkay_ && modeOkay_;
}

bool Gps::configureAirborne()
{
  return gnss.setVal8(kUbxCfgNavspgDynmodel, kGpsDynamicModel, kUbxConfigLayers);
}

// library polls in read()
void Gps::service()
{
}

Status Gps::read()
{
  const uint16_t maxWaitMs = kGpsPvtWaitMs;
  if (!ready_) {
    return Status::NotReady;
  }

  fixType_ = 0;
  llhChecked_ = false;
  invalidLlh_ = false;

  if (!gnss.getPVT(maxWaitMs)) {
    data_.valid = false;   // old coordinates stay
    LOG_W(Gps, "read") {
      line.field("ok", false);
      line.field("reason", "pvt");
    }
    return Status::Timeout;
  }

  // Copied even without a fix
  data_.latitudeE7 = gnss.getLatitude();
  data_.longitudeE7 = gnss.getLongitude();
  data_.altitudeMSLmm = gnss.getAltitudeMSL();
  data_.updatedMs = system_.nowMs();

  // Airborne models don't do 2D fixes ("No 2D position fixes supported", DYN_MODEL_AIRBORNE4g in the
  // u-blox library header)
  fixType_ = gnss.getFixType();
  if (fixType_ >= 3) {
    invalidLlh_ = gnss.getInvalidLlh();
    llhChecked_ = true;
  }
  data_.valid = (fixType_ >= 3) && !invalidLlh_;

  return data_.valid ? Status::Ok : Status::BadData;
}

} // namespace ChipSatDevices

#endif
