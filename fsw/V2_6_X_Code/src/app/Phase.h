#pragma once

#include <stdint.h>
#include "../config.h"
#include "../devices/SensorData.h"

namespace ChipSatApp
{

// Where the sat is in its flight, on the ground or in the air. Along with boot-air it sets the
// transmit and log rates (one of each per phase in the settings block) and the IMU's rate. What
// decides each change, and why, is in docs/phases.md
//
// Same values as the old Waiting (0) and Rising (1) so old records still read. An old 3 (Landed) is
// the ground, an old 2 (Falling, never entered) the air
enum class Phase : uint8_t
{
  Ground = 0,
  Air = 1,
};

// After a reset the log and the transmit gap start as the air's so a reset mid-flight still keeps
// the fall (the log decided 2026-10-22, the radio 2026-11-16), the phase on the ground. Boot-air ends
// at rest, in the air or at kBootAirMaxMs
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

  // A fresh IMU reading: the magnitude of the linear acceleration and of the rotation rate, and of
  // the acceleration the sensor feels, gravity included (about 9.8 at rest, near 0 falling free),
  // and the orientation. NAN and all zeros for the last two when the orientation isn't known
  void imuSample(uint32_t nowMs, float accelMps2, float gyroRadPerSec, float feltMps2,
                 const ChipSatSensors::Quaternionf &orientation);

  // Called when the IMU's data is older than the controller trusts, every loop until it is back
  void imuLost(uint32_t nowMs);

  // Asleep after landing the controller found gravity turned since it fell asleep: the board was
  // moved, so as a moving reading at 1 Hz, 20 Hz and the quiet timers from now
  void tilted(uint32_t nowMs);

  Phase phase() const { return phase_; }

  // True once after each change of phase, so the controller can act on it (transmit at once)
  bool changed();

  bool logsAir() const { return phase_ == Phase::Air || bootAir_; }   // what the flash log and transmit gap follow
  bool logChanged();   // true once after each change

  // 20 Hz: boot-air, the air, and the ground once something moved. 1 Hz otherwise
  bool imuFast() const { return bootAir_ || phase_ == Phase::Air || waking_; }

  // On the ground after a spell in the air this boot. Boot-air ending doesn't count
  bool landed() const { return landed_; }

  // Which altitude is live, as of the last sample: the barometer first, else GPS
  AltitudeSource altitudeSource() const { return source_; }

  // The vertical rate over the last second on the deciding source, m/s, up positive. 0 without one
  float verticalRateMps() const;

  // Height above the ground baseline on the deciding source, m. 0 without one
  float aboveGroundM() const;

private:
  // One altitude source: its history for the rate and the ground it climbs from
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
    bool have = false;
    uint32_t lastMs = 0;
    float riseGainM = 0.0f;   // how far above the baseline is the air, on this source
    float flatM = 0.0f;       // how far each second's lowest and highest may spread and still be flat

    // The second filling now, and the span of whole seconds before it that stayed flat
    uint32_t binIndex = UINT32_MAX;
    float binLowM = 0.0f;
    float binHighM = 0.0f;
    float lowM = 0.0f;   // the last whole second's lowest
    bool flat = false;
    uint32_t flatSinceMs = 0;
    uint32_t flatToMs = 0;
    float lowMinM = 0.0f;
    float lowMaxM = 0.0f;
    float highMinM = 0.0f;
    float highMaxM = 0.0f;

    bool live(uint32_t nowMs) const;
    void closeBin();
    bool flatFor(uint32_t nowMs, uint32_t spanMs) const { return live(nowMs) && flat && flatToMs - flatSinceMs >= spanMs; }
  };

  // A second of barometer readings, its lowest and highest
  struct Bin
  {
    uint32_t index = UINT32_MAX;
    float minM = 0.0f;
    float maxM = 0.0f;
  };

  void sample(Track &t, uint32_t nowMs, float altitudeM);
  Track *deciding(uint32_t nowMs);
  void judge(uint32_t nowMs);
  void enter(Phase next, uint32_t nowMs, uint8_t why, uint8_t part);   // ChipSatLog codes for the event
  uint8_t sourcePart() const;
  uint8_t landing(uint32_t nowMs, Track *t);   // the kWhy code, 0 still in the air
  bool descending(uint32_t nowMs) const;
  bool turned(uint32_t nowMs, const ChipSatSensors::Quaternionf &q);
  void clearSlot(uint32_t slot);
  void settle();

  Phase phase_ = kPhaseAtBoot;
  bool changed_ = false;
  bool bootAir_ = kLogAirAtBoot;
  bool logChanged_ = false;
  bool waking_ = false;
  bool landed_ = false;
  AltitudeSource source_ = AltitudeSource::None;

  Track baro_;
  Track gps_;
  bool altitudeInAir_ = false;   // a source was live at some point of this air
  Bin bins_[ChipSatConfig::kDescentBins + 1];   // by time, the one filling and the kDescentBins before it

  // The last kMotionWindowMs of IMU readings by time, a bit for each slot whose reading moved. The
  // slots a gap skips are empty, so a slow feed can't stretch the window
  static constexpr uint32_t kMotionSlots = ChipSatConfig::kMotionWindowMs / ChipSatConfig::kMotionSlotMs;
  uint8_t moved_[(kMotionSlots + 7) / 8] = {};
  uint32_t lastSlot_ = 0;
  uint16_t movedCount_ = 0;

  bool imuValid_ = false;   // readings coming. The window is kept through a gap but only counts while they are
  uint32_t stillSinceMs_ = 0;   // the last moving reading, falling reading or take-off. No reading is quiet

  bool falling_ = false;   // descending() on the last judge, for the event when it starts

  // the tumble: from its first reading over kTumbleRadPerSec to its last. A free fall: every reading
  // under kFreeFallMps2 since
  uint32_t spinSinceMs_ = 0;
  uint32_t spinLastMs_ = 0;
  bool weightless_ = false;
  uint32_t weightlessSinceMs_ = 0;
  uint32_t weightlessLastMs_ = 0;

  // the orientation at the first reading of each of the last few seconds, by second
  static constexpr uint32_t kTurnSlots = ChipSatConfig::kTurnSpanMs / 1000 + 2;
  ChipSatSensors::Quaternionf turn_[kTurnSlots];
  uint32_t turnSecond_[kTurnSlots] = {};
};

} // namespace ChipSatApp
