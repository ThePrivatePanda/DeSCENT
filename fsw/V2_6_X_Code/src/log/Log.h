#pragma once

#include <stddef.h>
#include <stdint.h>
#include "../config.h"
#include "../platform/System.h"
#include "../platform/Uart.h"

// Log lines look like: I GPS 131263 c42: read ok=1 fix=3 valid=1 ms=231
// Usage: LOG_I(Gps, "read") { line.field("ok", ok); }
// Lines above CHIPSAT_LOG_LEVEL aren't compiled in. Don't change any state inside the braces
// Event words need a letter past 'f' so a line never looks like packet hex

namespace ChipSatLog
{

enum class Tag : uint8_t { Boot, Rst, Imu, Gps, Soc, Env, Radio, Tx, Cyc, Pkt, Flash };

void begin(ChipSatPlatform::Uart &console, const ChipSatPlatform::System &system);
void setCycle(uint32_t cycle);

class Line
{
public:
  Line(char level, Tag tag, const char *event);
  ~Line();
  Line(const Line &) = delete;
  Line &operator=(const Line &) = delete;

  uint32_t ms() const { return ms_; }   // the time printed on this line

  void field(const char *key, bool value);
  void field(const char *key, int value);
  void field(const char *key, unsigned int value);
  void field(const char *key, long value);
  void field(const char *key, unsigned long value);
  void field(const char *key, const char *word);
  void field(const char *key, float value, uint8_t decimals);
  void fieldFixed(const char *key, int32_t scaled, uint8_t decimals);   // 424433500, 7 decimals -> 42.4433500
  void fieldHex(const char *key, uint32_t value, uint8_t digits);
  void fieldBits(const char *key, uint8_t bits, uint8_t count);         // bit 0 printed first
  void fieldList(const char *key, const uint32_t *values, uint8_t count);
  void key(const char *key);   // writes " key=", add the value with text()
  void text(const char *s);

private:
  uint32_t ms_;
};

void packetLine(const uint8_t *bytes, size_t length);

} // namespace ChipSatLog

// The {} else keeps a following else bound to the caller's if. The switch runs the block once
#define CHIPSAT_LOG_AT(minLevel, letter, tag, event) \
  if constexpr (!(CHIPSAT_LOG_LEVEL >= (minLevel))) {} else \
    switch (ChipSatLog::Line line{letter, ChipSatLog::Tag::tag, event}; 0) default:

#define LOG_E(tag, event) CHIPSAT_LOG_AT(1, 'E', tag, event)
#define LOG_W(tag, event) CHIPSAT_LOG_AT(2, 'W', tag, event)
#define LOG_I(tag, event) CHIPSAT_LOG_AT(3, 'I', tag, event)
#define LOG_D(tag, event) CHIPSAT_LOG_AT(4, 'D', tag, event)

// Boot lines print at every level except 0
#define LOG_BOOT(tag, event) CHIPSAT_LOG_AT(1, 'I', tag, event)

#define LOG_PACKET(bytes, length) \
  if constexpr (!(CHIPSAT_LOG_LEVEL >= 3)) {} else ChipSatLog::packetLine(bytes, length)
