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

constexpr void put32(uint8_t* at, uint32_t v)
{
  at[0] = v & 0xFF;
  at[1] = (v >> 8) & 0xFF;
  at[2] = (v >> 16) & 0xFF;
  at[3] = (v >> 24) & 0xFF;
}

constexpr uint32_t get32(const uint8_t* at)
{
  return static_cast<uint32_t>(at[0]) | static_cast<uint32_t>(at[1]) << 8 |
         static_cast<uint32_t>(at[2]) << 16 | static_cast<uint32_t>(at[3]) << 24;
}

// What a fresh image carries in one slot. A log rate is flat, its bands the fixed gap, so it is the
// same with the table on or off
constexpr Rate defaultRate(uint32_t fixedMs, bool fromSoc, bool flat)
{
  Rate out = {};
  out.fixedMs = fixedMs;
  out.fromSoc = fromSoc;
  for (size_t i = 0; i < kSocBands - 1; i++) {
    out.socPercent[i] = kSocPercentDefault[i];
  }
  for (size_t i = 0; i < kSocBands; i++) {
    out.socIntervalMs[i] = flat ? fixedMs : kSocIntervalMsDefault[i];
  }
  return out;
}

constexpr Rate defaultTransmit(size_t phase)
{
  return defaultRate(kTransmitFixedMsDefault[phase], kRateFromSocDefault[phase], false);
}

constexpr Rate defaultLog(size_t phase)
{
  return defaultRate(kLogFixedMsDefault[phase], kRateFromSocDefault[phase], true);
}

constexpr void putRate(uint8_t* r, const Rate& rate)
{
  put32(r + kRateFixed, rate.fixedMs);
  r[kRateFromSoc] = rate.fromSoc ? 1 : 0;
  for (size_t i = 0; i < kSocBands - 1; i++) {
    r[kRatePercent + i] = rate.socPercent[i];
  }
  for (size_t i = 0; i < kSocBands; i++) {
    put32(r + kRateInterval + 4 * i, rate.socIntervalMs[i]);
  }
}

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
  for (size_t p = 0; p < kPhases; p++) {
    putRate(out.b + kOffRates + kRateBytes * p, defaultTransmit(p));
    putRate(out.b + kOffRates + kRateBytes * (kPhases + p), defaultLog(p));
  }
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
  for (size_t p = 0; p < kPhases; p++) {
    out.transmit[p] = defaultTransmit(p);
    out.log[p] = defaultLog(p);
  }
  return out;
}

Settings active_ = compiledDefaults();
Source source_ = Source::kDefaults;
const char* reject_ = nullptr;

}  // namespace

static bool logGapTooFast(uint32_t ms)
{
  return ms != 0 && ms < kLogMinMs;
}

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
  // Each table is read top down and the first threshold the charge reaches wins, so the thresholds
  // have to fall. 0 and 100 are refused because a band nothing can reach is a number somebody got
  // wrong. Checked whether or not the slot follows the battery, so a table that is off is still one
  // that could be switched on
  const Rate* const rates[kRateSlots] = {&settings.transmit[0], &settings.transmit[1], &settings.log[0], &settings.log[1]};
  for (const Rate* rate : rates) {
    for (size_t i = 0; i < kSocBands - 1; i++) {
      if (rate->socPercent[i] < 1 || rate->socPercent[i] > 99) {
        *whyNot = "a battery threshold outside 1 to 99 percent";
        return false;
      }
      if (i > 0 && rate->socPercent[i] >= rate->socPercent[i - 1]) {
        *whyNot = "battery thresholds that do not fall from one band to the next";
        return false;
      }
    }
  }
  // The flash log cannot take a record faster than the sensors make one: 0 is a record per IMU
  // reading, and anything between that and the floor is a number somebody got wrong. Every value,
  // because a band that is off today gets switched on by a flag
  for (size_t p = 0; p < kPhases; p++) {
    bool tooFast = logGapTooFast(settings.log[p].fixedMs);
    for (size_t i = 0; i < kSocBands; i++) {
      tooFast = tooFast || logGapTooFast(settings.log[p].socIntervalMs[i]);
    }
    if (tooFast) {
      *whyNot = "a flash log gap of 1 to 19 ms: 0 is every reading";
      return false;
    }
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
  Settings wanted;
  wanted.csid = block[kOffCsid];
  wanted.module = block[kOffModule];
  wanted.spreadingFactor = block[kOffSf];
  wanted.targetDbm = static_cast<int8_t>(block[kOffDbm]);
  wanted.setBy = block[kOffSetBy];
  for (size_t slot = 0; slot < kRateSlots; slot++) {
    const uint8_t* r = block + kOffRates + kRateBytes * slot;
    Rate& rate = slot < kPhases ? wanted.transmit[slot] : wanted.log[slot - kPhases];
    rate.fixedMs = get32(r + kRateFixed);
    if (r[kRateFromSoc] > 1) {
      *whyNot = "a battery flag that is neither 0 nor 1";
      return Parse::kRejected;
    }
    rate.fromSoc = r[kRateFromSoc] == 1;
    for (size_t i = 0; i < kSocBands - 1; i++) {
      rate.socPercent[i] = r[kRatePercent + i];
    }
    for (size_t i = 0; i < kSocBands; i++) {
      rate.socIntervalMs[i] = get32(r + kRateInterval + 4 * i);
    }
  }

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
