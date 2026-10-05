// Log.cpp on the laptop. Build it once per level, the lines it writes to the file you name go
// through descent_decode.py
//   for l in 0 1 2 3 4; do g++ -std=gnu++17 -Wall -Wextra -DCHIPSAT_LOG_LEVEL=$l -Itest/mock -I../V2_6_X_Code/src/log test/test_log.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/log$l && /tmp/log$l /tmp/log$l.txt; done
#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>
#include "Log.h"

static std::string captured;
static uint32_t fakeNow = 0;

// fake platform: the uart appends to a string, the clock is whatever the test sets
namespace ChipSatPlatform
{
Uart::Uart(UartPort port) : port_(port) {}
Status Uart::begin(uint32_t) { return Status::Ok; }
size_t Uart::write(const uint8_t *data, size_t length) { captured.append(reinterpret_cast<const char *>(data), length); return length; }
size_t Uart::write(const char *text) { captured += text; return strlen(text); }
size_t Uart::available() { return 0; }
int Uart::read() { return -1; }
void Uart::flush() {}
Stream &Uart::arduinoStream() { static Stream s; return s; }
uint32_t System::nowMs() const { return fakeNow; }
}

static int checks = 0, failures = 0;

static void expect(const std::string &want, const char *what)
{
  ++checks;
  const bool ok = captured == want;
  if (!ok) {
    ++failures;
  }
  printf("%s %s\n", ok ? "ok  " : "FAIL", what);
  if (!ok) {
    printf("     want: \"%s\"\n     got:  \"%s\"\n", want.c_str(), captured.c_str());
  }
  captured.clear();
}

static std::string at(int minLevel, const char *line)
{
  return CHIPSAT_LOG_LEVEL >= minLevel ? std::string(line) : std::string();
}

static const uint8_t kVector[55] = {
  0x5C, 0x57, 0x4C, 0x19, 0x50, 0xBD, 0x66, 0xD2, 0x90, 0xD0, 0x03, 0x00, 0xB4, 0x5F, 0x00, 0x00,
  0x7B, 0x00, 0x38, 0xFE, 0xD4, 0x03, 0x0A, 0x00, 0xEC, 0xFF, 0x1E, 0x00, 0xFA, 0x00, 0x83, 0xFF,
  0xF4, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x7F, 0x29, 0x09, 0x94, 0x27, 0xC6, 0x11,
  0x2A, 0x00, 0x09, 0x96, 0xFF, 0x93, 0x93};

static const char *kVectorLine =
  "I PKT 131265 c43: 5C 57 4C 19 50 BD 66 D2 90 D0 03 00 B4 5F 00 00 7B 00 38 FE D4 03 0A 00 EC FF 1E 00 "
  "FA 00 83 FF F4 01 00 00 00 00 00 00 FF 7F 29 09 94 27 C6 11 2A 00 09 96 FF 93 93\r\n";

