#pragma once

#include <stddef.h>
#include <stdint.h>
#include "Status.h"

class Stream;

namespace ChipSatPlatform
{

// Console = USART1 PB7/PB6, GPS = LPUART1 PC1/PC0
enum class UartPort : uint8_t { Console, Gps };

class Uart
{
public:
  explicit Uart(UartPort port);

  Status begin(uint32_t baud);

  // Blocks when the 64 byte TX buffer is full
  size_t write(const uint8_t *data, size_t length);
  size_t write(const char *text);

  size_t available();
  int read();   // -1 when empty
  void flush();

  // For the u-blox library
  Stream &arduinoStream();

private:
  UartPort port_;
};

} // namespace ChipSatPlatform
