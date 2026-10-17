#pragma once

#include <stdint.h>
#include "../Settings.h"

namespace ChipSatApp
{

// The gap after a packet from a phase's rate and the battery: socIntervalMs[i] for the first
// threshold socPercent[i] the charge is at or above, and the last band below them all. A charge that
// is not a number is treated as empty, so a gauge talking nonsense gets the slowest gap rather than
// the fastest. "At or above" on every threshold, so a charge of exactly 75 % is in the top band
uint32_t txIntervalFromSoc(const ChipSatSettings::Rate &rate, float socPercent);

} // namespace ChipSatApp
