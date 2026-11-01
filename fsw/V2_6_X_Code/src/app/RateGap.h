#pragma once

#include <stdint.h>
#include "../Settings.h"

namespace ChipSatApp
{

// The gap a rate gives right now, transmit or log alike. The fixed gap unless it follows the battery
// and the gauge reads; then socIntervalMs[i] for the first threshold socPercent[i] the charge is at
// or above, and the last band below them all. A charge that is not a number is treated as empty, so
// a gauge talking nonsense gets the slowest band rather than the fastest. "At or above" on every
// threshold, so a charge of exactly 75 % is in the top band
uint32_t gapFromRate(const ChipSatSettings::Rate &rate, bool socOkay, float socPercent);

} // namespace ChipSatApp
