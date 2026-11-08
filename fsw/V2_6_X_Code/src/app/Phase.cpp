#include "Phase.h"
#include <math.h>
#include "../config.h"
#include "../log/SystemEvent.h"

namespace ChipSatApp
{

using namespace ChipSatConfig;
using namespace ChipSatLog;

bool FlightPhase::Track::live(uint32_t nowMs) const
{
  return have && nowMs - lastMs <= kAltitudeStaleMs;
}

// The span goes on while every whole second's lowest stays within flatM of the others' lowest, and
// its highest of their highest. Broken, it starts again from this second
void FlightPhase::Track::closeBin()
{
  lowM = binLowM;
  if (flat) {
    lowMinM = fminf(lowMinM, binLowM);
    lowMaxM = fmaxf(lowMaxM, binLowM);
    highMinM = fminf(highMinM, binHighM);
    highMaxM = fmaxf(highMaxM, binHighM);
    flat = lowMaxM - lowMinM <= flatM && highMaxM - highMinM <= flatM;
  }
  if (!flat) {
    flat = true;
    flatSinceMs = binIndex * kDescentBinMs;
    lowMinM = lowMaxM = binLowM;
    highMinM = highMaxM = binHighM;
  }
  flatToMs = (binIndex + 1) * kDescentBinMs;
}

void FlightPhase::envSample(uint32_t nowMs, float altitudeM)
{
  if (!isfinite(altitudeM)) {
    return;
  }
  const uint32_t index = nowMs / kDescentBinMs;
  Bin &b = bins_[index % (kDescentBins + 1)];
  if (b.index != index) {
    b.index = index;
    b.minM = altitudeM;
    b.maxM = altitudeM;
  } else {
    b.minM = fminf(b.minM, altitudeM);
    b.maxM = fmaxf(b.maxM, altitudeM);
  }
  baro_.riseGainM = kRiseGainM;
  baro_.stillM = kQuietAltitudeM;
  baro_.flatM = kFlatAltitudeM;
  sample(baro_, nowMs, altitudeM);
}

void FlightPhase::gpsSample(uint32_t nowMs, float altitudeM)
{
  if (!isfinite(altitudeM)) {
    return;
  }
  gps_.riseGainM = kRiseGainGpsM;
  gps_.stillM = kQuietAltitudeGpsM;
  gps_.flatM = kFlatAltitudeGpsM;
  sample(gps_, nowMs, altitudeM);
}

// Every source keeps its own baseline and peak whether or not it is the one deciding, so the one
// that takes over when the other dies is not starting from the air
void FlightPhase::sample(Track &t, uint32_t nowMs, float altitudeM)
{
  if (!t.have || nowMs - t.lastMs > kAltitudeStaleMs) {
    t.flat = false;   // back after a gap, so the flat starts again
    t.binIndex = UINT32_MAX;
    t.lowM = altitudeM;
  }
  const uint32_t bin = nowMs / kDescentBinMs;
  if (bin != t.binIndex) {
    if (t.binIndex != UINT32_MAX) {
      t.closeBin();
    }
    t.binIndex = bin;
    t.binLowM = altitudeM;
    t.binHighM = altitudeM;
  } else {
    t.binLowM = fminf(t.binLowM, altitudeM);
    t.binHighM = fmaxf(t.binHighM, altitudeM);
  }

  t.altM = altitudeM;
  t.lastMs = nowMs;
  t.histM[t.next] = altitudeM;
  t.histMs[t.next] = nowMs;
  t.next = static_cast<uint8_t>((t.next + 1) % Track::kHistory);
  if (t.count < Track::kHistory) {
    t.count++;
  }

  // the rate against the oldest reading at least kPhaseRateWindowMs back, so one noisy reading
  // moves it little. 1.6 s of history on the barometer, 8 s on GPS
  t.rateMps = 0.0f;
  for (uint8_t i = 0; i < t.count; i++) {
    const uint8_t at = static_cast<uint8_t>((t.next + Track::kHistory - 1 - i) % Track::kHistory);
    const uint32_t ageMs = nowMs - t.histMs[at];
    if (ageMs >= kPhaseRateWindowMs) {
      t.rateMps = (altitudeM - t.histM[at]) * 1000.0f / static_cast<float>(ageMs);
      break;
    }
  }

  if (!t.have) {
    t.baselineM = altitudeM;
    t.peakM = altitudeM;
    t.have = true;
  } else if (phase_ == Phase::Air) {
    if (altitudeM > t.peakM) {
      t.peakM = altitudeM;
    }
  } else if (fabsf(t.rateMps) < kPhaseBaselineRateMps) {
    // the ground is wherever it has been sitting: weather and a walk to the drone move it, a climb
    // does not get the chance to
    t.baselineM += (altitudeM - t.baselineM) * kPhaseBaselineGain;
  }
  judge(nowMs);
}

void FlightPhase::imuSample(uint32_t nowMs, float accelMps2, float gyroRadPerSec)
{
  if (!isfinite(accelMps2) || !isfinite(gyroRadPerSec)) {
    return;
  }
  imuValid_ = true;
  inQuiet_ = false;   // the spell on altitude only runs while the IMU is missing
  const bool moving = accelMps2 >= kQuietAccelMps2 || gyroRadPerSec >= kQuietGyroRadPerSec;
  if (moving) {
    stillSinceMs_ = nowMs;
  }
  if (!imuFast()) {
    recent_ = static_cast<uint8_t>((recent_ << 1 | (moving ? 1U : 0U)) & ((1U << kWakeReadings) - 1U));
    if (__builtin_popcount(recent_) >= kWakeMoving) {
      waking_ = true;
      recent_ = 0;
      noteSystem(kSourcePhase, kPhaseImuFast, kPartImu, kWhyMoving, 0, nowMs);
    }
  }

  // Empty every slot since the last reading up to this one, which last held a reading a window ago.
  // Two readings in one slot: the newer counts
  const uint32_t slot = nowMs / kMotionSlotMs;
  const uint32_t skipped = slot - lastSlot_ < kMotionSlots ? slot - lastSlot_ : kMotionSlots;
  for (uint32_t i = 1; i <= skipped; i++) {
    clearSlot((lastSlot_ + i) % kMotionSlots);
  }
  const uint32_t at = slot % kMotionSlots;
  clearSlot(at);
  lastSlot_ = slot;
  const uint8_t mask = static_cast<uint8_t>(1U << (at % 8));
  received_[at / 8] |= mask;
  receivedCount_++;
  if (moving) {
    moved_[at / 8] |= mask;
    movedCount_++;
  }
  judge(nowMs);
}

void FlightPhase::clearSlot(uint32_t at)
{
  const uint8_t mask = static_cast<uint8_t>(1U << (at % 8));
  if (received_[at / 8] & mask) {
    receivedCount_--;
    received_[at / 8] = static_cast<uint8_t>(received_[at / 8] & ~mask);
  }
  if (moved_[at / 8] & mask) {
    movedCount_--;
    moved_[at / 8] = static_cast<uint8_t>(moved_[at / 8] & ~mask);
  }
}

// The window is left as it is: a gap only leaves its slots empty
void FlightPhase::imuLost(uint32_t nowMs)
{
  if (imuValid_) {
    imuValid_ = false;
    imuLostMs_ = nowMs;
  }
  judge(nowMs);
}

bool FlightPhase::changed()
{
  const bool was = changed_;
  changed_ = false;
  return was;
}

bool FlightPhase::logChanged()
{
  const bool was = logChanged_;
  logChanged_ = false;
  return was;
}

float FlightPhase::verticalRateMps() const
{
  switch (source_) {
    case AltitudeSource::Barometer: return baro_.rateMps;
    case AltitudeSource::Gps: return gps_.rateMps;
    default: return 0.0f;
  }
}

float FlightPhase::aboveGroundM() const
{
  switch (source_) {
    case AltitudeSource::Barometer: return baro_.altM - baro_.baselineM;
    case AltitudeSource::Gps: return gps_.altM - gps_.baselineM;
    default: return 0.0f;
  }
}

// The barometer while it is live, else GPS, else nothing
FlightPhase::Track *FlightPhase::deciding(uint32_t nowMs)
{
  const AltitudeSource was = source_;
  Track *t = nullptr;
  if (baro_.live(nowMs)) {
    source_ = AltitudeSource::Barometer;
    t = &baro_;
  } else if (gps_.live(nowMs)) {
    source_ = AltitudeSource::Gps;
    t = &gps_;
  } else {
    source_ = AltitudeSource::None;
  }
  // a spell at rest carries across, judged still from the new source's own altitude
  if (source_ != was && t != nullptr) {
    quietAltM_ = t->altM;
  }
  return t;
}

// The last kDescentBins whole seconds each lower than the one before by kDescentStepM, on both their
// lowest and their highest reading. Readings offset by a transmit lift a second's highest and leave
// its lowest, a step moves one pair of seconds only, and a second without a reading breaks the run
bool FlightPhase::descending(uint32_t nowMs) const
{
  const uint32_t now = nowMs / kDescentBinMs;
  if (now < kDescentBins) {
    return false;
  }
  const Bin *before = nullptr;
  for (uint32_t index = now - kDescentBins; index < now; index++) {
    const Bin &b = bins_[index % (kDescentBins + 1)];
    if (b.index != index) {
      return false;
    }
    if (before != nullptr && (b.minM > before->minM - kDescentStepM || b.maxM > before->maxM - kDescentStepM)) {
      return false;
    }
    before = &b;
  }
  return true;
}

uint8_t FlightPhase::sourcePart() const
{
  switch (source_) {
    case AltitudeSource::Barometer: return kPartEnv;
    case AltitudeSource::Gps: return kPartGps;
    default: return kPartNone;
  }
}

void FlightPhase::enter(Phase next, uint32_t nowMs, uint8_t why, uint8_t part)
{
  if (next == phase_) {
    return;
  }
  noteSystem(kSourcePhase, next == Phase::Air ? kPhaseToAir : kPhaseToGround, part, why, 0, nowMs);
  const bool logged = logsAir();
  phase_ = next;
  changed_ = true;
  bootAir_ = false;
  waking_ = false;
  recent_ = 0;
  landed_ = next == Phase::Ground;
  motionSinceMs_ = nowMs;
  quietSinceMs_ = nowMs;
  if (next == Phase::Air) {
    inQuiet_ = false;
    altitudeInAir_ = false;
    stillSinceMs_ = nowMs;
    baro_.peakM = baro_.altM;
    gps_.peakM = gps_.altM;
  } else {
    settle();
  }
  logChanged_ = logChanged_ || logsAir() != logged;
}

// the ground is where it came to rest
void FlightPhase::settle()
{
  inQuiet_ = false;
  baro_.baselineM = baro_.altM;
  gps_.baselineM = gps_.altM;
}

void FlightPhase::judge(uint32_t nowMs)
{
  Track *t = deciding(nowMs);
  const bool falling = descending(nowMs);
  if (falling) {
    stillSinceMs_ = nowMs;
    if (!falling_ && phase_ == Phase::Air) {
      noteSystem(kSourcePhase, kPhaseDescent, kPartEnv, kWhyBaroDescent, 0, nowMs);
    }
  }
  falling_ = falling;
  // a gap holds the phase, only a dead IMU hands it to altitude
  const bool imuDead = !imuValid_ && nowMs - imuLostMs_ > kImuDeadMs;

  if (phase_ != Phase::Air) {
    // Most of the readings in the window moved, and most of the window has readings so a few after a
    // gap or a reset can't decide. A live altitude has to have climbed too, on the lowest reading
    // of the last whole second, or the window held a minute. Never on the barometer while the IMU
    // is there
    bool air = false;
    uint8_t why = kWhyImuMotion;
    if (imuValid_) {
      const bool moving = static_cast<float>(receivedCount_) >= kMotionFilled * static_cast<float>(kMotionSlots) &&
                          static_cast<float>(movedCount_) >= kMotionFraction * static_cast<float>(receivedCount_);
      if (!moving) {
        motionSinceMs_ = nowMs;
      }
      if (moving && (t == nullptr || t->lowM - t->baselineM >= t->riseGainM)) {
        air = true;
      } else if (moving && nowMs - motionSinceMs_ >= kMotionHeldMs) {
        air = true;
        why = kWhyMotionHeld;
      }
    } else {
      motionSinceMs_ = nowMs;
      if (imuDead && t != nullptr) {
        air = t->altM - t->baselineM >= t->riseGainM;
        why = kWhyImuDead;
      }
    }
    if (air) {
      enter(Phase::Air, nowMs, why, imuValid_ ? kPartImu : sourcePart());
      return;
    }
    if (waking_ && nowMs - stillSinceMs_ >= kWakeQuietMs) {
      waking_ = false;
      noteSystem(kSourcePhase, kPhaseImuSlow, kPartImu, kWhyQuiet, 0, nowMs);
    }
    // without this boot-air might never end and the chip would wrap over older flights. It has no
    // climb to come down from, so any height will do
    if (!bootAir_) {
      return;
    }
    const bool rest = imuValid_ ? nowMs - stillSinceMs_ >= kLandedMs && (t == nullptr || t->flatFor(nowMs, kLandedMs))
                                : stillOnAltitude(nowMs, t, false) && imuDead;
    if (rest || nowMs >= kBootAirMaxMs) {
      if (!rest) {
        noteSystem(kSourcePhase, kPhaseBootAirEnd, kPartNone, kWhyBootAirLimit, 0, nowMs);
      } else {
        noteSystem(kSourcePhase, kPhaseBootAirEnd, imuValid_ ? kPartImu : sourcePart(),
                   imuValid_ ? kWhyQuiet : kWhyImuDead, 0, nowMs);
      }
      bootAir_ = false;
      logChanged_ = true;
      quietSinceMs_ = nowMs;
      settle();
    }
    return;
  }

  if (t != nullptr) {
    altitudeInAir_ = true;
  }
  const uint8_t why = landing(nowMs, t, imuDead);
  if (why != 0) {
    enter(Phase::Ground, nowMs, why, why == kWhyImuDead ? sourcePart() : kPartImu);
  }
}

// Quiet, and a live altitude flat below half the climb, or flat that much longer at any height.
// Without a climb every height counts as low. With no altitude live all through the air,
// kGroundQuietMs quiet, and with the IMU dead the altitude alone as before
uint8_t FlightPhase::landing(uint32_t nowMs, Track *t, bool imuDead)
{
  if (!imuValid_) {
    return stillOnAltitude(nowMs, t, true) && imuDead ? kWhyImuDead : 0;
  }
  const uint32_t quietMs = nowMs - stillSinceMs_;
  if (!altitudeInAir_) {
    return quietMs >= kGroundQuietMs ? kWhyQuietNoAlt : 0;
  }
  if (t == nullptr) {
    return quietMs >= kLandedMs ? kWhyQuiet : 0;
  }
  const float climbM = t->peakM - t->baselineM;
  const bool low = climbM < t->riseGainM || t->lowM <= t->baselineM + 0.5f * climbM;
  if (quietMs >= kLandedMs && low && t->flatFor(nowMs, kLandedMs)) {
    return kWhyQuiet;
  }
  return quietMs >= kLandedHighMs && t->flatFor(nowMs, kLandedHighMs) ? kWhyQuietHigh : 0;
}

// With the IMU missing: a drone hovering dead still is above half the climb, so this never lands it.
// A drone that lands with the sat still on it is below. Without a climb every height counts as low.
// It runs from the moment the IMU goes missing so a dead one has its spell already
bool FlightPhase::stillOnAltitude(uint32_t nowMs, Track *t, bool climbed)
{
  if (t == nullptr) {
    return false;   // nothing to judge by: stays as it is
  }
  const float halfway = t->baselineM + 0.5f * (t->peakM - t->baselineM);
  const bool low = !climbed || t->altM <= halfway;
  const bool still = low && fabsf(t->altM - quietAltM_) < t->stillM;
  if (!inQuiet_ || !still) {
    inQuiet_ = low;
    quietSinceMs_ = nowMs;
    quietAltM_ = t->altM;
    return false;
  }
  return nowMs - quietSinceMs_ >= kLandedNoImuMs;
}

} // namespace ChipSatApp
