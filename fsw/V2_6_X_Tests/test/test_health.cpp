// Health.cpp and SensorWatch.cpp on the laptop, with fake drivers
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src test/test_health.cpp ../V2_6_X_Code/src/app/{Health,SensorWatch}.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/health && /tmp/health
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include "app/Health.h"
#include "log/Log.h"

using namespace ChipSatPlatform;
using namespace ChipSatConfig;
using ChipSatApp::Health;
using ChipSatApp::Rung;

static std::string captured;
static uint32_t fakeNow = 0;

// what the fake parts do
struct World
{
  bool imuReady = true, imuAsleep = false;
  uint8_t imuFlowing = 0x0F;   // reports still arriving
  bool envAlive = true, envReady = true;
  bool gaugeAlive = true;
  bool gpsTalking = true;
  bool busFaulted = false, busClearDue = false;
  bool envRestartWorks = false;
  std::vector<std::string> actions;   // with the time
};
static World world;

static void did(const char *what)
{
  world.actions.push_back(std::string(what) + "@" + std::to_string(fakeNow));
}

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
bool I2cBus::recover(uint32_t)
{
  if (!world.busClearDue) {
    return false;
  }
  did("busclear");
  world.busClearDue = false;
  return true;
}
bool I2cBus::faulted(uint32_t) const { return world.busFaulted; }
}

