// I2cWatch.cpp on the laptop: the bus clear against a simulated stuck part, and the watchdog's rules
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src test/test_i2c_watch.cpp ../V2_6_X_Code/src/platform/I2cWatch.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/i2cwatch && /tmp/i2cwatch
#include <cstdio>
#include <cstring>
#include <string>
#include "platform/I2cWatch.h"
#include "log/Log.h"
#include "constants.h"

using namespace ChipSatPlatform;
using namespace ChipSatConfig;

static std::string captured;
static uint32_t fakeNow = 0;

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

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
  }
  printf("%s %s\n", ok ? "ok  " : "FAIL", what);
}

static void expectLog(const std::string &want, const char *what)
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

// A part on the bus. holdSdaClocks: it holds SDA until that many SCL falling edges (-1 forever).
// holdScl: it holds SCL for good. stretch: half clocks it holds SCL after each release
struct Sim : BusLines
{
  bool ourSda = false, ourScl = false;   // true = we drive it low
  int holdSdaClocks = 0;
  bool holdScl = false;
  int stretch = 0;
  int stretchLeft = 0;
  int falling = 0;
  int halfClocks = 0;
  bool stopSeen = false;

  bool partSda() const { return holdSdaClocks < 0 || falling < holdSdaClocks; }
  bool sda() override { return !ourSda && !partSda(); }
  bool scl() override { return !ourScl && !holdScl && stretchLeft == 0; }
  void sdaLow(bool low) override
  {
    if (!low && ourSda && scl()) {
      stopSeen = true;   // SDA up while SCL is high
    }
    ourSda = low;
  }
  void sclLow(bool low) override
  {
    if (low && !ourScl) {
      ++falling;
    }
    if (!low && ourScl) {
      stretchLeft = stretch;
    }
    ourScl = low;
  }
  void halfClock() override
  {
    ++halfClocks;
    if (stretchLeft > 0) {
      --stretchLeft;
    }
  }
};

static void testClear()
{
  {
    Sim sim;
    const BusClear r = clearBus(sim);
    check(r.ok && r.pulses == 0 && r.sdaBefore && r.sclBefore && sim.stopSeen, "clear: idle bus, no clocks, a STOP");
  }
  {
    Sim sim;
    sim.holdSdaClocks = 3;
    const BusClear r = clearBus(sim);
    check(r.ok && r.pulses == 3 && !r.sdaBefore && r.sclBefore && sim.stopSeen, "clear: SDA held for 3 clocks, 3 pulses then a STOP");
  }
  {
    Sim sim;
    sim.holdSdaClocks = -1;
    const BusClear r = clearBus(sim);
    check(!r.ok && r.pulses == 9 && !r.sclHeld, "clear: SDA held for good, gives up after 9 clocks");
    check(sim.halfClocks * ChipSatConstants::kI2cHalfClockUs < 12000, "clear: under 12 ms");
  }
  {
    Sim sim;
    sim.holdScl = true;
    const BusClear r = clearBus(sim);
    check(!r.ok && r.sclHeld && r.pulses == 0 && !r.sclBefore, "clear: SCL held, no clocking against it");
    check(sim.falling == 0 && sim.halfClocks <= ChipSatConstants::kI2cStretchWaits + 2, "clear: SCL held, never driven, about 1 ms of waiting");
  }
  {
    Sim sim;
    sim.holdSdaClocks = 2;
    sim.stretch = 40;
    const BusClear r = clearBus(sim);
    check(r.ok && r.pulses == 2, "clear: a part stretching the clock 200 us is waited for");
  }
}

static void testHeld()
{
  BusWatch w;
  fakeNow = 1000;
  check(w.before(0x77, true, true, true, fakeNow) == BusGate::Go && !w.faulted(fakeNow), "gate: idle lines go");
  check(w.before(0x77, false, true, true, fakeNow) == BusGate::Held, "gate: SDA low is held, nothing sent");
  expectLog("W BUS 1000 c0: held sda=0 scl=1 part=env addr=0x77 when=before\r\n", "line: held");
  check(w.before(0x4A, false, true, true, fakeNow) == BusGate::Held, "gate: still held for the next part");
  expectLog("", "line: held once, not per transfer");
  check(w.faulted(fakeNow) && w.clearDue(fakeNow), "held: faulted, a clear is due");

  BusClear done;
  done.sdaBefore = false;
  done.pulses = 3;
  done.ok = true;
  w.cleared(done, fakeNow);
  expectLog("W BUS 1000 c0: clear sda=0 scl=1 pulses=3 ok=1 n=1\r\n", "line: clear that worked");
  check(!w.clearDue(fakeNow) && w.faulted(fakeNow + 999) && !w.faulted(fakeNow + 1000), "cleared: sensor steps wait kBusSettleMs");

  // a clear that fails is retried with a doubling wait, Wire stays down
  w.before(0x77, false, true, true, fakeNow);
  captured.clear();
  BusClear failed;
  failed.sdaBefore = false;
  failed.pulses = 9;
  w.cleared(failed, fakeNow);
  expectLog("W BUS 1000 c0: clear sda=0 scl=1 pulses=9 ok=0 sclheld=0 retryms=1000 n=2\r\n", "line: clear that failed");
  check(!w.clearDue(fakeNow + 999) && w.clearDue(fakeNow + 1000), "failed clear: next after 1 s");
  w.cleared(failed, fakeNow + 1000);
  captured.clear();
  check(!w.clearDue(fakeNow + 2999) && w.clearDue(fakeNow + 3000), "failed twice: next after 2 s");
  check(w.before(0x77, true, true, false, fakeNow + 3000) == BusGate::Held, "gate: lines free but Wire down, held until a clear works");
}

