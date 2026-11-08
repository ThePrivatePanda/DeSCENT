// GpsMaxM10s.cpp against a fake receiver on the UART: hAcc out of NAV-PVT, software standby that
// nothing is written to after, and the wake
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock test/test_gps.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/gps && /tmp/gps
#include <cstdio>
#include <cstring>
#include <deque>
#include <vector>
#include "../../V2_6_X_Code/src/devices/own/GpsMaxM10s.cpp"

using ChipSatPlatform::Status;
using namespace ChipSatConstants;

static uint32_t fakeNow = 0;

// Acks every CFG-VALSET while awake. PMREQ puts it in standby, where the first byte it gets wakes it
// and is lost
struct Receiver
{
  bool asleep = false;
  std::vector<uint8_t> written;   // everything the driver sent, in order
  std::vector<uint8_t> frame;     // the frame being taken in
  std::deque<uint8_t> out;
  int valsets = 0;
  int polls = 0;
  int pmreqs = 0;

  void send(uint8_t cls, uint8_t id, const std::vector<uint8_t> &payload)
  {
    std::vector<uint8_t> f = {kUbxSync1, kUbxSync2, cls, id, static_cast<uint8_t>(payload.size()),
                              static_cast<uint8_t>(payload.size() >> 8)};
    f.insert(f.end(), payload.begin(), payload.end());
    uint8_t a = 0, b = 0;
    for (size_t i = 2; i < f.size(); ++i) {
      a = static_cast<uint8_t>(a + f[i]);
      b = static_cast<uint8_t>(b + a);
    }
    f.push_back(a);
    f.push_back(b);
    out.insert(out.end(), f.begin(), f.end());
  }

  void take(uint8_t byte)
  {
    written.push_back(byte);
    if (asleep) {
      asleep = false;
      return;
    }
    if (frame.empty() && byte != kUbxSync1) {
      return;
    }
    frame.push_back(byte);
    if (frame.size() >= 6 && frame.size() == 8u + (frame[4] | frame[5] << 8)) {
      if (frame[2] == kUbxClassCfg && frame[3] == kUbxIdValset) {
        ++valsets;
        send(kUbxClassAck, kUbxIdAck, {kUbxClassCfg, kUbxIdValset});
      } else if (frame[2] == kUbxClassNav && frame[3] == kUbxIdNavPvt) {
        ++polls;
      } else if (frame[2] == kUbxClassRxm && frame[3] == kUbxIdPmreq) {
        ++pmreqs;
        asleep = true;
      }
      frame.clear();
    }
  }

  // a trusted 3D fix with this hAcc, and fully resolved time
  void pvt(uint32_t hAccMm)
  {
    std::vector<uint8_t> p(kNavPvtLength, 0);
    p[kNavPvtYear] = 0xEA;   // 2026
    p[kNavPvtYear + 1] = 0x07;
    p[kNavPvtMonth] = 11;
    p[kNavPvtDay] = 7;
    p[kNavPvtValid] = kNavPvtValidDate | kNavPvtValidTime | kNavPvtFullyResolved;
    p[kNavPvtFixType] = kNavPvtFix3d;
    p[kNavPvtFlags] = kNavPvtGnssFixOk;
    p[kNavPvtNumSatellites] = 9;
    const int32_t lat = 371234567, lon = -1221234567, alt = 120000;
    memcpy(&p[kNavPvtLat], &lat, 4);
    memcpy(&p[kNavPvtLon], &lon, 4);
    memcpy(&p[kNavPvtHeightMsl], &alt, 4);
    memcpy(&p[kNavPvtHAcc], &hAccMm, 4);
    send(kUbxClassNav, kUbxIdNavPvt, p);
  }
};
static Receiver rx;

namespace ChipSatPlatform
{
Uart::Uart(UartPort port) : port_(port) {}
Status Uart::begin(uint32_t) { return Status::Ok; }
void Uart::end() {}
size_t Uart::write(const uint8_t *data, size_t length)
{
  for (size_t i = 0; i < length; ++i) {
    rx.take(data[i]);
  }
  return length;
}
size_t Uart::write(const char *text) { return strlen(text); }
size_t Uart::available() { return rx.out.size(); }
int Uart::read()
{
  ++fakeNow;   // keeps the driver's ack wait moving
  if (rx.out.empty()) {
    return -1;
  }
  const int b = rx.out.front();
  rx.out.pop_front();
  return b;
}
void Uart::flush() {}
Stream &Uart::arduinoStream() { static Stream s; return s; }
uint32_t System::nowMs() const { return fakeNow; }
void System::waitMs(uint32_t ms) const { fakeNow += ms; }
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

// read() every 500 ms, the way sampleSensors does, a PVT a second if the receiver is up
static void runFor(ChipSatDevices::Gps &gps, uint32_t forMs, bool pvts)
{
  for (const uint32_t until = fakeNow + forMs; fakeNow < until; fakeNow += 500) {
    if (pvts && !rx.asleep && fakeNow % 1000 < 500) {
      rx.pvt(4000);
    }
    gps.read();
  }
}

int main()
{
  static ChipSatPlatform::System chipSystem;
  static ChipSatPlatform::Uart port(ChipSatPlatform::UartPort::Gps);
  ChipSatDevices::Gps gps(port, chipSystem);
  fakeNow = 1000;
  check(gps.begin() == Status::Ok, "begin against the fake receiver");

  rx.pvt(7500);
  check(gps.read() == Status::Ok && gps.data().valid && gps.data().horizontalAccuracyMm == 7500,
        "hAcc from NAV-PVT offset 40, mm");
  runFor(gps, 5000, true);
  check(gps.utcValid() && gps.data().horizontalAccuracyMm == 4000, "fixes and time coming in");

  const size_t before = rx.written.size();
  gps.standby();
  const std::vector<uint8_t> sent(rx.written.begin() + static_cast<long>(before), rx.written.end());
  const uint8_t want[24] = {0xB5, 0x62, 0x02, 0x41, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
                            0x06, 0, 0, 0, 0x08, 0, 0, 0, 0x61, 0x6B};
  check(sent.size() == sizeof want && memcmp(sent.data(), want, sizeof want) == 0,
        "standby: one UBX-RXM-PMREQ, duration 0, backup and force, woken by UART RX");
  check(rx.pmreqs == 1 && rx.asleep, "the receiver is in standby");
  check(!gps.data().valid && !gps.utcValid(), "standby: position and time invalid");

  const size_t asleepAt = rx.written.size();
  runFor(gps, 60000, false);
  check(rx.written.size() == asleepAt && rx.asleep, "a minute in standby with nothing coming in: nothing written, no poll, no reconfig");
  check(gps.read() == Status::NotReady, "read() in standby says not ready");

  const int valsetsBefore = rx.valsets;
  const Status woke = gps.wake();
  check(rx.written[asleepAt] == 0xFF, "wake: a byte first, which the receiver loses");
  check(woke == Status::Ok && !rx.asleep, "wake: Ok once the receiver acked a setting");
  check(rx.valsets - valsetsBefore == 7, "then all six settings sent again");

  const int pollsBefore = rx.polls;
  runFor(gps, 6000, false);
  check(rx.polls > pollsBefore, "awake with no positions: polled again as before");
  runFor(gps, 3000, true);
  check(gps.data().valid, "and positions again");
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
