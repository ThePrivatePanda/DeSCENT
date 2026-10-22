// ImuSh2.cpp against a simulated BNO085 over SHTP: the HP wake after TX has to stay within
// kImuWakeBudgetMs (plus one try) even when the hub takes writes and never answers
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock test/test_imu_wake.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/imuwake && /tmp/imuwake
#include <cstdio>
#include <cstring>
#include <deque>
#include <vector>
#include "../../V2_6_X_Code/src/devices/own/ImuSh2.cpp"

using ChipSatPlatform::Status;
using namespace ChipSatConstants;
using ChipSatDevices::kReportIds;

static uint32_t fakeNow = 0;

struct Hub
{
  bool answering = true;   // false: writes acked, nothing ever comes back
  bool awake = true;
  uint8_t enabled = 0;
  uint32_t lastReportMs = 0;
  uint8_t seq[4] = {0, 0, 0, 0};
  std::deque<std::vector<uint8_t>> out;
  bool inPacket = false;
  size_t offset = 0;

  void send(uint8_t channel, const std::vector<uint8_t> &cargo)
  {
    std::vector<uint8_t> p(4 + cargo.size());
    p[0] = static_cast<uint8_t>(p.size());
    p[1] = static_cast<uint8_t>(p.size() >> 8);
    p[2] = channel;
    p[3] = seq[channel]++;
    memcpy(&p[4], cargo.data(), cargo.size());
    out.push_back(p);
  }

  void take(const uint8_t *data, size_t length)
  {
    if (!answering || length < 5) {
      return;
    }
    const uint8_t channel = data[2];
    const uint8_t *cargo = &data[4];
    if (channel == kShtpChannelExecutable) {
      if (cargo[0] == kImuCommandReset) {
        enabled = 0;
        awake = true;
        send(kShtpChannelExecutable, {kImuResetComplete});
      } else if (cargo[0] == kImuCommandOn) {
        awake = true;
      } else if (cargo[0] == kImuCommandSleep) {
        awake = false;
      }
    } else if (channel == kShtpChannelControl && cargo[0] == kSh2SetFeature) {
      std::vector<uint8_t> reply(kSh2GetFeatureResponseLength, 0);
      reply[0] = kSh2GetFeatureResponse;
      reply[1] = cargo[1];
      memcpy(&reply[5], &cargo[5], 4);
      send(kShtpChannelControl, reply);
      for (uint8_t i = 0; i < 4; ++i) {
        if (cargo[1] == kReportIds[i]) {
          enabled |= static_cast<uint8_t>(1U << i);
        }
      }
    }
  }

  void maybeReport()
  {
    if (!answering || !awake || enabled != 0x0F || !out.empty() || fakeNow - lastReportMs < 50) {
      return;
    }
    lastReportMs = fakeNow;
    std::vector<uint8_t> cargo;
    for (uint8_t i = 0; i < 4; ++i) {
      std::vector<uint8_t> r(i == 3 ? kSh2RotationVectorLength : kSh2ReportLength, 0);
      r[0] = kReportIds[i];
      r[4] = 0x10;
      cargo.insert(cargo.end(), r.begin(), r.end());
    }
    send(kShtpChannelReports, cargo);
  }

  // a 4 byte read is the header alone; the first chunk after it starts at the header again,
  // later chunks repeat the header ahead of the next bytes
  void read(uint8_t *buffer, size_t length)
  {
    memset(buffer, 0, length);
    maybeReport();
    if (out.empty()) {
      return;
    }
    const std::vector<uint8_t> &p = out.front();
    if (!inPacket) {
      memcpy(buffer, p.data(), length < p.size() ? length : p.size());
      if (length == 4) {
        inPacket = true;
        offset = 0;
        return;
      }
      out.pop_front();
      return;
    }
    if (offset == 0) {
      const size_t n = length < p.size() ? length : p.size();
      memcpy(buffer, p.data(), n);
      offset = n;
    } else {
      memcpy(buffer, p.data(), 4);
      const size_t n = length - 4 < p.size() - offset ? length - 4 : p.size() - offset;
      memcpy(buffer + 4, p.data() + offset, n);
      offset += n;
    }
    if (offset >= p.size()) {
      out.pop_front();
      inPacket = false;
    }
  }
};
static Hub hub;

namespace ChipSatPlatform
{
Uart::Uart(UartPort port) : port_(port) {}
Status Uart::begin(uint32_t) { return Status::Ok; }
size_t Uart::write(const uint8_t *, size_t length) { return length; }
size_t Uart::write(const char *text) { return strlen(text); }
size_t Uart::available() { return 0; }
int Uart::read() { return -1; }
void Uart::flush() {}
Stream &Uart::arduinoStream() { static Stream s; return s; }
uint32_t System::nowMs() const { return fakeNow; }
void System::waitMs(uint32_t ms) const { fakeNow += ms; }

// a transfer of a few bytes at 100 kHz, rounded up to 1 ms
Status I2cBus::probe(uint8_t) { ++fakeNow; return Status::Ok; }
Status I2cBus::write(uint8_t, const uint8_t *data, size_t length) { ++fakeNow; hub.take(data, length); return Status::Ok; }
Status I2cBus::read(uint8_t, uint8_t *buffer, size_t length) { ++fakeNow; hub.read(buffer, length); return Status::Ok; }
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

int main()
{
  static ChipSatPlatform::System chipSystem;
  static ChipSatPlatform::I2cBus bus;
  ChipSatDevices::Imu imu(bus, chipSystem);
  fakeNow = 1000;
  check(imu.begin(ChipSatConfig::kImuReportIntervalMs) == Status::Ok && imu.ready(), "begin against the simulated hub");
  for (int i = 0; i < 20; ++i) {
    imu.service();
    fakeNow += 10;
  }
  check(imu.data().valid, "reports flowing");

  check(imu.sleep() == Status::Ok && imu.sleeping(), "sleep for a TX");
  fakeNow += 150;
  uint32_t start = fakeNow;
  const Status woke = imu.wake();
  const uint32_t healthyMs = fakeNow - start;
  printf("     healthy wake took %u ms\n", healthyMs);
  check(woke == Status::Ok && healthyMs < ChipSatConfig::kImuWakeBudgetMs, "a hub that answers wakes Ok inside the budget");

  imu.sleep();
  hub.answering = false;   // takes writes, says nothing
  fakeNow += 150;
  start = fakeNow;
  const Status mute = imu.wake();
  const uint32_t muteMs = fakeNow - start;
  printf("     mute hub wake took %u ms\n", muteMs);
  check(mute != Status::Ok, "a mute hub's wake fails");
  check(muteMs <= ChipSatConfig::kImuWakeBudgetMs + ChipSatConfig::kImuReplyWaitMs + ChipSatConfig::kImuEnableRetryDelayMs + 20,
        "and returns within the budget plus one try, not ~9.7 s");
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
