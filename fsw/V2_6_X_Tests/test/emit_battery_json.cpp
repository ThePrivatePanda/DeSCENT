// Prints the transmit rates a fresh image ships in its settings block, as JSON, from config.h: one
// per phase, each with its fixed gap, whether it follows the battery, its battery table and its flash
// log gap, then the blind log gap. The
// ground UI prefills from this instead of keeping its own copy of the numbers.
//   g++ -std=gnu++17 -Itest/mock -I../V2_6_X_Code/src test/emit_battery_json.cpp -o /tmp/emit && /tmp/emit
#include <cstdio>
#include "config.h"
#include "Settings.h"

using namespace ChipSatConfig;

int main()
{
  static const char* names[4] = {"waiting", "rising", "falling", "landed"};
  printf("  \"phases\": [");
  for (size_t p = 0; p < ChipSatSettings::kPhases; p++) {
    printf("%s\n    {\"name\": \"%s\", \"fixedMs\": %lu, \"follow\": %s, \"percent\": [", p ? "," : "", names[p],
           static_cast<unsigned long>(kRateFixedMsDefault[p]), kRateFromSocDefault[p] ? "true" : "false");
    for (size_t i = 0; i < ChipSatSettings::kSocBands - 1; i++) {
      printf("%s%u", i ? ", " : "", kSocPercentDefault[i]);
    }
    printf("], \"intervalMs\": [");
    for (size_t i = 0; i < ChipSatSettings::kSocBands; i++) {
      printf("%s%lu", i ? ", " : "", static_cast<unsigned long>(kSocIntervalMsDefault[i]));
    }
    printf("], \"logMs\": %lu}", static_cast<unsigned long>(kLogIntervalMsDefault[p]));
  }
  printf("\n  ],\n  \"logBlindMs\": %lu", static_cast<unsigned long>(kLogBlindIntervalMs));
  return 0;
}
