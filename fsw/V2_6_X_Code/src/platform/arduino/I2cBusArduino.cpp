#include <Arduino.h>
#include <Wire.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_I2C_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include "../I2cBus.h"
#include "../../log/Log.h"

namespace ChipSatPlatform
{

// endTransmission codes: 0 ok, 1 too long, 2 nack on address, 3 nack on data, 4 other, 5 timeout
static Status fromEndTransmission(uint8_t code)
{
  switch (code) {
    case 0: return Status::Ok;
    case 2:
    case 3: return Status::NoAck;
    case 5: return Status::Timeout;
    default: return Status::BusError;
  }
}

static uint32_t sda = 0;
static uint32_t scl = 0;
static uint8_t stuck = 0;   // timeouts in a row

// A part that resets mid-read can hold SDA low, then every transfer times out after 100 ms for the
// rest of the flight. Wire only frees the bus (9 clocks and a stop) in begin(), so start it again
static Status watch(Status status)
{
  if (status != Status::Timeout && status != Status::BusError) {
    stuck = 0;
    return status;
  }
  if (++stuck >= ChipSatConstants::kI2cStuckTransfers) {
    stuck = 0;
    Wire.end();
    Wire.setSDA(sda);
    Wire.setSCL(scl);
    Wire.begin();
    LOG_W(Cyc, "i2crecover");
  }
  return status;
}

Status I2cBus::begin(uint32_t sdaPin, uint32_t sclPin)
{
  sda = sdaPin;
  scl = sclPin;
  // Pins before begin(), otherwise Wire grabs PA9 (the LED)
  Wire.setSDA(sdaPin);
  Wire.setSCL(sclPin);
  Wire.begin();
  Wire.flush();
  return Status::Ok;
}

Status I2cBus::probe(uint8_t address)
{
  Wire.beginTransmission(address);
  return watch(fromEndTransmission(Wire.endTransmission()));
}

Status I2cBus::write(uint8_t address, const uint8_t *data, size_t length)
{
  Wire.beginTransmission(address);
  if (Wire.write(data, length) != length) {
    Wire.endTransmission();
    return Status::BusError;
  }
  return watch(fromEndTransmission(Wire.endTransmission()));
}

Status I2cBus::read(uint8_t address, uint8_t *buffer, size_t length)
{
  if (length > 255) {
    return Status::BadData;   // requestFrom only takes a uint8 count
  }
  if (Wire.requestFrom(address, static_cast<uint8_t>(length)) != length) {
    return Status::BusError;
  }
  for (size_t i = 0; i < length; ++i) {
    buffer[i] = static_cast<uint8_t>(Wire.read());
  }
  return Status::Ok;
}

Status I2cBus::writeThenRead(uint8_t address, const uint8_t *txData, size_t txLength,
                             uint8_t *rxBuffer, size_t rxLength)
{
  Wire.beginTransmission(address);
  Wire.write(txData, txLength);
  const Status sent = watch(fromEndTransmission(Wire.endTransmission(false)));
  if (sent != Status::Ok) {
    return sent;
  }
  return read(address, rxBuffer, rxLength);
}

TwoWire &I2cBus::arduinoWire()
{
  return Wire;
}

} // namespace ChipSatPlatform

#endif
