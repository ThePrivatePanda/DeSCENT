#pragma once

#include <stddef.h>
#include <stdint.h>

// The things that differ between one board and the next: which board it is, which radio module is
// on it, the spreading factor and transmit power it should use, how often it transmits and logs, and
// whether those follow the battery. They were compile-time constants, which meant a per-board image per
// board, and the dashboard has no compiler.
//
// So the image carries a 160-byte block at a fixed place, holding those values and a CRC. A tool
// patches the block in a copy of the image and flashes the whole image; the firmware reads the block
// at boot and uses it if the CRC is good, and falls back on the compiled defaults from config.h if
// it is absent, older than this firmware understands, or says something impossible.
//
// A freshly built image carries a VALID block holding the values it was compiled with and setBy =
// build. It is not an empty slot: it is a true statement about what this image does, signed by a CRC.
// That is what lets the ground tell two different things apart - a block that fails its CRC was
// damaged, and a board nobody has configured says so in a field instead of by an absence.
namespace ChipSatSettings
{

constexpr size_t kBlockBytes = 160;

// Bytes 2 to 11 spell CHIPSATCFG so a person can find the block with strings or a hex editor. The
// firmware never compares against this literal - it validates on the CRC - so the image holds
// exactly one copy and a tool scanning for it cannot match the comparison instead of the block
constexpr uint8_t kMagic[16] = {
  0x5A, 0xA5, 0x43, 0x48, 0x49, 0x50, 0x53, 0x41,
  0x54, 0x43, 0x46, 0x47, 0x5A, 0xA5, 0x7B, 0x3C };

// Layout 2 (2026-10) grew the block from 32 to 160 bytes: a transmit rate per flight phase, each
// with its own battery table. Layout 3 (2026-10-16) put the flash log gaps in what layout 2 kept
// reserved. Layout 4 (2026-10-31) makes the log a rate like transmit: ground and air transmit, then
// ground and air log, and the bytes that held the log gaps are reserved again. A reader has to take
// the layout byte first and only then knows where the CRC is: layout 1 had it at byte 30
constexpr uint8_t kLayoutVersion = 4;

constexpr size_t kPhases = 2;                 // ground, air, as in app/Phase.h
constexpr size_t kRateSlots = 2 * kPhases;    // ground transmit, air transmit, ground log, air log
constexpr size_t kSocBands = 5;    // one interval per state-of-charge band: four thresholds between

// byte offsets inside the block, after the magic
constexpr size_t kOffMagic       = 0;
constexpr size_t kOffLayout      = 16;
constexpr size_t kOffCsid        = 17;
constexpr size_t kOffModule      = 18;
constexpr size_t kOffSf          = 19;
constexpr size_t kOffDbm         = 20;
constexpr size_t kOffSetBy       = 21;
// Four rates, 29 bytes each: a fixed gap in ms (uint32), then whether the gap follows the battery
// instead (0/1), four thresholds in percent (falling, each 1 to 99) and five gaps in ms (uint32), one
// per band. The fixed gap is what a rate uses when it does not follow the battery, and what it uses
// while the gauge cannot be read when it does. Each slot stands alone: the dashboard writes the same
// flag and thresholds to a phase's transmit and log slots, this firmware does not care.
// Transmit: 0 sends back to back. Air log: 0 is a record per IMU reading (every 50 ms). Ground log:
// what's kept on the ground, 0 keeps nothing. 1 to 19 is refused in every log value, used or not
constexpr size_t kOffRates       = 22;
constexpr size_t kRateBytes      = 29;
constexpr size_t kRateFixed      = 0;
constexpr size_t kRateFromSoc    = 4;
constexpr size_t kRatePercent    = 5;
constexpr size_t kRateInterval   = 9;
constexpr size_t kOffReserved    = 138;  // zero to the CRC
constexpr uint32_t kLogMinMs     = 20;
constexpr size_t kOffCrc         = 158;  // uint16 over bytes 0 to 157, low byte first

constexpr uint8_t kModuleLe = 1;
constexpr uint8_t kModuleHp = 2;

constexpr uint8_t kSetByBuild = 1;   // nobody has written it; the values are whatever was compiled
constexpr uint8_t kSetByTool  = 2;   // a tool wrote it

// How often something happens: the gap after each packet, or between flash log records
struct Rate
{
  uint32_t fixedMs;
  bool     fromSoc;
  uint8_t  socPercent[kSocBands - 1];
  uint32_t socIntervalMs[kSocBands];
};

struct Settings
{
  uint8_t  csid;
  uint8_t  module;
  uint8_t  spreadingFactor;
  int8_t   targetDbm;
  uint8_t  setBy;
  Rate     transmit[kPhases];   // ground, air
  Rate     log[kPhases];        // ground, air
};

enum class Source : uint8_t
{
  kDefaults,   // compiled into this image
  kBlock,      // read from the block in flash
};

enum class Parse : uint8_t
{
  kDamaged,      // the CRC does not check out. Since the build writes a valid one, this only ever
                 // means the bytes were damaged - never that nobody has configured the board
  kRejected,     // something wrote it, and this firmware cannot transmit with what it says
  kGood,
};

// Reads 160 bytes as a block. Separate from begin() because the one block that matters lives in flash
// and cannot be moved, and a rule nobody can feed bad bytes to is a rule nobody has tested
Parse parse(const uint8_t block[kBlockBytes], Settings* out, const char** whyNot);

// Reads the block once and decides what this board is running. Called before the radio is configured
// and before the first packet is built, because both need the answer
void begin();

// What the board is actually using. Valid after begin(); before that it is the compiled defaults
const Settings& active();

Source source();

// Why the block was not used, for the boot line. nullptr when there was nothing wrong with it, which
// includes the ordinary case of an image nobody has written to yet
const char* rejectReason();

// Whether a set of values is one this firmware can transmit with. Separate from the block so the
// same rules can be tested on a laptop, and so a bad block and a bad default are judged identically
bool usable(const Settings& settings, const char** whyNot);

}  // namespace ChipSatSettings
