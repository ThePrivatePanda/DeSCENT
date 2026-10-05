#pragma once

#include <stdint.h>

namespace ChipSatPlatform
{

// RCC->CSR as read at boot
struct ResetCause
{
  uint32_t csr = 0;
  bool lowPower = false;
  bool windowWatchdog = false;
  bool independentWatchdog = false;
  bool software = false;
  bool brownOut = false;
  bool pin = false;
  bool optionByteLoad = false;
  bool radioIllegalAccess = false;
};

class System
{
public:
  uint32_t nowMs() const;   // wraps after ~49.7 days, compare as now - start
  void waitMs(uint32_t ms) const;   // runs the onWait function the whole time

  // for the GPS port, it only buffers 64 bytes
  void onWait(void (*idle)()) { idle_ = idle; }

  // Can't be stopped once started. waitMs feeds it, so only a hang with no wait in it trips it,
  // like a spin inside the HAL
  void startWatchdog(uint32_t timeoutMs);
  void feedWatchdog() const;

  // Clears the flags so the next boot only shows its own reset cause
  ResetCause readAndClearResetCause();

private:
  void (*idle_)() = nullptr;
};

} // namespace ChipSatPlatform
