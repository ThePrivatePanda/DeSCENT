// Prints what a fresh image ships in its settings block, as JSON, from config.h and Settings.h: the
// layout, then one object per rate slot in the block's order, each with its fixed gap, whether it
// follows the battery and its battery table. The ground UI prefills from this instead of keeping
// its own copy of the numbers.
//   g++ -std=gnu++17 -Itest/mock -I../V2_6_X_Code/src test/emit_battery_json.cpp -o /tmp/emit && /tmp/emit
#include <cstdio>
#include "config.h"
#include "Settings.h"

using namespace ChipSatConfig;
using ChipSatSettings::kPhases;

static void slot(const char *name, uint32_t fixedMs, bool follow, bool flat, bool last)
{
  printf("\n    {\"name\": \"%s\", \"fixedMs\": %lu, \"follow\": %s, \"percent\": [", name,
         static_cast<unsigned long>(fixedMs), follow ? "true" : "false");
  for (size_t i = 0; i < ChipSatSettings::kSocBands - 1; i++) {
    printf("%s%u", i ? ", " : "", kSocPercentDefault[i]);
  }
  printf("], \"intervalMs\": [");
  for (size_t i = 0; i < ChipSatSettings::kSocBands; i++) {
    printf("%s%lu", i ? ", " : "", static_cast<unsigned long>(flat ? fixedMs : kSocIntervalMsDefault[i]));
  }
  printf("]}%s", last ? "" : ",");
}

int main()
{
  static const char *transmit[kPhases] = {"ground transmit", "air transmit"};
  static const char *log[kPhases] = {"ground log", "air log"};
  printf("  \"layoutVersion\": %u,\n  \"slots\": [", ChipSatSettings::kLayoutVersion);
  for (size_t p = 0; p < kPhases; p++) {
    slot(transmit[p], kTransmitFixedMsDefault[p], kRateFromSocDefault[p], false, false);
  }
  for (size_t p = 0; p < kPhases; p++) {
    slot(log[p], kLogFixedMsDefault[p], kRateFromSocDefault[p], true, p + 1 == kPhases);
  }
  printf("\n  ]");
  return 0;
}
