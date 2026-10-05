#pragma once

#include <stdint.h>

namespace ChipSatPlatform
{

// Returned by every driver call. Library error codes don't leave the driver
enum class Status : uint8_t
{
  Ok = 0,
  NotReady,   // failed at boot, or asleep
  Timeout,
  BusError,
  NoAck,
  BadData,    // got an answer but it's no good (NaN, no fix)
  Failed
};

inline const char *statusName(Status status)
{
  switch (status) {
    case Status::Ok: return "ok";
    case Status::NotReady: return "notready";
    case Status::Timeout: return "timeout";
    case Status::BusError: return "buserror";
    case Status::NoAck: return "noack";
    case Status::BadData: return "baddata";
    case Status::Failed: return "failed";
  }
  return "unknown";
}

} // namespace ChipSatPlatform
