// ImuSh2.cpp against a simulated BNO085 over SHTP: the HP wake after TX has to stay within
// kImuWakeBudgetMs (plus one try) even when the hub takes writes and never answers, the
// report interval moves between 20 Hz and 1 Hz without a reset, and a hub the I2C reset can't
// reach gets the NRST pin
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock test/test_imu_wake.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/imuwake && /tmp/imuwake
#include <cstdio>
#include <algorithm>
#include <cstring>
#include <deque>
#include <string>
#include <vector>
#include "../../V2_6_X_Code/src/devices/own/ImuSh2.cpp"

using ChipSatPlatform::Status;
using namespace ChipSatConstants;
using ChipSatDevices::kReportIds;

static uint32_t fakeNow = 0;

struct Hub
{
  bool answering = true;   // false: writes acked, nothing ever comes back
  bool wedged = false;     // NACKs everything until a pin reset
  bool dead = false;       // and a pin reset doesn't help either
  bool pinDriven = false;  // PB4 an output
  bool pinLow = false;
  uint32_t lowFromMs = 0;
  uint32_t shortestHoldMs = 0xFFFFFFFF;
  std::vector<uint32_t> pinResetsAt;   // when NRST was let go
  uint32_t bootedAtMs = 0;   // after a pin reset it talks again from here
  bool bootPending = false;
  bool awake = true;
  uint8_t enabled = 0;
  uint32_t intervalUs[4] = {0, 0, 0, 0};
  int resets = 0;
  uint32_t lastReportMs = 0;
  uint8_t seq[4] = {0, 0, 0, 0};
  std::deque<std::vector<uint8_t>> out;
  bool inPacket = false;
  size_t offset = 0;

  bool acks() const { return !dead && !wedged && !(pinDriven && pinLow) && fakeNow >= bootedAtMs; }

