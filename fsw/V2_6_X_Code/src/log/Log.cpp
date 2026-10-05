#include <Arduino.h>
#include <math.h>
#include "Log.h"

namespace ChipSatLog
{

// No printf: newlib nano on this board can't print floats
static ChipSatPlatform::Uart *console_ = nullptr;
static const ChipSatPlatform::System *system_ = nullptr;
static uint32_t cycle_ = 0;

static const char *tagName(Tag tag)
{
  switch (tag) {
    case Tag::Boot: return "BOOT";
    case Tag::Rst: return "RST";
    case Tag::Imu: return "IMU";
    case Tag::Gps: return "GPS";
    case Tag::Soc: return "SOC";
    case Tag::Env: return "ENV";
    case Tag::Radio: return "RADIO";
    case Tag::Tx: return "TX";
    case Tag::Cyc: return "CYC";
    case Tag::Pkt: return "PKT";
    case Tag::Flash: return "FLASH";
  }
  return "?";
}

static void writeText(const char *s)
{
  if (console_ != nullptr) {
    console_->write(s);
  }
}

static void writeChar(char c)
{
  if (console_ != nullptr) {
    const uint8_t b = static_cast<uint8_t>(c);
    console_->write(&b, 1);
  }
}

static void writeU32(uint32_t value)
{
  char buf[11];
  int i = sizeof(buf) - 1;
  buf[i] = '\0';
  do {
    buf[--i] = static_cast<char>('0' + value % 10);
    value /= 10;
  } while (value != 0);
  writeText(&buf[i]);
}

static void writeI32(int32_t value)
{
  if (value < 0) {
    writeChar('-');
    writeU32(0u - static_cast<uint32_t>(value));   // fine for INT32_MIN too
  } else {
    writeU32(static_cast<uint32_t>(value));
  }
}

static void writeFixed(int32_t value, uint8_t decimals)
{
  if (decimals > 9) {
    decimals = 9;   // 10^10 won't fit in a uint32
  }
  const uint32_t mag = value < 0 ? 0u - static_cast<uint32_t>(value) : static_cast<uint32_t>(value);
  uint32_t div = 1;
  for (uint8_t i = 0; i < decimals; ++i) {
    div *= 10;
  }
  if (value < 0) {
    writeChar('-');
  }
  writeU32(mag / div);
  if (decimals == 0) {
    return;
  }
  writeChar('.');
  const uint32_t frac = mag % div;
  for (uint32_t d = div / 10; d > 0; d /= 10) {
    writeChar(static_cast<char>('0' + (frac / d) % 10));
  }
}

static void writeHex(uint32_t value, uint8_t digits)
{
  static const char kHex[] = "0123456789ABCDEF";
  if (digits == 0 || digits > 8) {
    digits = 8;   // shifting a uint32 by 32 is undefined
  }
  for (int shift = (digits - 1) * 4; shift >= 0; shift -= 4) {
    writeChar(kHex[(value >> shift) & 0x0F]);
  }
}

static void writePrefix(char level, Tag tag, uint32_t ms)
{
  writeChar(level);
  writeChar(' ');
  writeText(tagName(tag));
  writeChar(' ');
  writeU32(ms);
  writeText(" c");
  writeU32(cycle_);
  writeText(": ");
}

void begin(ChipSatPlatform::Uart &console, const ChipSatPlatform::System &system)
{
  console_ = &console;
  system_ = &system;
}

void setCycle(uint32_t cycle)
{
  cycle_ = cycle;
}

Line::Line(char level, Tag tag, const char *event)
  : ms_(system_ != nullptr ? system_->nowMs() : 0)
{
  writePrefix(level, tag, ms_);
  writeText(event);
}

Line::~Line()
{
  writeText("\r\n");   // CRLF, like println
}

void Line::key(const char *key)
{
  writeChar(' ');
  writeText(key);
  writeChar('=');
}

void Line::text(const char *s)
{
  writeText(s);
}

void Line::field(const char *key, bool value)
{
  this->key(key);
  writeChar(value ? '1' : '0');
}

void Line::field(const char *key, int value)
{
  this->key(key);
  writeI32(static_cast<int32_t>(value));
}

void Line::field(const char *key, unsigned int value)
{
  this->key(key);
  writeU32(static_cast<uint32_t>(value));
}

void Line::field(const char *key, long value)
{
  this->key(key);
  writeI32(static_cast<int32_t>(value));
}

void Line::field(const char *key, unsigned long value)
{
  this->key(key);
  writeU32(static_cast<uint32_t>(value));
}

void Line::field(const char *key, const char *word)
{
  this->key(key);
  writeText(word);
}

void Line::field(const char *key, float value, uint8_t decimals)
{
  this->key(key);
  if (isnan(value)) {
    writeText("nan");
    return;
  }
  if (isinf(value)) {
    writeText("inf");
    return;
  }
  if (decimals > 9) {
    decimals = 9;
  }
  double scaled = value;
  for (uint8_t i = 0; i < decimals; ++i) {
    scaled *= 10.0;
  }
  scaled = scaled < 0 ? ceil(scaled - 0.5) : floor(scaled + 0.5);   // half away from zero
  if (scaled > 2147483647.0 || scaled < -2147483648.0) {
    writeText("ovf");
    return;
  }
  // The last digit can differ from Print::print(float) when rounding
  writeFixed(static_cast<int32_t>(scaled), decimals);
}

void Line::fieldFixed(const char *key, int32_t scaled, uint8_t decimals)
{
  this->key(key);
  writeFixed(scaled, decimals);
}

void Line::fieldHex(const char *key, uint32_t value, uint8_t digits)
{
  this->key(key);
  writeText("0x");
  writeHex(value, digits);
}

void Line::fieldBits(const char *key, uint8_t bits, uint8_t count)
{
  this->key(key);
  for (uint8_t i = 0; i < count; ++i) {
    writeChar((bits & (1U << i)) ? '1' : '0');
  }
}

void Line::fieldList(const char *key, const uint32_t *values, uint8_t count)
{
  this->key(key);
  for (uint8_t i = 0; i < count; ++i) {
    if (i > 0) {
      writeChar(',');
    }
    writeU32(values[i]);
  }
}

void packetLine(const uint8_t *bytes, size_t length)
{
  // Label and bytes only. Don't print another hex line right after, the decoder would merge them
  writePrefix('I', Tag::Pkt, system_ != nullptr ? system_->nowMs() : 0);
  for (size_t i = 0; i < length; ++i) {
    if (i > 0) {
      writeChar(' ');
    }
    writeHex(bytes[i], 2);
  }
  writeText("\r\n");
}

} // namespace ChipSatLog
