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

// A block holding the given values, with a CRC that is right unless told otherwise. The air
// transmits at intervalMs; the log slots are flat like the build's, every table off
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
  const uint32_t fixed[kRateSlots] = {60000, intervalMs, 1000, 50};
  static const uint8_t pct[4] = {75, 50, 35, 20};
  static const uint32_t ms[5] = {10000, 30000, 60000, 120000, 180000};
  for (size_t p = 0; p < kRateSlots; p++) {
    uint8_t* r = rateAt(out, p);
    put32(r + kRateFixed, fixed[p]);
    r[kRateFromSoc] = 0;
    for (size_t i = 0; i < 4; i++) r[kRatePercent + i] = pct[i];
    for (size_t i = 0; i < 5; i++) put32(r + kRateInterval + 4 * i, p < kPhases ? ms[i] : fixed[p]);
  }
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
        got.targetDbm == 15 && got.setBy == kSetByTool && got.transmit[1].fixedMs == 500,
        "every field comes back, interval little endian");

  build(block, 12, kModuleHp, 9, 20, 0);
  check(run(block, &got, &why) == Parse::kGood && got.transmit[1].fixedMs == 0,
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
  build(block, 9, kModuleLe, 12, 15, 1000, 5);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "a layout this firmware does not know is refused, with a reason");
  for (uint8_t old : {uint8_t(1), uint8_t(2), uint8_t(3)}) {
    build(block, 9, kModuleLe, 12, 15, 1000, old);
    if (run(block, &got, &why) != Parse::kRejected || why == nullptr) {
      printf("FAILED layout %u was accepted\n", old);
      failures++;
    }
  }
  check(true, "an older layout is refused too: slots 2 and 3 held transmit gaps there");

  // layout 4: the log has rates of its own
  build(block, 9, kModuleLe, 12, 15);
  check(run(block, &got, &why) == Parse::kGood && got.log[0].fixedMs == 1000 && got.log[1].fixedMs == 50 &&
        !got.log[0].fromSoc && !got.log[1].fromSoc && got.log[1].socIntervalMs[4] == 50 &&
        got.log[0].socPercent[3] == 20,
        "both log rates are read");

  // 1 to 19 ms is refused in every log value, band or fixed, used or not
  int refused = 0, asked = 0;
  for (size_t slot : {size_t(2), size_t(3)}) {
    for (int cell = -1; cell < 5; cell++) {
      for (bool follows : {false, true}) {
        for (uint32_t bad : {uint32_t(1), uint32_t(19)}) {
          build(block, 9, kModuleLe, 12, 15);
          uint8_t* r = rateAt(block, slot);
          r[kRateFromSoc] = follows ? 1 : 0;
          put32(r + (cell < 0 ? kRateFixed : kRateInterval + 4 * cell), bad);
          sign(block);
          asked++;
          refused += run(block, &got, &why) == Parse::kRejected && why != nullptr ? 1 : 0;
        }
      }
    }
  }
  check(refused == asked, "1 and 19 ms are refused in every log cell, with or without the battery");
  build(block, 9, kModuleLe, 12, 15);
  { uint8_t* r = rateAt(block, 3);
    put32(r + kRateFixed, 0);
    put32(r + kRateInterval, 0);
    put32(r + kRateInterval + 4, 20);
    put32(rateAt(block, 2) + kRateInterval + 16, 0); }
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.log[1].fixedMs == 0 && got.log[1].socIntervalMs[0] == 0 &&
        got.log[1].socIntervalMs[1] == 20 && got.log[0].socIntervalMs[4] == 0,
        "0 and 20 ms are allowed in a log cell");
  build(block, 9, kModuleLe, 12, 15);
  put32(rateAt(block, 0) + kRateInterval, 7);
  put32(rateAt(block, 1) + kRateFixed, 19);
  sign(block);
  check(run(block, &got, &why) == Parse::kGood, "1 to 19 ms is still fine for transmit: it never was refused");

  // The bytes layout 3 kept log gaps in mean nothing now
  build(block, 9, kModuleLe, 12, 15);
  check(block[kOffReserved] == 0 && block[kOffCrc - 1] == 0 && kOffReserved + 20 == kOffCrc, "138 to 157 are reserved zero");

  // the same block settings.rs reads, offsets copied by hand between the two
  static const uint8_t golden[kBlockBytes] = {
#include "data/settings_layout4.inc"
  };
  check(run(golden, &got, &why) == Parse::kGood, "the golden layout 4 block is accepted");
  const Rate& gt = got.transmit[0];
  const Rate& at = got.transmit[1];
  const Rate& gl = got.log[0];
  const Rate& al = got.log[1];
  check(got.csid == 7 && got.module == kModuleHp && got.spreadingFactor == 9 && got.targetDbm == 20 && got.setBy == kSetByTool,
        "golden: the head");
  check(gt.fixedMs == 60000 && gt.fromSoc && gt.socPercent[0] == 80 && gt.socPercent[3] == 20 &&
        gt.socIntervalMs[0] == 11000 && gt.socIntervalMs[4] == 51000, "golden: ground transmit");
  check(at.fixedMs == 1000 && !at.fromSoc && at.socPercent[0] == 75 && at.socPercent[3] == 20 &&
        at.socIntervalMs[0] == 1100 && at.socIntervalMs[4] == 5100, "golden: air transmit");
  check(gl.fixedMs == 10000 && gl.fromSoc && gl.socPercent[1] == 60 && gl.socIntervalMs[0] == 12000 &&
        gl.socIntervalMs[3] == 42000 && gl.socIntervalMs[4] == 0, "golden: ground log");
  check(al.fixedMs == 50 && !al.fromSoc && al.socPercent[0] == 70 && al.socPercent[3] == 10 && al.socIntervalMs[0] == 50 &&
        al.socIntervalMs[1] == 100 && al.socIntervalMs[2] == 0 && al.socIntervalMs[3] == 20 && al.socIntervalMs[4] == 5000,
        "golden: air log");

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
  check(kMagic[2] == 'C' && kMagic[11] == 'G' && kBlockBytes == 160 && kOffCrc == 158 && kLayoutVersion == 4 &&
        kRateSlots == 4 && kOffRates + kRateBytes * kRateSlots == kOffReserved,
        "the layout is the one the dashboard was given");

  // a rate per slot
  build(block, 9, kModuleLe, 12, 15);
  check(run(block, &got, &why) == Parse::kGood && got.transmit[0].fixedMs == 60000 && got.transmit[1].fixedMs == 1000 &&
        !got.transmit[0].fromSoc && !got.transmit[1].fromSoc && got.transmit[1].socPercent[0] == 75 &&
        got.transmit[0].socIntervalMs[4] == 180000,
        "both transmit rates come back with their fixed gap, off, and the build's table");

  build(block, 9, kModuleLe, 12, 15);
  { uint8_t* r = rateAt(block, 3);   // the air log follows its own table
    r[kRateFromSoc] = 1;
    r[kRatePercent + 0] = 90; r[kRatePercent + 1] = 60; r[kRatePercent + 2] = 30; r[kRatePercent + 3] = 5;
    put32(r + kRateInterval + 4 * 4, 200000);
    put32(rateAt(block, 0) + kRateFixed, 300000); }
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.log[1].fromSoc && !got.transmit[1].fromSoc &&
        got.log[1].socPercent[0] == 90 && got.log[1].socPercent[3] == 5 && got.log[1].socIntervalMs[4] == 200000 &&
        got.transmit[1].socIntervalMs[4] == 180000 && got.log[0].socPercent[0] == 75 && got.transmit[0].fixedMs == 300000,
        "one slot's table is its own: the others keep theirs, and transmit and log are independent");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 1)[kRateFromSoc] = 2;
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected, "a battery flag that is not 0 or 1 is refused");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 2)[kRatePercent + 1] = 75;   // equal to the one above it, a log slot
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected && why != nullptr,
        "thresholds that do not fall are refused, even with the table off");

  build(block, 9, kModuleLe, 12, 15);
  rateAt(block, 0)[kRatePercent + 2] = 55;   // rises above the one before it
  sign(block);
  check(run(block, &got, &why) == Parse::kRejected, "a threshold that rises is refused, in any slot");

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
  for (size_t i = 0; i < 5; i++) put32(rateAt(block, 1) + kRateInterval + 4 * i, 0);
  sign(block);
  check(run(block, &got, &why) == Parse::kGood && got.transmit[1].socIntervalMs[2] == 0,
        "an interval of zero in a table is allowed, as it is for a fixed gap");

  printf("\n%s: %d failed\n", failures == 0 ? "all ok" : "FAILURES", failures);
  return failures == 0 ? 0 : 1;
}
