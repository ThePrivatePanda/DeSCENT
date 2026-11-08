#pragma once

#include <stdint.h>
#include "../config.h"

namespace ChipSatApp
{

// Where the sat is in its flight. A board is switched on mounted on the drone, often long before it
// flies, so the phase sets how fast it transmits and how much the flash log keeps: slow and a trickle
// on the ground, fast and everything in the air. Both come from the settings block, a transmit rate
// and a log rate for each phase. Only the fall matters (decided 2026-11-07), so the IMU runs at 1 Hz
// on the ground and at 20 Hz from the first sign of movement to the landing
//
//   Ground  switched on, and again once it has come to rest. kWakeMoving of the last kWakeReadings
//           at 1 Hz moving puts the IMU at 20 Hz, and kWakeQuietMs without one back to 1 Hz. At
//           20 Hz, the IMU moving for most of 10 s takes it to the air, as long as the altitude, if
//           one is live, has also climbed kRiseGain. The window held kMotionHeldMs is the air with
//           no climb, for a pressurised cabin
//   Air     ends when no IMU reading has moved for kLandedMs (30 s), the barometer is not falling,
//           and a live altitude has been flat those 30 s below half the climb. Flat and quiet for
//           kLandedHighMs (5 min) is the ground at any height: a tree, a roof. With no altitude
//           live all through the air, kGroundQuietMs (3 min) quiet instead: the BME280 reads
//           nothing below 300 hPa, about 9 km, so a high fall that doesn't tumble has nothing else
//           to hold it up
//
// The barometer alone never puts a board in the air. On 2026-11-01 its readings during 22 dBm
// transmits read about 46 m high, which took desk boards to the air and kept one there 46 minutes.
// So the climb and the flat are both judged on each second's lowest and highest reading: the
// climb on the lowest, which the offset leaves alone, and the flat on both staying within
// kFlatAltitude. A fall holds a board in the air when its readings fall 5 m/s for 3 s on both their
// lowest and highest in each second, which an offset coming and going can't do. The IMU takes 8 s
// of a running drone to call the air, 20 s on board 9's second drop of 2026-10-17 where the drone
// often read quiet, and the flash log's 30 s ring keeps the take-off either way
//
// A gap in the IMU holds the phase and counts as no movement. Missing for kImuDeadMs, the phase
// goes back to altitude: a climb of kRiseGain is the air, kLandedNoImuMs still below half of it
// the ground
//
// Known limits: a board in water keeps moving and stays in the air, the quiet lines were set on the
// old boards (grass in wind is not measured on the new ones), and a transmit offset that lifts some
// seconds and not others reads as not flat
//
// Same values as the old Waiting (0) and Rising (1) so old records still read. An old 3 (Landed) is
// the ground, an old 2 (Falling, never entered) the air
enum class Phase : uint8_t
{
  Ground = 0,
  Air = 1,
};

// After a reset the log starts in the air so a reset mid-flight still keeps the fall (decided
// 2026-10-22), the radio on the ground. Boot-air ends at rest, in the air or at kBootAirMaxMs
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

  // Called when the IMU's data is older than the controller trusts, every loop until it is back
  void imuLost(uint32_t nowMs);

  Phase phase() const { return phase_; }   // what the radio follows

  // True once after each change of phase, so the controller can act on it (transmit at once)
  bool changed();

  bool logsAir() const { return phase_ == Phase::Air || bootAir_; }   // what the flash log follows
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
  uint8_t landing(uint32_t nowMs, Track *t, bool imuDead);   // the kWhy code, 0 still in the air
  bool stillOnAltitude(uint32_t nowMs, Track *t, bool climbed);
  bool descending(uint32_t nowMs) const;
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

  // The last kMotionWindowMs of IMU readings by time, one bit each for whether a slot got a reading
  // and whether it moved. The slots a gap skips are empty, so a slow feed can't stretch the window
  static constexpr uint32_t kMotionSlots = ChipSatConfig::kMotionWindowMs / ChipSatConfig::kMotionSlotMs;
  uint8_t received_[kMotionSlots / 8] = {};
  uint8_t moved_[kMotionSlots / 8] = {};
  uint32_t lastSlot_ = 0;
  uint16_t receivedCount_ = 0;
  uint16_t movedCount_ = 0;
  uint32_t motionSinceMs_ = 0;   // the window has said moving from here on, continuously

  uint8_t recent_ = 0;   // the last kWakeReadings at 1 Hz, newest in bit 0, a bit for each that moved

  bool imuValid_ = false;
  uint32_t imuLostMs_ = 0;   // the IMU missing since; from boot until its first reading
  uint32_t stillSinceMs_ = 0;   // the last moving reading, falling reading or take-off

  // the spell at rest on altitude that ends a flight once the IMU is dead
  uint32_t quietSinceMs_ = 0;
  float quietAltM_ = 0.0f;
  bool inQuiet_ = false;

  bool falling_ = false;   // descending() on the last judge, for the event when it starts
};

} // namespace ChipSatApp
