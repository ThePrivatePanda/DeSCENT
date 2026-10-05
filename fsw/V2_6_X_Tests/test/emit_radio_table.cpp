// Prints what the firmware will actually accept, as JSON, from the real headers. The ground UI reads
// this instead of keeping its own copy of the table: the LE/HP power rows exist once, in constants.h.
// Built once per module, because the rows are behind the module #if.
//   g++ -std=gnu++17 -Itest/mock -I../V2_6_X_Code/src -DCHIPSAT_RADIO_MODULE=1 \
//     test/emit_radio_table.cpp -o /tmp/emit && /tmp/emit
#include <cstdio>
#include "constants.h"

using namespace ChipSatConstants;

int main()
{
  // every power this module has a row for, and the bytes each one programs
  static const int8_t wanted[] = {10, 14, 15, 17, 20, 22};

  printf("  \"%s\": {\n", kRadioModuleName);
  printf("    \"sleepsDuringTx\": %s,\n", kImuSleepsDuringTx ? "true" : "false");
  printf("    \"dbm\": [");
  bool first = true;
  for (int8_t dbm : wanted) {
    const PaSettings pa = kPaDeviceSel == 0x01 ? paSettingsLowPower(dbm) : paSettingsHighPower(dbm);
    // paSettingsLowPower and paSettingsHighPower fall through to their lowest row for a power the
    // module has no entry for, so a value only belongs in the list if it round-trips
    const bool real = (kPaDeviceSel == 0x01) ? (dbm == 10 || dbm == 14 || dbm == 15)
                                             : (dbm == 14 || dbm == 17 || dbm == 20 || dbm == 22);
    if (!real) {
      continue;
    }
    printf("%s\n      {\"dbm\": %d, \"txparams\": %d, \"dutycycle\": %u, \"hpmax\": %u, "
           "\"devicesel\": %u, \"lut\": %u}",
           first ? "" : ",", dbm, pa.txParamsDbm, pa.dutyCycle, pa.hpMax, pa.deviceSel, pa.lut);
    first = false;
  }
  printf("\n    ]\n  }");
  return 0;
}
