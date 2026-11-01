#pragma once

#include <stdint.h>

namespace ChipSatApp
{

// Where the sat is in its flight, from an altitude source and the IMU. A board is switched on
// mounted on the drone, often long before it flies, so the phase sets how fast it transmits and how
// much the flash log keeps: slow and a trickle on the ground, fast and everything in the air. Both
// come from the settings block, a transmit rate and a log rate for each phase
//
//   Ground  switched on, and again once it has come to rest. Handling it doesn't move it out of
//           here, only climbing does. The baseline it climbs from tracks slowly while quiet
//   Air     climbed kRiseGain above the baseline. Stays through the release and the fall, a release
//           can't be told from the drone (see config.h). Ends at rest: kLandedMs still below half
//           the climb, or kGroundQuietMs of a quiet IMU at any height (roof, hill, car ride). A
//           flying drone never read quiet that long (0.3 to 28 m/s2 on 2026-10-13)
//
// Same values as the old Waiting (0) and Rising (1) so old records still read. An old 3 (Landed) is
// the ground, an old 2 (Falling, never entered) the air
//
// Altitude comes from the barometer when it is live, else GPS (3D fix), each with its own baseline,
// thresholds and noise. With neither, the IMU alone tells a running drone from the ground. It never
// declares the air by itself while an altitude source is live
enum class Phase : uint8_t
{
  Ground = 0,
  Air = 1,
};

// After a reset the log starts in the air so a reset mid-flight still keeps the fall (decided
// 2026-10-22), the radio on the ground. Boot-air ends at rest, on a climb or at kBootAirMaxMs
constexpr Phase kPhaseAtBoot = Phase::Ground;
constexpr bool kLogAirAtBoot = true;

enum class AltitudeSource : uint8_t
{
  None = 0,
  Barometer = 1,
  Gps = 2,
};

class FlightPhase
{
public:
  // A fresh barometer reading. altitudeM is the sensor's own pressure altitude; only differences
  // are used, so the sea level it assumes does not matter
  void envSample(uint32_t nowMs, float altitudeM);

  // A fresh GPS altitude, from a trusted 3D fix only: before the fix the receiver says 0
  void gpsSample(uint32_t nowMs, float altitudeM);

  // A fresh IMU reading: the magnitude of the linear acceleration and of the rotation rate
  void imuSample(uint32_t nowMs, float accelMps2, float gyroRadPerSec);

  // Called when the IMU has gone quiet (its data is older than the controller trusts), so the
  // rules that need it can do without it rather than wait for it
  void imuLost(uint32_t nowMs);

  Phase phase() const { return phase_; }   // what the radio follows

  // True once after each change of phase, so the controller can act on it (transmit at once)
  bool changed();

  bool logsAir() const { return phase_ == Phase::Air || bootAir_; }   // what the flash log follows
  bool logChanged();   // true once after each change

  // Which altitude is deciding, as of the last sample. None means the IMU alone is
  AltitudeSource altitudeSource() const { return source_; }

  // The vertical rate over the last second on the deciding source, m/s, up positive. 0 without one
  float verticalRateMps() const;

  // Height above the ground baseline on the deciding source, m. 0 without one
  float aboveGroundM() const;

private:
  // One altitude source: its history for the rate, the ground it climbs from, the top it reached
  struct Track
  {
    static constexpr uint8_t kHistory = 8;
    float histM[kHistory] = {};
    uint32_t histMs[kHistory] = {};
    uint8_t count = 0;
    uint8_t next = 0;
    float altM = 0.0f;
    float rateMps = 0.0f;
    float baselineM = 0.0f;
    float peakM = 0.0f;
    bool have = false;
    uint32_t lastMs = 0;
    float riseGainM = 0.0f;   // how far above the baseline is the air, on this source
    float stillM = 0.0f;      // how much it may wander and still be at rest, on this source
    bool live(uint32_t nowMs) const;
  };

  void sample(Track &t, uint32_t nowMs, float altitudeM);
  Track *deciding(uint32_t nowMs);
  void judge(uint32_t nowMs);
  void enter(Phase next, uint32_t nowMs);
  bool atRest(uint32_t nowMs, Track *t, bool climbed);
  void settle();
  bool imuQuiet() const;
  void resetMotion();

  Phase phase_ = kPhaseAtBoot;
  bool changed_ = false;
  bool bootAir_ = kLogAirAtBoot;
  bool logChanged_ = false;
  AltitudeSource source_ = AltitudeSource::None;

  Track baro_;
  Track gps_;

  // IMU
  float accel_ = 0.0f;
  float gyro_ = 0.0f;
  bool imuValid_ = false;
  // The last kMotionWindowMs of IMU readings, one bit each, for the blind rule: how many were moving
  static constexpr uint8_t kMotionSamples = 200;   // 10 s at 50 ms
  uint8_t motionBits_[kMotionSamples / 8] = {};
  uint8_t motionNext_ = 0;
  uint8_t motionCount_ = 0;
  uint8_t moving_ = 0;

  // the spell at rest that ends a flight
  uint32_t quietSinceMs_ = 0;
  float quietAltM_ = 0.0f;
  bool inQuiet_ = false;

  // for kGroundQuietMs, altitude ignored
  uint32_t imuQuietSinceMs_ = 0;
  bool imuQuietRun_ = false;
};

} // namespace ChipSatApp
