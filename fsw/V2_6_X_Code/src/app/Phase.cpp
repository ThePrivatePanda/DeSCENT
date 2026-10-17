#include "Phase.h"
#include <math.h>
#include "../config.h"

namespace ChipSatApp
{

using namespace ChipSatConfig;

bool FlightPhase::Track::live(uint32_t nowMs) const
{
  return have && nowMs - lastMs <= kAltitudeStaleMs;
}

void FlightPhase::envSample(uint32_t nowMs, float altitudeM)
{
  if (!isfinite(altitudeM)) {
    return;
  }
  baro_.riseGainM = kRiseGainM;
  baro_.stillM = kQuietAltitudeM;
  sample(baro_, nowMs, altitudeM);
}

void FlightPhase::gpsSample(uint32_t nowMs, float altitudeM)
{
  if (!isfinite(altitudeM)) {
    return;
  }
  gps_.riseGainM = kRiseGainGpsM;
  gps_.stillM = kQuietAltitudeGpsM;
  sample(gps_, nowMs, altitudeM);
}

// Every source keeps its own baseline and peak whether or not it is the one deciding, so the one
// that takes over when the other dies is not starting from the air
void FlightPhase::sample(Track &t, uint32_t nowMs, float altitudeM)
{
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
  } else if (phase_ == Phase::Rising) {
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
  accel_ = accelMps2;
  gyro_ = gyroRadPerSec;
  imuValid_ = true;

  // one bit per reading: moving or not, over the last window
  const bool moving = !imuQuiet();
  const uint8_t byte = static_cast<uint8_t>(motionNext_ / 8);
  const uint8_t mask = static_cast<uint8_t>(1U << (motionNext_ % 8));
  if (motionCount_ == kMotionSamples) {
    if (motionBits_[byte] & mask) {
      moving_--;
    }
  } else {
    motionCount_++;
  }
  if (moving) {
    motionBits_[byte] |= mask;
    moving_++;
  } else {
    motionBits_[byte] = static_cast<uint8_t>(motionBits_[byte] & ~mask);
  }
  motionNext_ = static_cast<uint8_t>((motionNext_ + 1) % kMotionSamples);
  judge(nowMs);
}

void FlightPhase::imuLost(uint32_t nowMs)
{
  imuValid_ = false;
  resetMotion();
  judge(nowMs);
}

// The blind rule judges only readings taken since the last change, of phase or of source: what the
// IMU saw during a flight is no reason to call the ground after it a flight
void FlightPhase::resetMotion()
{
  motionCount_ = 0;
  motionNext_ = 0;
  moving_ = 0;
  for (uint8_t i = 0; i < kMotionSamples / 8; i++) {
    motionBits_[i] = 0;
  }
}

bool FlightPhase::changed()
{
  const bool was = changed_;
  changed_ = false;
  return was;
}

bool FlightPhase::imuQuiet() const
{
  return accel_ < kQuietAccelMps2 && gyro_ < kQuietGyroRadPerSec;
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
  if (source_ != was) {
    // A spell at rest carries across: the IMU is what mostly says still, and the new source's
    // own altitude is where it is judged still from. Only the blind rule starts over
    resetMotion();
    if (t != nullptr) {
      quietAltM_ = t->altM;
    }
  }
  return t;
}

void FlightPhase::enter(Phase next, uint32_t nowMs)
{
  if (next == phase_) {
    return;
  }
  phase_ = next;
  changed_ = true;
  inQuiet_ = false;
  quietSinceMs_ = nowMs;
  resetMotion();
  if (next == Phase::Rising) {
    baro_.peakM = baro_.altM;
    gps_.peakM = gps_.altM;
  } else {
    // the ground is where it came to rest, on every source that has a reading
    baro_.baselineM = baro_.altM;
    gps_.baselineM = gps_.altM;
  }
}

void FlightPhase::judge(uint32_t nowMs)
{
  Track *t = deciding(nowMs);

  if (phase_ != Phase::Rising) {
    // Waiting and Landed end the same way: by climbing. With an altitude, kRiseGain above the
    // ground. Without one, the IMU reading a running drone for most of the last window
    if (t != nullptr) {
      if (t->altM - t->baselineM >= t->riseGainM) {
        enter(Phase::Rising, nowMs);
      }
    } else if (imuValid_ && motionCount_ == kMotionSamples &&
               static_cast<float>(moving_) >= kMotionFraction * static_cast<float>(kMotionSamples)) {
      enter(Phase::Rising, nowMs);
    }
    return;
  }

  // Rising ends by coming to rest below half of what it climbed, for long enough. A drone hovering
  // dead still is above that line, so it is never landed; a drone that lands with the sat still on
  // it is, and the next take-off is rising again from there
  bool still;
  uint32_t need;
  if (t != nullptr) {
    const float halfway = t->baselineM + 0.5f * (t->peakM - t->baselineM);
    const bool low = t->altM <= halfway;
    const bool altStill = fabsf(t->altM - quietAltM_) < t->stillM;
    still = low && altStill && (!imuValid_ || imuQuiet());
    need = imuValid_ ? kLandedMs : kLandedNoImuMs;
    if (!inQuiet_ || !still) {
      inQuiet_ = low && (!imuValid_ || imuQuiet());
      quietSinceMs_ = nowMs;
      quietAltM_ = t->altM;
      return;
    }
  } else {
    if (!imuValid_) {
      return;   // nothing to judge by: stays as it is
    }
    still = imuQuiet();
    need = kBlindLandedMs;
    if (!inQuiet_ || !still) {
      inQuiet_ = still;
      quietSinceMs_ = nowMs;
      return;
    }
  }
  if (nowMs - quietSinceMs_ >= need) {
    enter(Phase::Landed, nowMs);
  }
}

} // namespace ChipSatApp
