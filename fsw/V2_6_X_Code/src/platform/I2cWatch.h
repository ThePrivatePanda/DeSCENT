#pragma once

#include <stdint.h>
#include "Status.h"

namespace ChipSatPlatform
{

// I2C bus watchdog, no hardware in here so it runs on the laptop (pins and Wire are in I2cBusArduino.cpp)
//   held     a line low before a transfer, transfer not tried, clear due
//   jam      a line low right after a transfer that started idle, blamed on that address
//   timeout  failed transfer that took kBusSlowMs or more, i.e. it spun in the HAL
// A part holding a line for good can't be freed from software, and a reboot doesn't power-cycle the sensors

// Open drain: letting go reads high unless a part holds the line
class BusLines
{
public:
  virtual bool sda() = 0;
  virtual bool scl() = 0;
  virtual void sdaLow(bool low) = 0;
  virtual void sclLow(bool low) = 0;
  virtual void halfClock() = 0;   // kI2cHalfClockUs
};

struct BusClear
{
  bool sdaBefore = true;
  bool sclBefore = true;
  uint8_t pulses = 0;
  bool sclHeld = false;   // clocking can't help then
  bool ok = false;        // both lines high at the end
};

// UM10204 3.1.16: up to 9 clocks until SDA is free, then a STOP. Under 12 ms worst case
BusClear clearBus(BusLines &lines);

enum class BusGate : uint8_t
{
  Go,
  Held,      // line low, or Wire still down after a failed clear
  Refused,   // part quarantined or held back
};

class BusWatch
{
public:
  BusGate before(uint8_t address, bool sda, bool scl, bool wireUp, uint32_t nowMs);
  void after(uint8_t address, Status status, uint32_t tookMs, bool sda, bool scl, uint32_t nowMs);

  bool clearDue(uint32_t nowMs) const;
  void cleared(const BusClear &result, uint32_t nowMs);

  // also true for kBusSettleMs after a clear, so the parts get a chance before their own resets
  bool faulted(uint32_t nowMs) const;

  bool quarantined(uint8_t address, uint32_t nowMs) const;
  uint32_t clears() const { return clears_; }

private:
  static constexpr uint8_t kSlots = 4;   // imu, env, soc, everything else shares one (bench scan)
  struct Part
  {
    uint8_t jams = 0;
    uint32_t firstJamMs = 0;
    bool quarantined = false;
    uint32_t quarantineUntilMs = 0;
    uint32_t quarantineMs = 0;   // 0 before the first
    uint8_t timeouts = 0;        // in a row
    bool held = false;
    uint32_t holdUntilMs = 0;
    uint32_t holdMs = 0;
    bool failedRecently = false;
    uint32_t failedMs = 0;
  };
  static uint8_t slot(uint8_t address);
  void jam(uint8_t address, bool sda, bool scl, uint32_t nowMs);

  Part parts_[kSlots];
  bool linesHeld_ = false;
  bool clearNeeded_ = false;
  bool clearedAny_ = false;
  uint32_t clearedMs_ = 0;
  uint32_t clearRetryMs_ = 0;
  uint32_t clears_ = 0;
};

const char *busPartName(uint8_t address);

} // namespace ChipSatPlatform
