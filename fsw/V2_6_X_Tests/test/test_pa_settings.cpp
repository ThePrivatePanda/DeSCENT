// The PA row for each module and target power, against DS_SX1261-2 Rev 2.2 Table 13-21 p.82.
// Builds the real config.h and constants.h once per combination, through -D.
//   for m in 1 2; do for d in 10 14 15 17 20 22; do
//     g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src \
//       -DCHIPSAT_RADIO_MODULE=$m -DCHIPSAT_TARGET_DBM=$d test/test_pa_settings.cpp -o /tmp/pa 2>/dev/null \
//       && /tmp/pa; done; done
// A combination the module has no row for fails to compile, which is the point of the static_asserts.
#include <cstdio>
#include "constants.h"

using namespace ChipSatConstants;
using ChipSatConfig::kRadioTargetDbm;

// the table, written out again here so a typo in constants.h can't agree with itself
struct Expected { int8_t dbm; int8_t txParams; uint8_t duty; uint8_t hpMax; uint8_t deviceSel; };

#if CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_LE
constexpr Expected kTable[] = {
  {15, 14, 0x06, 0x00, 0x01},
  {14, 14, 0x04, 0x00, 0x01},
  {10, 13, 0x01, 0x00, 0x01},
};
constexpr const char *kWhich = "LE/SX1261";
#else
constexpr Expected kTable[] = {
  {22, 22, 0x04, 0x07, 0x00},
  {20, 22, 0x03, 0x05, 0x00},
  {17, 22, 0x02, 0x03, 0x00},
  {14, 22, 0x02, 0x02, 0x00},
};
constexpr const char *kWhich = "HP/SX1262";
#endif

int main()
{
  const Expected *want = nullptr;
  for (const Expected &row : kTable) {
    if (row.dbm == kRadioTargetDbm) {
      want = &row;
    }
  }
  if (want == nullptr) {
    printf("FAILED %s %d dBm: not a row in the table, yet it compiled\n", kWhich, kRadioTargetDbm);
    return 1;
  }

  int bad = 0;
  bad += kPaPowerDbm != want->txParams;
  bad += kRadioPowerDbm != want->txParams;   // both SetTxParams writes, last one wins
  bad += kPaDutyCycle != want->duty;
  bad += kPaHpMax != want->hpMax;
  bad += kPaDeviceSel != want->deviceSel;
  bad += kPaLut != 0x01;

  printf("%s %s %2d dBm: txparams %2d duty 0x%02X hpmax 0x%02X devsel 0x%02X\n",
         bad == 0 ? "ok    " : "FAILED", kWhich, kRadioTargetDbm,
         kPaPowerDbm, kPaDutyCycle, kPaHpMax, kPaDeviceSel);
  return bad == 0 ? 0 : 1;
}
