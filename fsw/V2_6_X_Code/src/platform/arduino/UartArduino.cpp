#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_UART_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include "../Uart.h"

namespace ChipSatPlatform
{

// Constructors only store the pins
static HardwareSerial consoleSerial(ChipSatConstants::kConsoleRxPin, ChipSatConstants::kConsoleTxPin);
static HardwareSerial gpsSerial(ChipSatConstants::kGpsRxPin, ChipSatConstants::kGpsTxPin);

static HardwareSerial &serialFor(UartPort port)
{
  return port == UartPort::Console ? consoleSerial : gpsSerial;
}

Uart::Uart(UartPort port) : port_(port)
{
}

Status Uart::begin(uint32_t baud)
{
  serialFor(port_).begin(baud);
  return Status::Ok;
}

size_t Uart::write(const uint8_t *data, size_t length)
{
  return serialFor(port_).write(data, length);
}

size_t Uart::write(const char *text)
{
  return serialFor(port_).write(text);
}

size_t Uart::available()
{
  return static_cast<size_t>(serialFor(port_).available());
}

int Uart::read()
{
  return serialFor(port_).read();
}

void Uart::flush()
{
  serialFor(port_).flush();
}

Stream &Uart::arduinoStream()
{
  return serialFor(port_);
}

} // namespace ChipSatPlatform

#endif