int main(int argc, char **argv)
{
  const char *excerptPath = argc > 1 ? argv[1] : "excerpt.txt";
  printf("CHIPSAT_LOG_LEVEL=%d\n", CHIPSAT_LOG_LEVEL);

  ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
  ChipSatPlatform::System sys;

  // nothing before begin(), no crash
  LOG_E(Boot, "halt") { line.field("reason", "early"); }
  expect("", "before begin: no output");

  ChipSatLog::begin(console, sys);

  fakeNow = 2;
  LOG_BOOT(Boot, "start") { line.field("log", CHIPSAT_LOG_LEVEL); }
  {
    char want[64];
    snprintf(want, sizeof want, "I BOOT 2 c0: start log=%d\r\n", CHIPSAT_LOG_LEVEL);
    expect(at(1, want), "boot record prints from level 1");
  }

  fakeNow = 5;
  LOG_I(Imu, "start");
  expect(at(3, "I IMU 5 c0: start\r\n"), "event with no fields");

  fakeNow = 131263;
  ChipSatLog::setCycle(42);
  LOG_I(Gps, "read") {
    line.field("ok", true);
    line.field("fix", static_cast<uint8_t>(3));
    line.field("llhbad", false);
    line.field("valid", true);
    line.field("ms", static_cast<uint32_t>(231));
  }
  expect(at(3, "I GPS 131263 c42: read ok=1 fix=3 llhbad=0 valid=1 ms=231\r\n"), "info line, bool and unsigned fields");

  LOG_W(Imu, "timeout") {
    line.field("limitms", static_cast<uint16_t>(500));
    line.key("missing");
    line.text("linaccel");
    line.text(",");
    line.text("quat");
  }
  expect(at(2, "W IMU 131263 c42: timeout limitms=500 missing=linaccel,quat\r\n"), "warning line, key + text");

  LOG_E(Tx, "done") {
    line.field("ok", false);
    line.field("code", static_cast<int16_t>(-5));
    line.field("ms", static_cast<uint32_t>(2281));
  }
  expect(at(1, "E TX 131263 c42: done ok=0 code=-5 ms=2281\r\n"), "error line, negative int");

  LOG_D(Imu, "values") {
    line.field("ax", 0.012f, 3);
    line.field("gy", -0.0003f, 4);
    line.field("mz", 40.12f, 2);
  }
  expect(at(4, "D IMU 131263 c42: values ax=0.012 gy=-0.0003 mz=40.12\r\n"), "debug line, floats");

  // the same checks again at every level so the formatter is always tested
  if constexpr (CHIPSAT_LOG_LEVEL >= 1) {
    LOG_E(Cyc, "fmt") {
      line.field("a", 1013.25f, 2);
      line.field("b", static_cast<float>(NAN), 2);
      line.field("c", static_cast<float>(INFINITY), 2);
      line.field("d", -static_cast<float>(INFINITY), 2);
      line.field("e", 3.0e9f, 0);
      line.field("f", 2.5f, 0);
      line.field("g", -2.5f, 0);
      line.field("h", 0.125f, 2);
      line.field("i", -0.0f, 2);
    }
    expect("E CYC 131263 c42: fmt a=1013.25 b=nan c=inf d=inf e=ovf f=3 g=-3 h=0.13 i=0.00\r\n", "float edge cases");

    LOG_E(Cyc, "fmt") {
      line.fieldFixed("a", 424433500, 7);
      line.fieldFixed("b", -765018800, 7);
      line.fieldFixed("c", -5000000, 7);
      line.fieldFixed("d", 0, 7);
      line.fieldFixed("e", -17000, 3);
      line.fieldFixed("f", INT32_MIN, 7);
      line.fieldFixed("g", INT32_MAX, 3);
      line.fieldFixed("h", 7, 0);
    }
    expect("E CYC 131263 c42: fmt a=42.4433500 b=-76.5018800 c=-0.5000000 d=0.0000000 e=-17.000 "
           "f=-214.7483648 g=2147483.647 h=7\r\n", "fixed point edge cases");

    const uint32_t list[4] = {4, 4, 12, 4};
    LOG_E(Cyc, "fmt") {
      line.fieldHex("csr", 0x0C000000UL, 8);
      line.fieldHex("valid", 0x6F, 2);
      line.fieldBits("bits", 0x0B, 4);
      line.fieldList("age", list, 4);
      line.field("word", "never");
      line.field("big", 4294967295UL);
      line.field("neg", static_cast<long>(INT32_MIN));
    }
    expect("E CYC 131263 c42: fmt csr=0x0C000000 valid=0x6F bits=1101 age=4,4,12,4 word=never "
           "big=4294967295 neg=-2147483648\r\n", "hex, bits, list, word, integer limits");
  }

  // if/else without braces around a log line: the else has to belong to the outer if
  const bool okay = true;
  bool elseRan = false;
  if (okay)
    LOG_E(Imu, "branch") { line.field("took", "if"); }
  else
    elseRan = true;
  ++checks;
  failures += elseRan ? 1 : 0;
  printf("%s unbraced else binds to the outer if\n", elseRan ? "FAIL" : "ok  ");
  expect(at(1, "E IMU 131263 c42: branch took=if\r\n"), "log line as the if body");

  const bool notOkay = false;
  bool elseRan2 = false;
  if (notOkay)
    LOG_E(Imu, "branch") { line.field("took", "if"); }
  else
    elseRan2 = true;
  ++checks;
  failures += elseRan2 ? 0 : 1;
  printf("%s unbraced else runs when the outer if is false\n", elseRan2 ? "ok  " : "FAIL");
  expect("", "no log line when the outer if is false");

  // a realistic stretch around the packet for the decoder
  fakeNow = 131264;
  ChipSatLog::setCycle(43);
  LOG_I(Env, "read") {
    line.field("ok", true);
    line.field("valid", true);
    line.field("ms", 5);
  }
  fakeNow = 131265;
  LOG_PACKET(kVector, sizeof(kVector));
  const size_t packetAt = captured.find("I PKT");
  const std::string packetOnly = packetAt == std::string::npos ? std::string() : captured.substr(packetAt);
  fakeNow = 131281;
  LOG_I(Cyc, "packet") {
    line.field("ctr", 42);
    line.fieldHex("valid", 0xFF, 2);
    line.field("fresh", true);
  }

  if constexpr (CHIPSAT_LOG_LEVEL >= 3) {
    ++checks;
    const bool ok = packetOnly.compare(0, strlen(kVectorLine), kVectorLine) == 0;
    failures += ok ? 0 : 1;
    printf("%s packet line text\n", ok ? "ok  " : "FAIL");
  }

  FILE *f = fopen(excerptPath, "w");
  fputs(captured.c_str(), f);
  fclose(f);
  captured.clear();

  printf("%s: %d checks, %d failed\n", failures ? "FAILED" : "PASSED", checks, failures);
  return failures ? 1 : 0;
}