static void testQuarantine()
{
  BusWatch w;
  fakeNow = 5000;
  BusClear ok;
  ok.ok = true;
  // env grabs the bus right after its transfer
  w.before(0x77, true, true, true, fakeNow);
  w.after(0x77, Status::Timeout, 100, false, true, fakeNow);
  expectLog("W BUS 5000 c0: held sda=0 scl=1 part=env addr=0x77 when=after\r\n", "line: jam blamed on the part addressed");
  w.cleared(ok, fakeNow);
  captured.clear();
  check(!w.quarantined(0x77, fakeNow), "one jam is not a quarantine");
  fakeNow += 2000;
  w.before(0x77, true, true, true, fakeNow);
  w.after(0x77, Status::Timeout, 100, false, true, fakeNow);
  expectLog("W BUS 7000 c0: held sda=0 scl=1 part=env addr=0x77 when=after\r\n"
            "W BUS 7000 c0: quarantine part=env addr=0x77 jams=2 forms=30000\r\n", "line: two jams in a minute, quarantine");
  w.cleared(ok, fakeNow);
  captured.clear();
  check(w.before(0x77, true, true, true, fakeNow + 29999) == BusGate::Refused, "quarantine: nothing sent to it for 30 s");
  check(w.before(0x4A, true, true, true, fakeNow + 29999) == BusGate::Go, "quarantine: the other parts go on");
  fakeNow += 30000;
  check(w.before(0x77, true, true, true, fakeNow) == BusGate::Go, "quarantine: over after 30 s");
  expectLog("I BUS 37000 c0: release part=env addr=0x77\r\n", "line: release");
  w.after(0x77, Status::Timeout, 100, false, true, fakeNow);
  check(w.quarantined(0x77, fakeNow), "probation: one more jam and it is back in");
  check(captured.find("forms=60000") != std::string::npos, "probation: for twice as long");
  captured.clear();
}

static void testHold()
{
  BusWatch w;
  fakeNow = 100000;
  w.after(0x4A, Status::Timeout, 101, true, true, fakeNow);
  check(w.before(0x4A, true, true, true, fakeNow) == BusGate::Go && captured.empty(), "hold: one timeout is let through");
  fakeNow += 200;
  w.after(0x4A, Status::BusError, 100, true, true, fakeNow);
  fakeNow -= 200;
  expectLog("W BUS 100200 c0: hold part=imu addr=0x4A why=timeout ms=100 forms=1000\r\n", "line: two slow failures in a row, hold");
  check(w.before(0x4A, true, true, true, fakeNow + 1199) == BusGate::Refused, "hold: no 100 ms spins for 1 s");
  check(w.before(0x4A, true, true, true, fakeNow + 1200) == BusGate::Go, "hold: over after 1 s");
  w.after(0x4A, Status::Timeout, 100, true, true, fakeNow + 1200);
  w.after(0x4A, Status::Timeout, 100, true, true, fakeNow + 1300);
  check(captured.find("forms=2000") != std::string::npos, "hold: doubles while it goes on");
  captured.clear();
  w.after(0x4A, Status::Ok, 1, true, true, fakeNow + 5000);
  w.after(0x4A, Status::Timeout, 100, true, true, fakeNow + 5100);
  w.after(0x4A, Status::Timeout, 100, true, true, fakeNow + 5200);
  check(captured.find("forms=1000") != std::string::npos, "hold: a good transfer starts it over");
  captured.clear();

  BusWatch fast;
  for (int i = 0; i < 10; ++i) {
    fast.after(0x77, Status::NoAck, 0, true, true, fakeNow + i);
    fast.after(0x77, Status::BusError, 1, true, true, fakeNow + i);
  }
  check(captured.empty() && fast.before(0x77, true, true, true, fakeNow + 10) == BusGate::Go && !fast.faulted(fakeNow + 10),
        "fast failures (an absent part) are no fault and no hold");
}

static void testTwoParts()
{
  BusWatch w;
  fakeNow = 200000;
  w.after(0x77, Status::Timeout, 100, true, true, fakeNow);
  check(!w.clearDue(fakeNow), "one part timing out is that part");
  fakeNow += 500;
  w.after(0x36, Status::Timeout, 100, true, true, fakeNow);
  fakeNow -= 500;
  expectLog("W BUS 200500 c0: fault why=timeouts part=soc ms=100\r\n", "line: two parts timing out within 1 s is the bus");
  check(w.clearDue(fakeNow + 500) && w.faulted(fakeNow + 500), "two parts: clear due");

  BusWatch apart;
  apart.after(0x77, Status::Timeout, 100, true, true, fakeNow);
  apart.after(0x36, Status::Timeout, 100, true, true, fakeNow + 1001);
  check(!apart.clearDue(fakeNow + 1001), "two parts more than 1 s apart: no bus fault");
  captured.clear();
}

int main()
{
  static Uart console(UartPort::Console);
  static System chipSystem;
  ChipSatLog::begin(console, chipSystem);
  testClear();
  testHeld();
  testQuarantine();
  testHold();
  testTwoParts();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