  // 90 + 4 ms from NRST high to H_INTN, BNO08X datasheet 6.5.3
  void setPin(bool driven, bool low)
  {
    const bool wasLow = pinDriven && pinLow;
    pinDriven = driven;
    pinLow = low;
    if (!wasLow && driven && low) {
      lowFromMs = fakeNow;
    }
    if (wasLow && !(driven && low)) {
      shortestHoldMs = std::min(shortestHoldMs, fakeNow - lowFromMs);
      pinResetsAt.push_back(fakeNow);
      if (!dead) {
        wedged = false;
        answering = true;
        enabled = 0;
        awake = true;
        out.clear();
        inPacket = false;
        memset(seq, 0, sizeof seq);
        bootedAtMs = fakeNow + 94;
        bootPending = true;
      }
    }
  }

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
        ++resets;
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
          intervalUs[i] = static_cast<uint32_t>(cargo[5] | cargo[6] << 8 | cargo[7] << 16) | static_cast<uint32_t>(cargo[8]) << 24;
        }
      }
    }
  }

  void maybeReport()
  {
    if (!answering || !awake || enabled != 0x0F || !out.empty() || fakeNow - lastReportMs < intervalUs[0] / 1000) {
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
    if (bootPending) {
      bootPending = false;
      send(kShtpChannelExecutable, {kImuResetComplete});
    }
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
Status I2cBus::probe(uint8_t) { ++fakeNow; return hub.acks() ? Status::Ok : Status::NoAck; }
Status I2cBus::write(uint8_t, const uint8_t *data, size_t length)
{
  ++fakeNow;
  if (!hub.acks()) {
    return Status::NoAck;
  }
  hub.take(data, length);
  return Status::Ok;
}
Status I2cBus::read(uint8_t, uint8_t *buffer, size_t length)
{
  ++fakeNow;
  if (!hub.acks()) {
    return Status::NoAck;
  }
  hub.read(buffer, length);
  return Status::Ok;
}
}

void pinMode(uint32_t pin, uint32_t mode)
{
  if (pin == kImuResetPin) {
    hub.setPin(mode == OUTPUT, hub.pinLow);
  }
}

void digitalWrite(uint32_t pin, uint32_t value)
{
  if (pin == kImuResetPin) {
    hub.setPin(hub.pinDriven, value == LOW);
  }
}

static int checks = 0, failures = 0;
static std::vector<ChipSatLog::SystemEvent> events;

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
  // on the ground: 1 Hz through Set Feature alone, then back to 20 Hz
  hub.answering = true;
  imu.sleep();
  imu.wake();
  const int resetsBefore = hub.resets;
  check(imu.setReportInterval(ChipSatConfig::kImuGroundIntervalMs) == Status::Ok && imu.reportIntervalMs() == 1000,
        "1 Hz: the four reports turned on again");
  check(hub.intervalUs[0] == 1000000 && hub.intervalUs[1] == 1000000 && hub.intervalUs[2] == 1000000 &&
        hub.intervalUs[3] == 1000000 && hub.resets == resetsBefore, "1 Hz: Set Feature at 1 000 000 us on all four, no reset");
  uint32_t updates = 0, lastMs = imu.data().linearAccelerationUpdatedMs;
  for (const uint32_t until = fakeNow + 10000; fakeNow < until; fakeNow += 10) {
    imu.service();
    if (imu.data().linearAccelerationUpdatedMs != lastMs) {
      lastMs = imu.data().linearAccelerationUpdatedMs;
      ++updates;
    }
  }
  printf("     %u reports in 10 s at 1 Hz\n", static_cast<unsigned int>(updates));
  check(updates >= 9 && updates <= 11, "1 Hz: about ten reports in 10 s");
  check(imu.setReportInterval(ChipSatConfig::kImuReportIntervalMs) == Status::Ok && hub.intervalUs[3] == 50000,
        "back to 20 Hz the same way");
  imu.sleep();
  check(imu.setReportInterval(ChipSatConfig::kImuGroundIntervalMs) == Status::NotReady && imu.reportIntervalMs() == 1000,
        "asleep: not sent, kept for the wake");
  imu.wake();
  check(hub.intervalUs[0] == 1000000, "and the wake turns them on at it");

  using namespace ChipSatLog;
  setSystemSink([](void *, const SystemEvent &e) { events.push_back(e); }, nullptr);
  const auto steps = [] {
    std::string out;
    for (const SystemEvent &e : events) {
      out += std::to_string(e.what) + "/" + std::to_string(e.reason) + "/" + std::to_string(e.detail) + " ";
    }
    printf("     %s\n", out.c_str());
    return out;
  };
  const auto serviceFor = [&imu](uint32_t ms) {
    for (const uint32_t until = fakeNow + ms; fakeNow < until; fakeNow += 10) {
      imu.service();
    }
  };

  // the boot: NRST before begin(), which then waits for that reset instead of sending its own
  int resetsBefore2 = hub.resets;
  imu.pinReset(kWhyBoot);
  check(imu.begin(ChipSatConfig::kImuReportIntervalMs) == Status::Ok && imu.ready() && hub.resets == resetsBefore2 &&
        hub.pinResetsAt.size() == 1, "boot: the pin reset, then begin waits for it, no I2C reset after it");
  check(!hub.pinDriven && hub.shortestHoldMs >= 1, "boot: PB4 low at least 1 ms, then an input again");
  check(steps() == "7/8/0 ", "boot: one pin reset event, reason boot");
  serviceFor(200);
  check(imu.data().valid, "boot: reports flowing");

  // NACKs everything, as boards 15 and 18 did after their bus was cleared
  events.clear();
  hub.wedged = true;
  resetsBefore2 = hub.resets;
  imu.fullRestart();
  serviceFor(40000);
  check(imu.ready() && hub.pinResetsAt.size() == 2 && hub.resets == resetsBefore2 && !hub.pinDriven,
        "a hub that NACKs: the probe fails, the next try is the pin, and it's back");
  check(steps() == "8/10/0 7/10/1 8/0/1 ", "and its events: failed on noack, pin reset after one failure, restart ok");
  check(hub.pinResetsAt[1] - events[0].ms >= ChipSatConfig::sensorRetryMs(events[0].ms, ChipSatConfig::kImuRetryMs),
        "the pin waits the usual retry spacing");
  serviceFor(200);
  check(imu.data().valid, "reports flowing after the pin reset");

  // takes the I2C reset, never says reset complete
  events.clear();
  hub.answering = false;
  imu.fullRestart();
  serviceFor(40000);
  check(imu.ready() && hub.pinResetsAt.size() == 3 && !hub.pinDriven, "a hub that never says reset complete: the pin on the next try");
  check(steps() == "8/11/0 7/11/1 8/0/1 ", "and its events: failed on noreset, pin reset, restart ok");

  // the pin doesn't help: one pin reset every try and no faster, each failure on the chip
  events.clear();
  hub.dead = true;
  const size_t pinsBefore = hub.pinResetsAt.size();
  imu.fullRestart();
  serviceFor(120000);
  bool spaced = true;
  for (size_t i = pinsBefore + 1; i < hub.pinResetsAt.size(); ++i) {
    spaced = spaced && hub.pinResetsAt[i] - hub.pinResetsAt[i - 1] >= ChipSatConfig::kImuRetryMs;
  }
  const size_t pins = hub.pinResetsAt.size() - pinsBefore;
  printf("     %u pin resets in 120 s\n", static_cast<unsigned int>(pins));
  check(!imu.ready() && pins >= 8 && pins <= 10 && spaced && !hub.pinDriven, "a dead hub: a pin reset every retry, no faster, PB4 let go each time");
  check(std::count_if(events.begin(), events.end(), [](const SystemEvent &e) {
          return e.what == kSensorDriverRestart && e.reason == kWhyNoReset && e.detail == 0;
        }) == static_cast<long>(pins), "each pin reset that got no answer is a failure event");
  setSystemSink(nullptr, nullptr);
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