// The drivers' per-pass calls, faked: they copy the world into the driver as the real ones would
namespace ChipSatDevices
{
Imu::Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
void Imu::service()
{
  ready_ = world.imuReady;
  sleeping_ = world.imuAsleep;
  for (uint8_t i = 0; i < 4; ++i) {
    if (ready_ && !sleeping_ && (world.imuFlowing & (1U << i))) {
      ++events_[i];
    }
  }
}
Status Imu::resetHub() { did("imu.softreset"); world.imuReady = false; return Status::Ok; }
Status Imu::fullRestart() { did("imu.restart"); world.imuReady = false; return Status::Ok; }

EnvSensor::EnvSensor(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status EnvSensor::read()
{
  ready_ = world.envReady;
  if (world.envAlive) {
    data_.valid = true;
    data_.updatedMs = fakeNow;
    return Status::Ok;
  }
  data_.valid = false;
  return Status::NoAck;
}
bool EnvSensor::restart()
{
  did("env.restart");
  ready_ = world.envReady = world.envRestartWorks;
  world.envAlive = world.envRestartWorks;
  return ready_;
}

FuelGauge::FuelGauge(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status FuelGauge::read()
{
  ready_ = true;
  if (world.gaugeAlive) {
    data_.valid = true;
    data_.updatedMs = fakeNow;
  } else {
    data_.valid = false;
  }
  return Status::Ok;
}
bool FuelGauge::restart() { did("soc.restart"); return false; }

Gps::Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system) : port_(port), system_(system) {}
void Gps::service()
{
  if (world.gpsTalking) {
    pvtSeen_ = byteSeen_ = true;
    lastPvtMs_ = lastByteMs_ = fakeNow;
  }
}
void Gps::restartPort() { did("gps.port"); }
void Gps::hotStart() { did("gps.hotstart"); }
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

static bool logged(const char *text)
{
  return captured.find(text) != std::string::npos;
}

static uint32_t countOf(const char *prefix)
{
  uint32_t n = 0;
  for (const auto &a : world.actions) {
    n += a.rfind(prefix, 0) == 0 ? 1 : 0;
  }
  return n;
}

static ChipSatPlatform::System chipSystem;
static ChipSatPlatform::I2cBus bus;
static ChipSatPlatform::Uart gpsPort(ChipSatPlatform::UartPort::Gps);
static ChipSatDevices::Imu imu(bus, chipSystem);
static ChipSatDevices::Gps gps(gpsPort, chipSystem);
static ChipSatDevices::EnvSensor env(bus, chipSystem);
static ChipSatDevices::FuelGauge gauge(bus, chipSystem);

// the loop: a pass every 10 ms, the env read every 200 ms, the gauge every second
static void run(Health &h, uint32_t forMs, bool air)
{
  const uint32_t until = fakeNow + forMs;
  while (fakeNow < until) {
    imu.service();
    gps.service();
    if (fakeNow % 200 == 0) {
      env.read();
    }
    if (fakeNow % 1000 == 0) {
      gauge.read();
    }
    h.service(air);
    fakeNow += 10;
  }
}

static Health *fresh()
{
  world = World();
  captured.clear();
  fakeNow = 100000;
  static Health *h = nullptr;
  delete h;
  h = new Health(chipSystem, bus, imu, gps, env, gauge);
  run(*h, 1000, false);   // everything up before begin
  h->begin();
  return h;
}

static void testAllGood()
{
  Health &h = *fresh();
  run(h, 600000, false);
  check(world.actions.empty() && captured.empty(), "all good ten minutes: no step, no line");
}

static void testEnvLadder()
{
  Health &h = *fresh();
  run(h, 5000, false);
  const uint32_t died = fakeNow;
  world.envAlive = false;
  run(h, 2300, false);
  check(logged("W ENV") && logged("recover step=retry reason=stale") && world.actions.empty(), "baro dead 2 s: retry, nothing done to it yet");
  run(h, 2000, false);
  check(countOf("env.restart") == 1 && logged("recover step=reset how=start attempt=1 reason=stale ok=0"), "baro dead 4 s: reset once");
  run(h, 1100, false);
  check(logged("recover step=aside reason=notready backoffms=1000 attempts=1"), "reset did not help: set aside 1 s");
  run(h, 1100, false);
  check(countOf("env.restart") == 2, "after the aside: reset again");
  run(h, 120000, false);
  check(logged("backoffms=30000") && !logged("backoffms=60000"), "on the ground the aside doubles to 30 s and no further");
  const uint32_t resets = countOf("env.restart");
  check(resets >= 8 && resets <= 12, "two minutes dead: a reset every backoff, not every pass");
  captured.clear();
  world.envRestartWorks = true;
  run(h, 40000, false);
  check(logged("I ENV") && logged("recover step=ok") && h.envWatch().rung() == Rung::Ok, "baro back after a reset: ok");
  (void)died;
  captured.clear();
  world.actions.clear();
  run(h, 60000, false);
  check(world.actions.empty() && captured.empty(), "and quiet after");
}

static void testAirCeiling()
{
  Health &h = *fresh();
  world.envAlive = false;
  run(h, 60000, true);
  check(logged("backoffms=4000") && logged("backoffms=5000") && !logged("backoffms=8000"), "in the air the aside stops at 5 s");
}

static void testOneStepAPass()
{
  Health &h = *fresh();
  world.envAlive = false;
  world.gaugeAlive = false;
  world.gpsTalking = false;
  run(h, 30000, false);
  bool spaced = true;
  for (size_t i = 1; i < world.actions.size(); ++i) {
    const uint32_t a = std::stoul(world.actions[i - 1].substr(world.actions[i - 1].find('@') + 1));
    const uint32_t b = std::stoul(world.actions[i].substr(world.actions[i].find('@') + 1));
    spaced = spaced && b - a >= kHealthGapMs;
  }
  check(world.actions.size() >= 4 && spaced, "three dead parts: one step at a time, 200 ms apart at least");
  check(countOf("env.restart") >= 1 && countOf("soc.restart") >= 1 && countOf("gps.port") >= 1,
        "each dead part gets its own steps, none blocks another");
}

static void testGroundOrder()
{
  Health &h = *fresh();
  world.busFaulted = true;   // everything comes due while the bus is down
  world.envAlive = false;
  world.gpsTalking = false;
  world.imuFlowing = 0;
  run(h, 15000, false);
  world.busFaulted = false;
  run(h, 1000, false);
  check(world.actions.size() >= 3 && world.actions[0].rfind("gps.port", 0) == 0 &&
        world.actions[1].rfind("imu.softreset", 0) == 0 && world.actions[2].rfind("env.restart", 0) == 0,
        "all due at once on the ground: GPS, then IMU, then baro");

  Health &a = *fresh();
  world.busFaulted = true;
  world.envAlive = false;
  world.gpsTalking = false;
  world.imuFlowing = 0;
  run(a, 15000, true);
  world.busFaulted = false;
  run(a, 1000, true);
  check(world.actions.size() >= 3 && world.actions[0].rfind("imu.softreset", 0) == 0 &&
        world.actions[1].rfind("env.restart", 0) == 0 && world.actions[2].rfind("gps.port", 0) == 0,
        "all due at once in the air: IMU, then baro, then GPS");
}

static void testGps()
{
  Health &h = *fresh();
  run(h, 600000, false);
  check(world.actions.empty(), "GPS talking with no fix: never touched");   // the fake has no fix at all
  world.gpsTalking = false;
  run(h, 10500, false);
  check(countOf("gps.port") == 1 && logged("W GPS") && logged("reason=nobytes"), "10 s silent: port restarted");
  run(h, 5500, false);
  check(countOf("gps.hotstart") == 1 && logged("how=hotstart"), "5 s more: hot start");
  run(h, 5500, false);
  check(logged("W GPS") && logged("step=aside"), "then set aside");
}

static void testBusFirst()
{
  Health &h = *fresh();
  world.envAlive = false;
  world.busFaulted = true;
  world.busClearDue = true;
  run(h, 10000, false);
  check(countOf("busclear") == 1 && countOf("env.restart") == 0, "bus faulted: the clear runs, no sensor step over a bus that is down");
  world.busFaulted = false;
  run(h, 300, false);
  check(countOf("env.restart") == 1, "bus back: the baro's reset that was due goes ahead");
}

static void testImu()
{
  // all four silent in the air: dead, so reset there and then
  {
    Health &h = *fresh();
    world.imuFlowing = 0;
    run(h, 3500, true);
    check(countOf("imu.softreset") == 1 && logged("W IMU") && logged("reason=silent silent=1111"), "IMU silent in the air: soft reset");
    // the driver's restart brings it back, reports flowing
    world.imuReady = true;
    world.imuFlowing = 0x0F;
    run(h, 3000, true);
    check(logged("I IMU") && logged("recover step=ok") && countOf("imu.restart") == 0, "and back: ok, no full restart");
  }
  // one report silent in the air: partly working, held for the ground
  {
    Health &h = *fresh();
    world.imuFlowing = 0x0B;   // mag stopped
    run(h, 60000, true);
    check(countOf("imu") == 0 && logged("recover step=defer reason=partial silent=0010"), "mag silent in the air: deferred, accel and gyro untouched");
    run(h, 300, false);
    check(countOf("imu.softreset") == 1, "on the ground: the deferred reset goes ahead");
  }
  // reset doesn't help: soft reset, full restart, aside
  {
    Health &h = *fresh();
    world.imuFlowing = 0;
    run(h, 3500, false);
    world.imuReady = true;   // comes back up but stays silent
    run(h, 6000, false);
    check(countOf("imu.restart") == 1, "still silent after the soft reset: full restart");
    world.imuReady = true;
    run(h, 8000, false);
    check(logged("recover step=aside reason=silent backoffms=1000 attempts=2") && countOf("imu.restart") == 1 &&
          countOf("imu.softreset") == 2, "still silent: set aside 1 s, then the soft reset again");
  }
  // HP: asleep for a TX then woken, reports back within the grace: nothing said
  {
    Health &h = *fresh();
    run(h, 3000, false);
    world.imuFlowing = 0;
    run(h, 200, false);   // a TX without the controller knowing would not look like this; the wake does
    h.imuWoke();
    run(h, 900, false);
    world.imuFlowing = 0x0F;
    run(h, 5000, false);
    check(captured.empty() && world.actions.empty(), "wake: reports back within the grace, nothing said");
  }
  // asleep in the loop: the wake failed, counts as dead
  {
    Health &h = *fresh();
    world.imuAsleep = true;
    run(h, 3500, false);
    check(logged("reason=asleep") && countOf("imu.softreset") == 1, "asleep outside a TX: soft reset");
  }
}

int main()
{
  static ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
  ChipSatLog::begin(console, chipSystem);
  testAllGood();
  testEnvLadder();
  testAirCeiling();
  testOneStepAPass();
  testGroundOrder();
  testGps();
  testBusFirst();
  testImu();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
