#pragma once

#include <stdint.h>

namespace ChipSatApp
{

// Has the sat come to rest. Free fall reads the same as sitting still on the accelerometer, so the
// pressure has to hold steady as well, for a while, and never before the sat has had time to fly.
// Landed only slows the logging down, it never changes anything about transmitting
class Landing
{
public:
  // accelerationMps2 is the magnitude of the linear acceleration, pressureHpa the last reading.
  // valid says whether both readings can be believed this time
  void update(uint32_t nowMs, float accelerationMps2, float pressureHpa, bool valid);

  bool landed() const { return landed_; }
  uint32_t quietMs() const { return quietMs_; }

private:
  bool landed_ = false;
  bool haveReference_ = false;
  uint32_t quietSinceMs_ = 0;
  uint32_t quietMs_ = 0;
  float referencePressureHpa_ = 0.0f;
};

} // namespace ChipSatApp
