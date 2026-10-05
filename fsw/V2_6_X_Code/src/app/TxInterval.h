#pragma once

#include <stdint.h>

namespace ChipSatApp
{

// Next TX interval from battery %: above 75 10 s, 50-75 30 s, 35-50 60 s, 20-35 120 s, below 20 (or NaN) 180 s
uint32_t txIntervalFromSoc(float socPercent);

} // namespace ChipSatApp
