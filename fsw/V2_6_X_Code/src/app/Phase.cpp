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
  baro_.flatM = kFlatAltitudeM;
  sample(baro_, nowMs, altitudeM);
}

void FlightPhase::gpsSample(uint32_t nowMs, float altitudeM)
{
  if (!isfinite(altitudeM)) {
    return;
  }
  gps_.riseGainM = kRiseGainGpsM;
  gps_.flatM = kFlatAltitudeGpsM;
  sample(gps_, nowMs, altitudeM);
}

// Every source keeps its own baseline whether or not it is the one deciding, so the one that takes
// over when the other dies is not starting from the air
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
    t.have = true;
  } else if (phase_ != Phase::Air && fabsf(t.rateMps) < kPhaseBaselineRateMps) {
    // the ground is wherever it has been sitting: weather and a walk to the drone move it, a climb
    // does not get the chance to
    t.baselineM += (altitudeM - t.baselineM) * kPhaseBaselineGain;
  }
  judge(nowMs);
}

void FlightPhase::imuSample(uint32_t nowMs, float accelMps2, float gyroRadPerSec, float feltMps2,
                            const ChipSatSensors::Quaternionf &orientation)
{
  if (!isfinite(accelMps2) || !isfinite(gyroRadPerSec)) {
    return;
  }
  imuValid_ = true;
  const bool moving = accelMps2 >= kQuietAccelMps2 || gyroRadPerSec >= kQuietGyroRadPerSec;
  if (moving || turned(nowMs, orientation)) {
    stillSinceMs_ = nowMs;
  }
  // At 1 Hz the readings are a second apart, so neither spell can form there
  if (gyroRadPerSec >= kTumbleRadPerSec) {
    if (nowMs - spinLastMs_ > kSpellGapMs) {
      spinSinceMs_ = nowMs;
    }
    spinLastMs_ = nowMs;
  }
  if (feltMps2 < kFreeFallMps2) {
    if (!weightless_ || nowMs - weightlessLastMs_ > kSpellGapMs) {
      weightlessSinceMs_ = nowMs;
    }
    weightless_ = true;
    weightlessLastMs_ = nowMs;
  } else {
    weightless_ = false;
  }
  // One moving reading at 1 Hz is 20 Hz: the start of a fall is the part that must not be lost, and
  // a knock costs at most kWakeQuietMs at 20 Hz
  if (moving && !imuFast()) {
    waking_ = true;
    noteSystem(kSourcePhase, kPhaseImuFast, kPartImu, kWhyMoving, 0, nowMs);
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
  if (moving) {
    moved_[at / 8] |= static_cast<uint8_t>(1U << (at % 8));
    movedCount_++;
  }
  judge(nowMs);
}

// Against the orientation kTurnSpanMs before, through the angle between the two: 2 acos |q1 . q2|.
// All zeros is no orientation at all
bool FlightPhase::turned(uint32_t nowMs, const ChipSatSensors::Quaternionf &q)
{
  if (q.i == 0.0f && q.j == 0.0f && q.k == 0.0f && q.real == 0.0f) {
    return false;
  }
  const uint32_t second = nowMs / 1000 + 1;   // 0 is an empty slot
  const uint32_t at = second % kTurnSlots;
  if (turnSecond_[at] != second) {
    turnSecond_[at] = second;
    turn_[at] = q;
  }
  const uint32_t then = second - kTurnSpanMs / 1000;
  const ChipSatSensors::Quaternionf &p = turn_[then % kTurnSlots];
  if (second <= kTurnSpanMs / 1000 || turnSecond_[then % kTurnSlots] != then) {
    return false;
  }
  const float dot = fabsf(p.i * q.i + p.j * q.j + p.k * q.k + p.real * q.real);
  return dot < cosf(0.5f * kTurnedRad);
}

void FlightPhase::clearSlot(uint32_t at)
{
  const uint8_t mask = static_cast<uint8_t>(1U << (at % 8));
  if (moved_[at / 8] & mask) {
    movedCount_--;
    moved_[at / 8] = static_cast<uint8_t>(moved_[at / 8] & ~mask);
  }
}

// The window is left as it is: a gap only leaves its slots empty
void FlightPhase::imuLost(uint32_t nowMs)
{
  imuValid_ = false;
  judge(nowMs);
}

void FlightPhase::tilted(uint32_t nowMs)
{
  stillSinceMs_ = nowMs;
  if (!imuFast()) {
    waking_ = true;
    noteSystem(kSourcePhase, kPhaseImuFast, kPartImu, kWhyTilted, 0, nowMs);
  }
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
  if (baro_.live(nowMs)) {
    source_ = AltitudeSource::Barometer;
    return &baro_;
  }
  if (gps_.live(nowMs)) {
    source_ = AltitudeSource::Gps;
    return &gps_;
  }
  source_ = AltitudeSource::None;
  return nullptr;
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
  landed_ = next == Phase::Ground;
  if (next == Phase::Air) {
    altitudeInAir_ = false;
    stillSinceMs_ = nowMs;
  } else {
    settle();
  }
  logChanged_ = logChanged_ || logsAir() != logged;
}

// the ground is where it came to rest
void FlightPhase::settle()
{
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

  if (phase_ != Phase::Air) {
    // The air at the slightest hint (decided 2026-11-09). A fall needs no climb: the board may have
    // been on the ground since a hover or a missed take-off, and a drop shorter than kRiseGainM never
    // climbs back past the baseline. A climb needs nothing from the IMU: with transmit readings kept
    // out the barometer can be taken at its word, and a drone climbs smoothly with its IMU often
    // quiet (board 15's missed drop, 2026-11-08). It is judged on the last whole second's lowest
    uint8_t why = 0;
    if (imuValid_ && spinLastMs_ - spinSinceMs_ >= kTumbleMs && nowMs - spinLastMs_ <= kSpellGapMs) {
      why = kWhyTumbling;
    } else if (imuValid_ && weightless_ && weightlessLastMs_ - weightlessSinceMs_ >= kFreeFallMs &&
               nowMs - weightlessLastMs_ <= kSpellGapMs) {
      why = kWhyFreeFall;
    } else if (imuValid_ && movedCount_ >= kHintMoving) {
      why = kWhyMotionHint;
    } else if (t != nullptr && t->lowM - t->baselineM >= t->riseGainM) {
      why = kWhyClimb;
    }
    if (why != 0) {
      enter(Phase::Air, nowMs, why, why == kWhyClimb ? sourcePart() : kPartImu);
      return;
    }
    if (waking_ && nowMs - stillSinceMs_ >= kWakeQuietMs) {
      waking_ = false;
      noteSystem(kSourcePhase, kPhaseImuSlow, kPartImu, kWhyQuiet, 0, nowMs);
    }
    // without this boot-air might never end and the chip would wrap over older flights
    if (!bootAir_) {
      return;
    }
    const bool rest = nowMs - stillSinceMs_ >= kLandedMs && (t == nullptr || t->flatFor(nowMs, kLandedMs));
    if (rest || nowMs >= kBootAirMaxMs) {
      if (rest) {
        noteSystem(kSourcePhase, kPhaseBootAirEnd, kPartImu, kWhyQuiet, 0, nowMs);
      } else {
        noteSystem(kSourcePhase, kPhaseBootAirEnd, kPartNone, kWhyBootAirLimit, 0, nowMs);
      }
      bootAir_ = false;
      logChanged_ = true;
      settle();
    }
    return;
  }

  if (t != nullptr) {
    altitudeInAir_ = true;
  }
  const uint8_t why = landing(nowMs, t);
  if (why != 0) {
    enter(Phase::Ground, nowMs, why, kPartImu);
  }
}

// At any height: a drone's IMU is never quiet for long while it flies (decided 2026-11-16). The live
// altitude flat as well lands it at kLandedMs, and kLandedImuMs lands it whatever the altitude does:
// on 2026-11-08 the barometer kept boards lying still from ever reading flat. With no altitude live
// all through the air, kGroundQuietMs quiet. An IMU with no readings is quiet, dead or in a gap
// (decided 2026-11-16): the altitude is only the backup
uint8_t FlightPhase::landing(uint32_t nowMs, Track *t)
{
  const uint32_t quietMs = nowMs - stillSinceMs_;
  if (!altitudeInAir_) {
    return quietMs >= kGroundQuietMs ? kWhyQuietNoAlt : 0;
  }
  if (quietMs >= kLandedMs && (t == nullptr || t->flatFor(nowMs, kLandedMs))) {
    return kWhyQuiet;
  }
  return quietMs >= kLandedImuMs ? kWhyQuietImu : 0;
}

} // namespace ChipSatApp
