#include "Settings.h"

#include "config.h"
#include "app/Telemetry.h"

using ChipSatTelemetry::calculateCRC16;
using namespace ChipSatConfig;

namespace ChipSatSettings
{

namespace
{

// The block itself, in the image. const so it sits in flash with the code, volatile so the compiler
// reads it rather than folding in the values it can see here - without that, patching the bytes
// would change nothing, because every use would already have been replaced by the initialiser
//
// The CRC bytes are zero, which is not the CRC of this content, so a freshly built image is "no
// settings written" and the firmware runs the compiled defaults. A tool fills in the four values and
// the real CRC
// The block itself, in the image. const so it sits in flash with the code, volatile so the compiler
// reads it rather than folding in the values it can see here - without that, patching the bytes
// would change nothing, because every use would already have been replaced by the initialiser.
//
// Built by a constexpr function so the CRC is computed over exactly these bytes while compiling.
// A freshly built image therefore holds a VALID block saying setBy = build: the values the firmware
// is really using, signed by a CRC that checks out. That matters on the ground - a bad CRC then means
// the block was damaged, and never means "nobody wrote it yet", so those two can be told apart
struct BlockBytes
{
  uint8_t b[kBlockBytes];
};

constexpr BlockBytes buildBlock()
{
  BlockBytes out = {};
  for (size_t i = 0; i < sizeof kMagic; i++) {
    out.b[i] = kMagic[i];
  }
  out.b[kOffLayout] = kLayoutVersion;
  out.b[kOffCsid] = CHIPSAT_ID;
  out.b[kOffModule] = CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_HP ? kModuleHp : kModuleLe;
  out.b[kOffSf] = CHIPSAT_SPREADING_FACTOR;
  out.b[kOffDbm] = static_cast<uint8_t>(CHIPSAT_TARGET_DBM);
  out.b[kOffSetBy] = kSetByBuild;
  out.b[kOffTxInterval + 0] = (kTxIntervalMs      ) & 0xFF;
  out.b[kOffTxInterval + 1] = (kTxIntervalMs >>  8) & 0xFF;
  out.b[kOffTxInterval + 2] = (kTxIntervalMs >> 16) & 0xFF;
  out.b[kOffTxInterval + 3] = (kTxIntervalMs >> 24) & 0xFF;
  const uint16_t crc = calculateCRC16(out.b, kOffCrc);
  out.b[kOffCrc] = crc & 0xFF;
  out.b[kOffCrc + 1] = (crc >> 8) & 0xFF;
  return out;
}

const volatile BlockBytes storedBlock = buildBlock();

Settings compiledDefaults()
{
  Settings out;
  out.csid = kChipSatId;
#if CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_HP
  out.module = kModuleHp;
#else
  out.module = kModuleLe;
#endif
  out.spreadingFactor = kRadioSpreadingFactor;
  out.targetDbm = kRadioTargetDbm;
  out.setBy = kSetByBuild;
  out.txIntervalMs = kTxIntervalMs;
  return out;
}

Settings active_ = compiledDefaults();
Source source_ = Source::kDefaults;
const char* reject_ = nullptr;

}  // namespace

bool usable(const Settings& settings, const char** whyNot)
{
  // 0 reads as a board that exists, so it is not allowed even though 255 is - 255 is the value that
  // says nobody configured this, and a board reporting 255 is a board you can go and fix
  if (settings.csid == 0) {
    *whyNot = "board id 0";
    return false;
  }
  if (settings.module != kModuleLe && settings.module != kModuleHp) {
    *whyNot = "radio module is neither LE nor HP";
    return false;
  }
  if (settings.spreadingFactor < 5 || settings.spreadingFactor > 12) {
    *whyNot = "spreading factor outside 5 to 12";
    return false;
  }
  // Each module has rows for its own powers and nothing else. Asking an LE module for 20 dBm used to
  // fail the build; now it has to fail here instead, because the value arrives after the build
  const bool ok = settings.module == kModuleHp
    ? (settings.targetDbm == 14 || settings.targetDbm == 17 ||
       settings.targetDbm == 20 || settings.targetDbm == 22)
    : (settings.targetDbm == 10 || settings.targetDbm == 14 || settings.targetDbm == 15);
  if (!ok) {
    *whyNot = settings.module == kModuleHp ? "HP takes 14, 17, 20 or 22 dBm"
                                           : "LE takes 10, 14 or 15 dBm";
    return false;
  }
  *whyNot = nullptr;
  return true;
}

Parse parse(const uint8_t block[kBlockBytes], Settings* out, const char** whyNot)
{
  *whyNot = nullptr;
  const uint16_t want = static_cast<uint16_t>(block[kOffCrc]) |
                        static_cast<uint16_t>(block[kOffCrc + 1]) << 8;
  if (calculateCRC16(block, kOffCrc) != want) {
    // The build signs the block, so a CRC that does not check out is damage, not an empty slot
    *whyNot = "the settings block is damaged";
    return Parse::kDamaged;
  }
  if (block[kOffLayout] != kLayoutVersion) {
    *whyNot = "the block is a layout this firmware does not know";
    return Parse::kRejected;
  }
  // A reserved byte that is not zero, with a CRC that checks out, means something wrote a field this
  // firmware cannot see. Reading the rest and ignoring that field would be guessing
  for (size_t i = kOffReserved; i < kOffCrc; i++) {
    if (block[i] != 0) {
      *whyNot = "the block holds a field this firmware does not know";
      return Parse::kRejected;
    }
  }

  Settings wanted;
  wanted.csid = block[kOffCsid];
  wanted.module = block[kOffModule];
  wanted.spreadingFactor = block[kOffSf];
  wanted.targetDbm = static_cast<int8_t>(block[kOffDbm]);
  wanted.setBy = block[kOffSetBy];
  wanted.txIntervalMs = static_cast<uint32_t>(block[kOffTxInterval]) |
                        static_cast<uint32_t>(block[kOffTxInterval + 1]) << 8 |
                        static_cast<uint32_t>(block[kOffTxInterval + 2]) << 16 |
                        static_cast<uint32_t>(block[kOffTxInterval + 3]) << 24;

  if (!usable(wanted, whyNot)) {
    return Parse::kRejected;
  }

  *out = wanted;
  return Parse::kGood;
}

void begin()
{
  uint8_t block[kBlockBytes];
  for (size_t i = 0; i < kBlockBytes; i++) {
    block[i] = storedBlock.b[i];
  }

  Settings wanted;
  const char* whyNot = nullptr;
  if (parse(block, &wanted, &whyNot) == Parse::kGood) {
    active_ = wanted;
    source_ = Source::kBlock;
    reject_ = nullptr;
    return;
  }
  // Anything short of a good block leaves the compiled values standing and says why, because with a
  // build-signed block there is no longer an innocent reason for one to be unreadable
  reject_ = whyNot;
}

const Settings& active()
{
  return active_;
}

Source source()
{
  return source_;
}

const char* rejectReason()
{
  return reject_;
}

}  // namespace ChipSatSettings
