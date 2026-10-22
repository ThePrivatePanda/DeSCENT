#pragma once

#include <stdint.h>

namespace ChipSatApp
{

// One sensor's recovery ladder, hardware-free. retry (driver re-reads only) -> reset(s) -> aside
// for a backoff, then the resets again. Any good reading goes back to ok
enum class Seen : uint8_t
{
  Good,
  Bad,
  Unknown,   // just started, waking, or the driver restarting the part itself
};

enum class Rung : uint8_t { Ok, Retry, Due, Settling, Aside };

enum class WatchChange : uint8_t
{
  None,
  WentBad,
  Deferred,   // reset held, part still partly works and we might be flying
  Aside,
  Recovered,
};

struct WatchRules
{
  uint32_t retryMs;
  uint32_t settleMs;
  uint8_t resets;   // 1 or 2 (soft then full)
};

class SensorWatch
{
public:
  explicit SensorWatch(const WatchRules &rules) : rules_(rules) {}

  // partial: some of it still works. air: such a part is left alone until the ground
  WatchChange observe(uint32_t nowMs, Seen seen, bool partial, bool air);

  bool due() const { return rung_ == Rung::Due && !held_; }
  uint8_t resetIndex() const { return reset_; }
  void didReset(uint32_t nowMs);

  Rung rung() const { return rung_; }
  uint8_t attempts() const { return attempts_; }   // resets since last good
  uint32_t backoffMs() const { return backoffMs_; }
  uint32_t badSinceMs() const { return badSinceMs_; }

private:
  WatchRules rules_;
  Rung rung_ = Rung::Ok;
  uint8_t reset_ = 0;
  uint8_t attempts_ = 0;
  bool held_ = false;   // deferred, already logged
  uint32_t badSinceMs_ = 0;
  uint32_t sinceMs_ = 0;   // start of the current rung
  uint32_t backoffMs_ = 0;
};

} // namespace ChipSatApp
