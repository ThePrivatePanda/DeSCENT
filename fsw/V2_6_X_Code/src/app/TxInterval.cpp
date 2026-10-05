#include "TxInterval.h"

namespace ChipSatApp
{

uint32_t txIntervalFromSoc(float socPercent)
{
  if (socPercent > 75.0f) {
    return 10UL * 1000UL;
  } else if (socPercent >= 50.0f) {
    return 30UL * 1000UL;
  } else if (socPercent >= 35.0f) {
    return 60UL * 1000UL;
  } else if (socPercent >= 20.0f) {
    return 120UL * 1000UL;
  } else {
    return 180UL * 1000UL;
  }
}

} // namespace ChipSatApp
