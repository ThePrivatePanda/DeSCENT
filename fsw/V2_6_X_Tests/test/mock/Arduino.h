// Host stand-in for the STM32 Arduino core (2.12.0): only what V2.5 Telemetry.{h,cpp}
// and the headers they pull in (Sensors.h, Debug.h) need to compile on a laptop.
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <math.h>
#include <string>

// Copied verbatim from cores/arduino/wiring_constants.h:59 and :79 (core 2.12.0).
#define RAD_TO_DEG 57.295779513082320876798154814105
#define constrain(amt,low,high) ((amt)<(low)?(low):((amt)>(high)?(high):(amt)))

#define F(s) (s)
#define DEC 10
#define HEX 16

// Stream that records everything printed, so print helpers can be checked too.
// Number formatting follows Arduino Print::printNumber (uppercase hex, no padding);
// println() ends lines with "\r\n" like Arduino Print::println.
class Stream {
 public:
  std::string out;
  void print(const char *s) { out += s; }
  void print(char c) { out += c; }
  void print(unsigned long v, int base = DEC) { out += num(v, base); }
  void print(long v, int base = DEC) {
    if (v < 0 && base == DEC) { out += '-'; out += num(0UL - (unsigned long)v, DEC); }
    else out += num((unsigned long)v, base);
  }
  void print(unsigned int v, int base = DEC) { print((unsigned long)v, base); }
  void print(int v, int base = DEC) { print((long)v, base); }
  void print(unsigned char v, int base = DEC) { print((unsigned long)v, base); }
  void println() { out += "\r\n"; }
  template <typename T> void println(T v) { print(v); println(); }
  template <typename T> void println(T v, int base) { print(v, base); println(); }
 private:
  static std::string num(unsigned long v, int base) {
    char buf[70]; int i = 69; buf[i] = 0;
    do { int d = (int)(v % base); buf[--i] = (char)(d < 10 ? '0' + d : 'A' + d - 10); v /= base; } while (v);
    return std::string(&buf[i]);
  }
};

// Sensors.h holds a HardwareSerial by value, so the type must be complete.
class HardwareSerial : public Stream {
 public:
  HardwareSerial(uint32_t, uint32_t) {}
};

// port only: pin names and levels config.h mentions. values dont matter on the laptop
#define LOW 0
#define HIGH 1
#define PA4 4
#define PA5 5
#define PA9 9
#define PA10 10
#define PA15 15
#define PB9 25
#define PB13 29
#define PB14 30
#define PB6 22
#define PB7 23
#define PB15 31
#define PC0 32
#define PC1 33
