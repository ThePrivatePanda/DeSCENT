#include <Arduino.h>
#include "../config.h"
#include "../constants.h"
#include "Led.h"

namespace ChipSatDevices
{

void Led::begin()
{
  pinMode(ChipSatConstants::kLedPin, OUTPUT);
  digitalWrite(ChipSatConstants::kLedPin, ChipSatConfig::kLedLevelAtBoot);
}

void Led::transmitStarted()
{
  digitalWrite(ChipSatConstants::kLedPin, ChipSatConfig::kLedLevelDuringTx);
}

void Led::transmitEnded()
{
  digitalWrite(ChipSatConstants::kLedPin, ChipSatConfig::kLedLevelAfterTx);
}

} // namespace ChipSatDevices
