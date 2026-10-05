#include "Landing.h"
#include <math.h>
#include "../config.h"

namespace ChipSatApp
{

using namespace ChipSatConfig;

void Landing::update(uint32_t nowMs, float accelerationMps2, float pressureHpa, bool valid)
{
  if (!valid || !isfinite(accelerationMps2) || !isfinite(pressureHpa)) {
    return;   // nothing to judge on, keep whatever we decided last
  }

  const bool still = accelerationMps2 < kLandingAccelMps2;
  const bool levelled = haveReference_ && fabsf(pressureHpa - referencePressureHpa_) < kLandingPressureHpa;

  if (!still || !levelled) {
    // moving again, or still falling
    haveReference_ = true;
    referencePressureHpa_ = pressureHpa;
    quietSinceMs_ = nowMs;
    quietMs_ = 0;
    landed_ = false;
    return;
  }

  quietMs_ = nowMs - quietSinceMs_;
  if (quietMs_ >= kLandingQuietMs && nowMs >= kLandingEarliestMs) {
    landed_ = true;
  }
}

} // namespace ChipSatApp
