#include "TxInterval.h"

namespace ChipSatApp
{

uint32_t txIntervalFromSoc(const ChipSatSettings::Rate &rate, float socPercent)
{
  for (size_t i = 0; i < ChipSatSettings::kSocBands - 1; i++) {
    if (socPercent >= static_cast<float>(rate.socPercent[i])) {
      return rate.socIntervalMs[i];
    }
  }
  return rate.socIntervalMs[ChipSatSettings::kSocBands - 1];
}

} // namespace ChipSatApp
