#pragma once

#include <stddef.h>
#include <stdint.h>
#include "Status.h"

class TwoWire;

namespace ChipSatPlatform
{

// The one I2C bus (IMU, BME280, fuel gauge)
class I2cBus
{
public:
  // No setClock, it runs at the core default 100 kHz
  Status begin(uint32_t sdaPin, uint32_t sclPin);

  Status probe(uint8_t address);
  Status write(uint8_t address, const uint8_t *data, size_t length);
  Status read(uint8_t address, uint8_t *buffer, size_t length);
  Status writeThenRead(uint8_t address, const uint8_t *txData, size_t txLength,
                       uint8_t *rxBuffer, size_t rxLength);   // repeated start

  // The libraries need the Wire object itself
  TwoWire &arduinoWire();
};

} // namespace ChipSatPlatform
