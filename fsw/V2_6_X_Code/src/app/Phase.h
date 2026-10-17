#pragma once

#include <stdint.h>

namespace ChipSatApp
{

// Where the sat is in its flight, from an altitude source and the IMU. A board is switched on
// mounted on the drone, often long before it flies, so the phases are what set how fast it logs and
// transmits: slowly while it waits and after it lands, fast while it is in the air
//
//   Waiting  switched on, on the ground or on the drone. Handling it does not move it out of here:
//            only climbing does. The altitude baseline it climbs from tracks slowly while quiet
//   Rising   in the air: climbed kRiseGain above the baseline. Stays through the release and the
//            fall, because a release cannot be told from the drone (see config.h). Fast logging
//   Landed   still, for kLandedMs, below half of what it climbed. A hovering drone is not landed:
//            it is still up there. Landed re-arms: the next climb from where it lies is rising again
//   Falling  never entered. Kept because the settings block and the ground have a slot for it
//
// Altitude comes from the barometer when it is live, else GPS (3D fix), each with its own baseline,
// thresholds and noise. With neither, the IMU alone tells a running drone from the ground, and the
// controller logs at the blind rate whatever the phase says
enum class Phase : uint8_t
{
  Waiting = 0,
  Rising = 1,
  Falling = 2,
  Landed = 3,
};

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

  Phase phase() const { return phase_; }

  // True once after each change of phase, so the controller can act on it (transmit at once)
  bool changed();

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
  bool imuQuiet() const;
  void resetMotion();

  Phase phase_ = Phase::Waiting;
  bool changed_ = false;
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
};

} // namespace ChipSatApp
