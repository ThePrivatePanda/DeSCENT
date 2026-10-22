#include "SensorWatch.h"
#include "../config.h"

namespace ChipSatApp
{

using namespace ChipSatConfig;

WatchChange SensorWatch::observe(uint32_t nowMs, Seen seen, bool partial, bool air)
{
  if (seen == Seen::Unknown) {
    return WatchChange::None;   // clocks keep running, a restart counts against its settle
  }
  if (seen == Seen::Good) {
    const bool wasBad = rung_ != Rung::Ok;
    rung_ = Rung::Ok;
    reset_ = 0;
    held_ = false;
    backoffMs_ = 0;
    if (!wasBad) {
      return WatchChange::None;
    }
    return WatchChange::Recovered;   // attempts() kept for the log line
  }

  switch (rung_) {
    case Rung::Ok:
      rung_ = Rung::Retry;
      attempts_ = 0;
      badSinceMs_ = nowMs;
      sinceMs_ = nowMs;
      return WatchChange::WentBad;

    case Rung::Retry:
      if (nowMs - sinceMs_ < rules_.retryMs) {
        return WatchChange::None;
      }
      rung_ = Rung::Due;
      reset_ = 0;
      break;   // so this pass can already defer it

    case Rung::Settling:
      if (nowMs - sinceMs_ < rules_.settleMs) {
        return WatchChange::None;
      }
      if (reset_ + 1 < rules_.resets) {
        ++reset_;
        rung_ = Rung::Due;
        break;
      }
      rung_ = Rung::Aside;
      sinceMs_ = nowMs;
      {
        const uint32_t ceiling = air ? kRecoverBackoffAirMs : kRecoverBackoffGroundMs;
        backoffMs_ = backoffMs_ == 0 ? kRecoverBackoffMs : backoffMs_ * 2;
        if (backoffMs_ > ceiling) {
          backoffMs_ = ceiling;
        }
      }
      return WatchChange::Aside;

    case Rung::Aside: {
      // ceiling follows the phase now, a take-off cuts a long ground aside short
      const uint32_t ceiling = air ? kRecoverBackoffAirMs : kRecoverBackoffGroundMs;
      const uint32_t wait = backoffMs_ < ceiling ? backoffMs_ : ceiling;
      if (nowMs - sinceMs_ < wait) {
        return WatchChange::None;
      }
      rung_ = Rung::Due;
      reset_ = 0;
      break;
    }

    case Rung::Due:
      break;
  }

  // in the air only a dead part gets reset
  if (air && partial) {
    if (!held_) {
      held_ = true;
      return WatchChange::Deferred;
    }
    return WatchChange::None;
  }
  held_ = false;
  return WatchChange::None;
}

void SensorWatch::didReset(uint32_t nowMs)
{
  rung_ = Rung::Settling;
  sinceMs_ = nowMs;
  held_ = false;
  if (attempts_ < 0xFF) {
    ++attempts_;
  }
}

} // namespace ChipSatApp
