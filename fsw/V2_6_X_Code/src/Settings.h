#pragma once

#include <stddef.h>
#include <stdint.h>

// The four things that differ between one board and the next: which board it is, which radio module
// is on it, and the spreading factor and transmit power it should use. They were compile-time
// constants, which meant a per-board image per board, and the dashboard has no compiler.
//
// So the image carries a 32-byte block at a fixed place, holding those four values and a CRC. A tool
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

constexpr size_t kBlockBytes = 32;

// Bytes 2 to 11 spell CHIPSATCFG so a person can find the block with strings or a hex editor. The
// firmware never compares against this literal - it validates on the CRC - so the image holds
// exactly one copy and a tool scanning for it cannot match the comparison instead of the block
constexpr uint8_t kMagic[16] = {
  0x5A, 0xA5, 0x43, 0x48, 0x49, 0x50, 0x53, 0x41,
  0x54, 0x43, 0x46, 0x47, 0x5A, 0xA5, 0x7B, 0x3C };

constexpr uint8_t kLayoutVersion = 1;

// byte offsets inside the block, after the magic
constexpr size_t kOffMagic       = 0;
constexpr size_t kOffLayout      = 16;
constexpr size_t kOffCsid        = 17;
constexpr size_t kOffModule      = 18;
constexpr size_t kOffSf          = 19;
constexpr size_t kOffDbm         = 20;
constexpr size_t kOffSetBy       = 21;
constexpr size_t kOffTxInterval  = 22;   // uint32, little endian
constexpr size_t kOffReserved    = 26;   // 4 bytes, zero
constexpr size_t kOffCrc         = 30;   // uint16 over bytes 0 to 29, low byte first

constexpr uint8_t kModuleLe = 1;
constexpr uint8_t kModuleHp = 2;

constexpr uint8_t kSetByBuild = 1;   // nobody has written it; the values are whatever was compiled
constexpr uint8_t kSetByTool  = 2;   // a tool wrote it

struct Settings
{
  uint8_t  csid;
  uint8_t  module;
  uint8_t  spreadingFactor;
  int8_t   targetDbm;
  uint8_t  setBy;
  uint32_t txIntervalMs;
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

// Reads 32 bytes as a block. Separate from begin() because the one block that matters lives in flash
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
