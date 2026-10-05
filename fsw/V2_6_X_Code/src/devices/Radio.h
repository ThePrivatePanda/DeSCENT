#pragma once

#include <stddef.h>
#include <stdint.h>
#include "../platform/Status.h"
#include "../platform/System.h"

namespace ChipSatDevices
{

// The STM32WL's built in LoRa radio. Setup is split into steps so the caller can log each one
class Radio
{
public:
  explicit Radio(ChipSatPlatform::System &system);

  ChipSatPlatform::Status begin();
  ChipSatPlatform::Status setCurrentLimit();
  float currentLimitMa();   // reads it back over SPI
  ChipSatPlatform::Status setOutputPower();
  // Also puts the current limit back to 60 mA LP / 140 mA HP (DS_SX1261-2 Rev 2.2 Table 5-2)
  ChipSatPlatform::Status applyPaConfig();

  // Transmit in three steps so the caller can do the waiting, see FlightController::transmit
  ChipSatPlatform::Status startTransmit(const uint8_t *data, size_t length);
  bool transmitDone();
  ChipSatPlatform::Status finishTransmit();   // clears the flags, back to standby
  uint32_t timeOnAirMs(size_t length);

  // Raw codes for logging, RadioLib's or ours depending on the driver
  int16_t lastCode() const { return lastCode_; }
  int16_t ldroCode() const { return ldroCode_; }

private:
  ChipSatPlatform::System &system_;
  int16_t lastCode_ = 0;
  int16_t ldroCode_ = 0;
};

} // namespace ChipSatDevices
