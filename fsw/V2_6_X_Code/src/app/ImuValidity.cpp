#include "ImuValidity.h"

namespace ChipSatApp
{

static bool isStale(bool valid, uint32_t updatedMs, uint32_t nowMs, uint32_t staleMs)
{
  return valid && nowMs - updatedMs > staleMs;   // unsigned, fine across the millis wrap
}

uint8_t clearStaleImuReports(ChipSatSensors::IMUData &imu, uint32_t nowMs, uint32_t staleMs)
{
  uint8_t cleared = 0;

  if (isStale(imu.linearAccelerationValid, imu.linearAccelerationUpdatedMs, nowMs, staleMs)) {
    imu.linearAccelerationValid = false;
    cleared |= 1U << 0;
  }
  if (isStale(imu.gyroscopeValid, imu.gyroscopeUpdatedMs, nowMs, staleMs)) {
    imu.gyroscopeValid = false;
    cleared |= 1U << 1;
  }
  if (isStale(imu.magnetometerValid, imu.magnetometerUpdatedMs, nowMs, staleMs)) {
    imu.magnetometerValid = false;
    cleared |= 1U << 2;
  }
  if (isStale(imu.orientationValid, imu.orientationUpdatedMs, nowMs, staleMs)) {
    imu.orientationValid = false;
    cleared |= 1U << 3;
  }

  imu.valid = imu.linearAccelerationValid && imu.gyroscopeValid && imu.magnetometerValid && imu.orientationValid;
  return cleared;
}

uint8_t silentImuReports(const uint32_t *countsNow, const uint32_t *countsBefore)
{
  uint8_t silent = 0;
  for (uint8_t i = 0; i < 4; ++i) {
    if (countsNow[i] == countsBefore[i]) {
      silent |= 1U << i;
    }
  }
  return silent;
}

} // namespace ChipSatApp
