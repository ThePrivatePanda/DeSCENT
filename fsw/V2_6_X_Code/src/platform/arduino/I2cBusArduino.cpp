#include <Arduino.h>
#include <Wire.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_I2C_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include "../I2cBus.h"
#include "../I2cWatch.h"
#include "../../log/Log.h"

namespace ChipSatPlatform
{

using namespace ChipSatConfig;
using namespace ChipSatConstants;

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
static bool wireUp = false;
static BusWatch watch;

// IDR reads the pin in AF open drain and output open drain alike. Low gets 50 us to come back
static bool linesIdle(bool &sdaHigh, bool &sclHigh)
{
  for (uint8_t i = 0; i < 6; ++i) {
    sdaHigh = digitalRead(sda) == HIGH;
    sclHigh = digitalRead(scl) == HIGH;
    if (sdaHigh && sclHigh) {
      return true;
    }
    delayMicroseconds(10);
  }
  return false;
}

class PinLines : public BusLines
{
public:
  bool sda() override { return digitalRead(ChipSatPlatform::sda) == HIGH; }
  bool scl() override { return digitalRead(ChipSatPlatform::scl) == HIGH; }
  void sdaLow(bool low) override { digitalWrite(ChipSatPlatform::sda, low ? LOW : HIGH); }
  void sclLow(bool low) override { digitalWrite(ChipSatPlatform::scl, low ? LOW : HIGH); }
  void halfClock() override { delayMicroseconds(kI2cHalfClockUs); }
};

static void startWire()
{
  // pins before begin(), otherwise Wire grabs PA9 (the LED)
  Wire.setSDA(sda);
  Wire.setSCL(scl);
  Wire.begin();
  wireUp = true;
}

static Status gate(uint8_t address)
{
  bool sdaHigh = true;
  bool sclHigh = true;
  linesIdle(sdaHigh, sclHigh);
  switch (watch.before(address, sdaHigh, sclHigh, wireUp, millis())) {
    case BusGate::Go: return Status::Ok;
    case BusGate::Held: return Status::BusError;
    case BusGate::Refused: return Status::NotReady;
  }
  return Status::BusError;
}

// reads too, the IMU only ever reads
static Status settle(uint8_t address, Status status, uint32_t startMs)
{
  bool sdaHigh = true;
  bool sclHigh = true;
  linesIdle(sdaHigh, sclHigh);
  const uint32_t now = millis();
  watch.after(address, status, now - startMs, sdaHigh, sclHigh, now);
  return status;
}

static Status readNow(uint8_t address, uint8_t *buffer, size_t length)
{
  if (Wire.requestFrom(address, static_cast<uint8_t>(length)) != length) {
    return Status::BusError;   // NACK or timeout, settle() tells them apart by time
  }
  for (size_t i = 0; i < length; ++i) {
    buffer[i] = static_cast<uint8_t>(Wire.read());
  }
  return Status::Ok;
}

Status I2cBus::begin(uint32_t sdaPin, uint32_t sclPin)
{
  sda = sdaPin;
  scl = sclPin;
  startWire();
  Wire.flush();
  return Status::Ok;
}

Status I2cBus::probe(uint8_t address)
{
  const Status gated = gate(address);
  if (gated != Status::Ok) {
    return gated;
  }
  const uint32_t startMs = millis();
  Wire.beginTransmission(address);
  return settle(address, fromEndTransmission(Wire.endTransmission()), startMs);
}

Status I2cBus::write(uint8_t address, const uint8_t *data, size_t length)
{
  const Status gated = gate(address);
  if (gated != Status::Ok) {
    return gated;
  }
  const uint32_t startMs = millis();
  Wire.beginTransmission(address);
  if (Wire.write(data, length) != length) {
    Wire.endTransmission();
    return settle(address, Status::BusError, startMs);
  }
  return settle(address, fromEndTransmission(Wire.endTransmission()), startMs);
}

Status I2cBus::read(uint8_t address, uint8_t *buffer, size_t length)
{
  if (length > 255) {
    return Status::BadData;   // requestFrom only takes a uint8 count
  }
  const Status gated = gate(address);
  if (gated != Status::Ok) {
    return gated;
  }
  const uint32_t startMs = millis();
  return settle(address, readNow(address, buffer, length), startMs);
}

// Lines checked before the write only. After a write with no STOP the master holds SCL low
// until the repeated start, a check in between would always see it held
Status I2cBus::writeThenRead(uint8_t address, const uint8_t *txData, size_t txLength,
                             uint8_t *rxBuffer, size_t rxLength)
{
  if (rxLength > 255) {
    return Status::BadData;
  }
  const Status gated = gate(address);
  if (gated != Status::Ok) {
    return gated;
  }
  const uint32_t startMs = millis();
  Wire.beginTransmission(address);
  Wire.write(txData, txLength);
  const Status sent = fromEndTransmission(Wire.endTransmission(false));
  if (sent != Status::Ok) {
    return settle(address, sent, startMs);   // NACK ends in an automatic STOP
  }
  return settle(address, readNow(address, rxBuffer, rxLength), startMs);
}

// Wire stays down until both lines are free: Wire.begin() runs the core's recoverBus(), which
// drives SCL push-pull, and that shouldn't fight a part holding SCL
bool I2cBus::recover(uint32_t nowMs)
{
  if (!watch.clearDue(nowMs)) {
    return false;
  }
  if (wireUp) {
    Wire.end();
    wireUp = false;
  }
  digitalWrite(sda, HIGH);
  digitalWrite(scl, HIGH);
  pinMode(sda, OUTPUT_OPEN_DRAIN);
  pinMode(scl, OUTPUT_OPEN_DRAIN);
  PinLines lines;
  const BusClear result = clearBus(lines);
  if (result.ok) {
    startWire();
  } else {
    pinMode(sda, INPUT);   // read only until the next clear
    pinMode(scl, INPUT);
  }
  watch.cleared(result, millis());
  return true;
}

bool I2cBus::faulted(uint32_t nowMs) const
{
  return watch.faulted(nowMs);
}

TwoWire &I2cBus::arduinoWire()
{
  return Wire;
}

} // namespace ChipSatPlatform

#endif
