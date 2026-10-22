#include "I2cWatch.h"
#include "../config.h"
#include "../constants.h"
#include "../log/Log.h"

namespace ChipSatPlatform
{

using namespace ChipSatConfig;
using namespace ChipSatConstants;

// wrap-safe
static bool earlier(uint32_t a, uint32_t b)
{
  return static_cast<int32_t>(a - b) < 0;
}

static uint32_t doubled(uint32_t ms, uint32_t first, uint32_t most)
{
  if (ms == 0) {
    return first;
  }
  return ms >= most / 2 ? most : ms * 2;
}

const char *busPartName(uint8_t address)
{
  switch (address) {
    case kImuAddress: return "imu";
    case kEnvAddress: return "env";
    case kGaugeAddress: return "soc";
    default: return "other";
  }
}

static bool sclComesUp(BusLines &lines)
{
  for (uint16_t waited = 0; !lines.scl(); ++waited) {
    if (waited >= kI2cStretchWaits) {
      return false;
    }
    lines.halfClock();
  }
  return true;
}

BusClear clearBus(BusLines &lines)
{
  BusClear result;
  lines.sdaLow(false);
  lines.sclLow(false);
  lines.halfClock();
  result.sdaBefore = lines.sda();
  result.sclBefore = lines.scl();
  if (!sclComesUp(lines)) {
    result.sclHeld = true;
    return result;
  }

  // a part stuck mid-byte holds SDA until it has clocked the byte out
  while (!lines.sda() && result.pulses < kI2cClearPulses) {
    lines.sclLow(true);
    lines.halfClock();
    lines.sclLow(false);
    if (!sclComesUp(lines)) {
      result.sclHeld = true;
      return result;
    }
    lines.halfClock();
    ++result.pulses;
  }

  // STOP
  lines.sclLow(true);
  lines.halfClock();
  lines.sdaLow(true);
  lines.halfClock();
  lines.sclLow(false);
  if (!sclComesUp(lines)) {
    lines.sdaLow(false);
    result.sclHeld = true;
    return result;
  }
  lines.halfClock();
  lines.sdaLow(false);
  lines.halfClock();
  result.ok = lines.sda() && lines.scl();
  return result;
}

uint8_t BusWatch::slot(uint8_t address)
{
  switch (address) {
    case kImuAddress: return 0;
    case kEnvAddress: return 1;
    case kGaugeAddress: return 2;
    default: return 3;
  }
}

BusGate BusWatch::before(uint8_t address, bool sda, bool scl, bool wireUp, uint32_t nowMs)
{
  if (!sda || !scl) {
    if (!linesHeld_) {
      LOG_W(Bus, "held") {
        line.field("sda", sda);
        line.field("scl", scl);
        line.field("part", busPartName(address));
        line.fieldHex("addr", address, 2);
        line.field("when", "before");
      }
    }
    linesHeld_ = true;
    clearNeeded_ = true;
    return BusGate::Held;
  }
  if (!wireUp) {
    return BusGate::Held;   // only a clear that works brings Wire back
  }

  Part &part = parts_[slot(address)];
  if (part.quarantined) {
    if (earlier(nowMs, part.quarantineUntilMs)) {
      return BusGate::Refused;
    }
    // probation, one more jam puts it back in for twice as long
    part.quarantined = false;
    part.jams = kBusJamsToQuarantine - 1;
    part.firstJamMs = nowMs;
    LOG_I(Bus, "release") {
      line.field("part", busPartName(address));
      line.fieldHex("addr", address, 2);
    }
  }
  if (part.held) {
    if (earlier(nowMs, part.holdUntilMs)) {
      return BusGate::Refused;
    }
    part.held = false;
  }
  return BusGate::Go;
}

void BusWatch::jam(uint8_t address, bool sda, bool scl, uint32_t nowMs)
{
  if (!linesHeld_) {
    LOG_W(Bus, "held") {
      line.field("sda", sda);
      line.field("scl", scl);
      line.field("part", busPartName(address));
      line.fieldHex("addr", address, 2);
      line.field("when", "after");
    }
  }
  linesHeld_ = true;
  clearNeeded_ = true;

  Part &part = parts_[slot(address)];
  if (part.jams == 0 || nowMs - part.firstJamMs > kBusJamWindowMs) {
    part.jams = 0;
    part.firstJamMs = nowMs;
  }
  if (++part.jams < kBusJamsToQuarantine) {
    return;
  }
  part.quarantineMs = doubled(part.quarantineMs, kBusQuarantineMs, kBusQuarantineMaxMs);
  part.quarantineUntilMs = nowMs + part.quarantineMs;
  part.quarantined = true;
  LOG_W(Bus, "quarantine") {
    line.field("part", busPartName(address));
    line.fieldHex("addr", address, 2);
    line.field("jams", static_cast<unsigned int>(part.jams));
    line.field("forms", part.quarantineMs);
  }
  part.jams = 0;
}

void BusWatch::after(uint8_t address, Status status, uint32_t tookMs, bool sda, bool scl, uint32_t nowMs)
{
  if (!sda || !scl) {   // was idle at the gate, so it's this address
    jam(address, sda, scl, nowMs);
    return;   // not counted as a timeout as well
  }

  const uint8_t at = slot(address);
  Part &part = parts_[at];
  if (status == Status::Ok) {
    part.timeouts = 0;
    part.holdMs = 0;
    return;
  }
  // fast NACK, absent part, costs nothing
  if (status != Status::Timeout && tookMs < kBusSlowMs) {
    part.timeouts = 0;
    return;
  }

  if (++part.timeouts >= kBusTimeoutsToHold) {
    part.holdMs = doubled(part.holdMs, kBusHoldMs, kBusHoldMaxMs);
    part.holdUntilMs = nowMs + part.holdMs;
    part.held = true;
    part.timeouts = 0;
    LOG_W(Bus, "hold") {
      line.field("part", busPartName(address));
      line.fieldHex("addr", address, 2);
      line.field("why", "timeout");
      line.field("ms", tookMs);
      line.field("forms", part.holdMs);
    }
  }

  // two parts spinning close together is the bus, not a part
  for (uint8_t other = 0; other < kSlots; ++other) {
    if (other != at && parts_[other].failedRecently && nowMs - parts_[other].failedMs <= kBusFaultWindowMs) {
      if (!clearNeeded_) {
        LOG_W(Bus, "fault") {
          line.field("why", "timeouts");
          line.field("part", busPartName(address));
          line.field("ms", tookMs);
        }
      }
      clearNeeded_ = true;
      for (uint8_t i = 0; i < kSlots; ++i) {
        parts_[i].failedRecently = false;
      }
      return;
    }
  }
  part.failedRecently = true;
  part.failedMs = nowMs;
}

bool BusWatch::clearDue(uint32_t nowMs) const
{
  return clearNeeded_ && (clearRetryMs_ == 0 || nowMs - clearedMs_ >= clearRetryMs_);
}

void BusWatch::cleared(const BusClear &result, uint32_t nowMs)
{
  ++clears_;
  clearedAny_ = true;
  clearedMs_ = nowMs;
  if (result.ok) {
    linesHeld_ = false;
    clearNeeded_ = false;
    clearRetryMs_ = 0;
  } else {
    linesHeld_ = true;
    clearRetryMs_ = doubled(clearRetryMs_, kBusClearRetryMs, kBusClearRetryMaxMs);
  }
  LOG_W(Bus, "clear") {
    line.field("sda", result.sdaBefore);
    line.field("scl", result.sclBefore);
    line.field("pulses", static_cast<unsigned int>(result.pulses));
    line.field("ok", result.ok);
    if (!result.ok) {
      line.field("sclheld", result.sclHeld);
      line.field("retryms", clearRetryMs_);
    }
    line.field("n", clears_);
  }
}

bool BusWatch::faulted(uint32_t nowMs) const
{
  return linesHeld_ || clearNeeded_ || (clearedAny_ && nowMs - clearedMs_ < kBusSettleMs);
}

bool BusWatch::quarantined(uint8_t address, uint32_t nowMs) const
{
  const Part &part = parts_[slot(address)];
  return part.quarantined && earlier(nowMs, part.quarantineUntilMs);
}

} // namespace ChipSatPlatform
