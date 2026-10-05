// The settings block: what is accepted, what is refused, and why. Runs on a laptop.
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -DCHIPSAT_RADIO_MODULE=1 
//     test/test_settings.cpp ../V2_6_X_Code/src/Settings.cpp ../V2_6_X_Code/src/app/Telemetry.cpp \
//     -o /tmp/settings && /tmp/settings
//
// The block that flies cannot be fed bad bytes - it is one const object in flash - so these go
// through parse(), which is the same code reading the same rules.
#include <cstdio>
#include <cstring>

#include "Settings.h"
#include "app/Telemetry.h"

using namespace ChipSatSettings;

static int failures = 0;

static void check(bool ok, const char* what)
{
  printf("%s %s\n", ok ? "ok    " : "FAILED", what);
  failures += ok ? 0 : 1;
}

// A block holding the given values, with a CRC that is right unless told otherwise
static void build(uint8_t out[kBlockBytes], uint8_t csid, uint8_t module, uint8_t sf, int8_t dbm,
                  uint32_t intervalMs = 1000, uint8_t layout = kLayoutVersion, bool goodCrc = true)
{
  memset(out, 0, kBlockBytes);
  memcpy(out, kMagic, sizeof kMagic);
  out[kOffLayout] = layout;
  out[kOffCsid] = csid;
  out[kOffModule] = module;
  out[kOffSf] = sf;
  out[kOffDbm] = static_cast<uint8_t>(dbm);
  out[kOffSetBy] = kSetByTool;
  out[kOffTxInterval + 0] = intervalMs & 0xFF;
  out[kOffTxInterval + 1] = (intervalMs >> 8) & 0xFF;
  out[kOffTxInterval + 2] = (intervalMs >> 16) & 0xFF;
  out[kOffTxInterval + 3] = (intervalMs >> 24) & 0xFF;
  const uint16_t crc = ChipSatTelemetry::calculateCRC16(out, kOffCrc);
  out[kOffCrc] = goodCrc ? (crc & 0xFF) : ((crc & 0xFF) ^ 0x01);
  out[kOffCrc + 1] = (crc >> 8) & 0xFF;
}

static Parse run(const uint8_t block[kBlockBytes], Settings* out, const char** why)
{
  *why = nullptr;
  return parse(block, out, why);
}

int main()
{
  uint8_t block[kBlockBytes];
  Settings got;
  const char* why = nullptr;

  // A freshly built image carries a valid block saying setBy = build. It is a true statement about
  // what the image does, not an empty slot, so the ground can tell damage from "nobody configured it"
  build(block, 255, kModuleLe, 7, 14, 1000);
  block[kOffSetBy] = kSetByBuild;
  const uint16_t rebuilt = ChipSatTelemetry::calculateCRC16(block, kOffCrc);
  block[kOffCrc] = rebuilt & 0xFF;
  block[kOffCrc + 1] = rebuilt >> 8;
  check(run(block, &got, &why) == Parse::kGood && got.setBy == kSetByBuild,
        "a build-signed block is good, and says the build put it there");

  // the ordinary case
  build(block, 9, kModuleLe, 12, 15, 500);
  check(run(block, &got, &why) == Parse::kGood, "a written block is accepted");
  check(got.csid == 9 && got.module == kModuleLe && got.spreadingFactor == 12 &&
        got.targetDbm == 15 && got.setBy == kSetByTool && got.txIntervalMs == 500,
        "every field comes back, interval little endian");

  build(block, 12, kModuleHp, 9, 20, 0);
  check(run(block, &got, &why) == Parse::kGood && got.txIntervalMs == 0,
        "an interval of zero is allowed and means back to back");

  // a damaged block is never half-believed
  build(block, 9, kModuleLe, 12, 15, 500, kLayoutVersion, false);
  check(run(block, &got, &why) == Parse::kDamaged && why != nullptr,
        "a bad CRC is damage, and says so - it can no longer mean nobody wrote it");

  int caught = 0;
  for (size_t i = 0; i < kOffCrc; i++) {
    build(block, 9, kModuleLe, 12, 15);
    block[i] ^= 0x01;              // one bit, anywhere the CRC covers
    caught += run(block, &got, &why) != Parse::kGood;
  }
  check(caught == static_cast<int>(kOffCrc), "one flipped bit anywhere under the CRC is caught");

  // the two states the operator has to be able to tell apart
  build(block, 9, kModuleLe, 12, 15);
  block[kOffSetBy] = kSetByTool;
  { const uint16_t c = ChipSatTelemetry::calculateCRC16(block, kOffCrc);
    block[kOffCrc] = c & 0xFF; block[kOffCrc + 1] = c >> 8; }
  check(run(block, &got, &why) == Parse::kGood && got.setBy == kSetByTool,
        "a tool-written block says a tool put it there");

  // things that are written but cannot be transmitted with
  build(block, 9, kModuleLe, 12, 15, 1000, 2);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "a layout this firmware does not know is refused, with a reason");

  build(block, 9, kModuleLe, 12, 15);
  block[kOffReserved] = 1;
  block[kOffCrc] = ChipSatTelemetry::calculateCRC16(block, kOffCrc) & 0xFF;
  block[kOffCrc + 1] = ChipSatTelemetry::calculateCRC16(block, kOffCrc) >> 8;
  check(run(block, &got, &why) == Parse::kRejected,
        "a reserved byte in use means a field this firmware cannot see, so the block is refused");

  build(block, 0, kModuleLe, 12, 15);
  check(run(block, &got, &why) == Parse::kRejected, "board id 0 is refused");

  build(block, 255, kModuleLe, 7, 14);
  check(run(block, &got, &why) == Parse::kGood, "board id 255 is allowed: it says nobody set it");

  build(block, 9, 3, 12, 15);
  check(run(block, &got, &why) == Parse::kRejected, "a module that is neither LE nor HP is refused");

  for (uint8_t sf : {uint8_t(4), uint8_t(13), uint8_t(0), uint8_t(24)}) {
    build(block, 9, kModuleLe, sf, 15);
    if (run(block, &got, &why) != Parse::kRejected) {
      printf("FAILED spreading factor %u was accepted\n", sf);
      failures++;
    }
  }
  check(true, "spreading factors outside 5 to 12 are refused, 24 among them");

  // the September failure, now a runtime rule instead of a static_assert
  build(block, 9, kModuleLe, 9, 20);
  check(run(block, &got, &why) == Parse::kRejected, "20 dBm on an LE module is refused");
  build(block, 12, kModuleHp, 9, 20);
  check(run(block, &got, &why) == Parse::kGood, "20 dBm on an HP module is accepted");
  build(block, 9, kModuleLe, 9, 15);
  check(run(block, &got, &why) == Parse::kGood, "15 dBm on an LE module is accepted");
  build(block, 12, kModuleHp, 9, 15);
  check(run(block, &got, &why) == Parse::kRejected, "15 dBm on an HP module is refused: no such row");

  // the magic has to be exactly the agreed bytes, or a tool and the firmware disagree about where
  // the block is
  check(kMagic[2] == 'C' && kMagic[11] == 'G' && kBlockBytes == 32 && kOffCrc == 30,
        "the layout is the one the dashboard was given");

  printf("\n%s: %d failed\n", failures == 0 ? "all ok" : "FAILURES", failures);
  return failures == 0 ? 0 : 1;
}
