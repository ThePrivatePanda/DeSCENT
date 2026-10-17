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

static void sign(uint8_t out[kBlockBytes], bool goodCrc);

static void put32(uint8_t* at, uint32_t v)
{
  for (size_t b = 0; b < 4; b++) at[b] = (v >> (8 * b)) & 0xFF;
}

// One phase's rate at its place in the block
static uint8_t* rateAt(uint8_t block[kBlockBytes], size_t phase)
{
  return block + kOffRates + kRateBytes * phase;
}

// A block holding the given values, with a CRC that is right unless told otherwise. The falling
// phase gets intervalMs as its fixed gap; every phase gets the table the build ships, off
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
  static const uint32_t fixed[4] = {60000, 10000, 0, 60000};
  static const uint8_t pct[4] = {75, 50, 35, 20};
  static const uint32_t ms[5] = {10000, 30000, 60000, 120000, 180000};
  for (size_t p = 0; p < kPhases; p++) {
    uint8_t* r = rateAt(out, p);
    put32(r + kRateFixed, p == 2 ? intervalMs : fixed[p]);
    r[kRateFromSoc] = 0;
    for (size_t i = 0; i < 4; i++) r[kRatePercent + i] = pct[i];
    for (size_t i = 0; i < 5; i++) put32(r + kRateInterval + 4 * i, ms[i]);
  }
  static const uint32_t logs[4] = {1000, 50, 50, 1000};
  for (size_t p = 0; p < kPhases; p++) put32(out + kOffLogs + 4 * p, logs[p]);
  put32(out + kOffLogBlind, 200);
  sign(out, goodCrc);
}

static void sign(uint8_t out[kBlockBytes], bool goodCrc = true)
{
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
        got.targetDbm == 15 && got.setBy == kSetByTool && got.rate[2].fixedMs == 500,
        "every field comes back, interval little endian");

  build(block, 12, kModuleHp, 9, 20, 0);
  check(run(block, &got, &why) == Parse::kGood && got.rate[2].fixedMs == 0,
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
  build(block, 9, kModuleLe, 12, 15, 1000, 4);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "a layout this firmware does not know is refused, with a reason");
  build(block, 9, kModuleLe, 12, 15, 1000, 2);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "layout 2, from before the log gaps, is refused too: its reserved bytes are fields now");

  // layout 3: the flash log gaps
  build(block, 9, kModuleLe, 12, 15);
  check(run(block, &got, &why) == Parse::kGood && got.logMs[0] == 1000 && got.logMs[1] == 50 &&
        got.logMs[3] == 1000 && got.logBlindMs == 200,
        "the log gap of every phase and the blind one are read");
  build(block, 9, kModuleLe, 12, 15);
  put32(block + kOffLogs + 4, 10);
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "a log gap under 20 ms is refused: the sensors make nothing that fast");
  build(block, 9, kModuleLe, 12, 15);
  put32(block + kOffLogBlind, 0);
  put32(block + kOffLogs + 4, 0);
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.logMs[1] == 0 && got.logBlindMs == 0,
        "0 is allowed and means a record per IMU reading");
  build(block, 9, kModuleLe, 12, 15);
  put32(block + kOffLogBlind, 19);
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected, "19 ms is refused: between 0 and the floor is a mistake");

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
  check(kMagic[2] == 'C' && kMagic[11] == 'G' && kBlockBytes == 160 && kOffCrc == 158 && kLayoutVersion == 3 &&
        kOffRates + kRateBytes * kPhases == kOffLogs && kOffLogs + 4 * kPhases == kOffLogBlind &&
        kOffLogBlind + 4 == kOffCrc,
        "the layout is the one the dashboard was given");

  // layout 2: a rate per phase
  build(block, 9, kModuleLe, 12, 15);
  check(run(block, &got, &why) == Parse::kGood && got.rate[0].fixedMs == 60000 && got.rate[1].fixedMs == 10000 &&
        got.rate[2].fixedMs == 1000 && got.rate[3].fixedMs == 60000 &&
        !got.rate[0].fromSoc && !got.rate[3].fromSoc && got.rate[1].socPercent[0] == 75 &&
        got.rate[3].socPercent[3] == 20 && got.rate[2].socIntervalMs[4] == 180000,
        "every phase comes back with its fixed gap, off, and the build's table");

  build(block, 9, kModuleLe, 12, 15);
  { uint8_t* r = rateAt(block, 3);   // landed follows its own table
    r[kRateFromSoc] = 1;
    r[kRatePercent + 0] = 90; r[kRatePercent + 1] = 60; r[kRatePercent + 2] = 30; r[kRatePercent + 3] = 5;
    put32(r + kRateInterval + 4 * 4, 200000);
    put32(rateAt(block, 0) + kRateFixed, 300000); }
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.rate[3].fromSoc && !got.rate[2].fromSoc &&
        got.rate[3].socPercent[0] == 90 && got.rate[3].socPercent[3] == 5 && got.rate[3].socIntervalMs[4] == 200000 &&
        got.rate[2].socIntervalMs[4] == 180000 && got.rate[0].fixedMs == 300000,
        "one phase's table is its own: the others keep theirs");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 1)[kRateFromSoc] = 2;
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected, "a battery flag that is not 0 or 1 is refused");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 2)[kRatePercent + 1] = 75;   // equal to the one above it
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "thresholds that do not fall are refused, even with the table off");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 0)[kRatePercent + 2] = 55;   // rises above the one before it
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected, "a threshold that rises is refused, in any phase");

  for (uint8_t bad : {uint8_t(0), uint8_t(100), uint8_t(255)}) {
    build(block, 9, kModuleLe, 12, 15);
    uint8_t* r = rateAt(block, 3);
    if (bad == 0) r[kRatePercent + 3] = 0; else r[kRatePercent + 0] = bad;
    sign(block);
    if (run(block, &got, &why) != Parse::kRejected) {
      printf("FAILED threshold %u was accepted\n", bad);
      failures++;
    }
  }
  check(true, "thresholds outside 1 to 99 are refused");

  build(block, 9, kModuleLe, 12, 15);
  for (size_t i = 0; i < 5; i++) put32(rateAt(block, 2) + kRateInterval + 4 * i, 0);
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.rate[2].socIntervalMs[2] == 0,
        "an interval of zero in a table is allowed, as it is for a fixed gap");

  printf("\n%s: %d failed\n", failures == 0 ? "all ok" : "FAILURES", failures);
  return failures == 0 ? 0 : 1;
}
