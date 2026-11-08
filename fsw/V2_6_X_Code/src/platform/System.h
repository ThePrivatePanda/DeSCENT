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
  void waitMs(uint32_t ms) const;   // runs the onWait function the whole time, at least once

  // Sleeps until the next interrupt, the 1 ms tick at the latest (config.h kSleepWhenIdle)
  void idleUntilTick() const;

  // Stop2 for ms, kSleepMaxMs at most, then back on the core's clock with nowMs() moved on by what
  // it slept, which it returns. Everything but LSI stops: nothing is received meanwhile, and anything
  // still being sent is cut off. Feeds the watchdog first
  uint32_t sleepFor(uint32_t ms) const;

  // for the GPS port, it only buffers 64 bytes
  void onWait(void (*idle)()) { idle_ = idle; }

  // Can't be stopped once started. waitMs feeds it, so only a hang with no wait in it trips it,
  // like a spin inside the HAL
  void startWatchdog(uint32_t timeoutMs);
  void feedWatchdog() const;

  // Clears the flags so the next boot only shows its own reset cause
  ResetCause readAndClearResetCause();

  // Stack high-water mark. paintStack fills the free RAM with a pattern at boot, stackUntouchedBytes
  // counts what's still the pattern. A few hundred us, fine once a packet
  void paintStack();
  uint32_t stackUntouchedBytes() const;

private:
  void (*idle_)() = nullptr;
};

} // namespace ChipSatPlatform
