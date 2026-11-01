// FlightController's loop on the laptop: the real phase, watchdogs, flash log and settings, fake
// drivers and a chip kept in RAM. Once per module, HP pauses the flash and sleeps the IMU during TX
//   for m in 1 2; do g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -DCHIPSAT_RADIO_MODULE=$m test/test_flight_loop.cpp ../V2_6_X_Code/src/app/{FlightController,FlashLog,Phase,Health,SensorWatch,Telemetry,RateGap,ImuValidity}.cpp ../V2_6_X_Code/src/Settings.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/flightloop$m && /tmp/flightloop$m; done
#include <cstdio>
#include <cstring>
#include <algorithm>
#include <memory>
#include <string>
#include <vector>
#include "app/FlightController.h"
#include "Settings.h"

using namespace ChipSatPlatform;
using ChipSatApp::FlashLog;
using ChipSatApp::FlightController;
using ChipSatApp::LogRecord;

static uint32_t fakeNow = 0;
static std::string captured;

struct World
{
  float groundM = 100.0f;
  float climbM = 30.0f;
  uint32_t climbAtMs = 0xFFFFFFFF;   // the barometer reads groundM + climbM from here on
  uint32_t toaMs = 500;
  std::vector<uint32_t> txStart;
  std::vector<uint32_t> txEnd;
  uint32_t txStartedMs = 0;
  float imuAccel = 0.1f;     // above 2 m/s2 is moving
  float gaugePercent = 80.0f;
  bool gaugeValid = true;
};
static World world;

// the chip, as in test_flash_log
static std::vector<uint8_t> memory(ChipSatConstants::kFlashSize, 0xFF);
static uint32_t eraseDoneMs = 0;
static bool eraseStuck = false;
static bool nextEraseHangs = false;
static int programs = 0;

namespace ChipSatPlatform
{
uint32_t System::nowMs() const { return fakeNow; }
void System::waitMs(uint32_t ms) const { fakeNow += ms; }
void System::startWatchdog(uint32_t) {}
void System::feedWatchdog() const {}
ResetCause System::readAndClearResetCause() { return ResetCause(); }
void System::paintStack() {}
uint32_t System::stackUntouchedBytes() const { return 1000; }

Uart::Uart(UartPort port) : port_(port) {}
Status Uart::begin(uint32_t) { return Status::Ok; }
size_t Uart::write(const uint8_t *data, size_t length) { captured.append(reinterpret_cast<const char *>(data), length); return length; }
size_t Uart::write(const char *text) { captured += text; return strlen(text); }
size_t Uart::available() { return 0; }
int Uart::read() { return -1; }
void Uart::flush() {}

Status I2cBus::begin(uint32_t, uint32_t) { return Status::Ok; }
bool I2cBus::recover(uint32_t) { return false; }
bool I2cBus::faulted(uint32_t) const { return false; }
}

namespace ChipSatDevices
{
// lying still, a report every 50 ms
Imu::Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status Imu::begin(uint16_t) { ready_ = true; return Status::Ok; }
void Imu::service()
{
  if (!ready_ || sleeping_) {
    return;
  }
  const uint32_t at = fakeNow - fakeNow % 50;
  if (at == data_.linearAccelerationUpdatedMs) {
    return;
  }
  data_.linearAccelerationValid = data_.gyroscopeValid = data_.magnetometerValid = data_.orientationValid = true;
  data_.linearAccelerationUpdatedMs = data_.gyroscopeUpdatedMs = data_.magnetometerUpdatedMs = data_.orientationUpdatedMs = at;
  data_.linearAccelerationMps2.x = world.imuAccel;
  for (uint8_t i = 0; i < 4; ++i) {
    ++events_[i];
  }
}
Status Imu::sleep() { sleeping_ = true; return Status::Ok; }
Status Imu::wake() { sleeping_ = false; return Status::Ok; }
Status Imu::resetHub() { return Status::Ok; }
Status Imu::fullRestart() { return Status::Ok; }

// never a fix, never UTC
Gps::Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system) : port_(port), system_(system) {}
Status Gps::begin() { ready_ = true; return Status::Ok; }
Status Gps::read() { return Status::Ok; }
void Gps::service() {}
void Gps::restartPort() {}
void Gps::hotStart() {}

EnvSensor::EnvSensor(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status EnvSensor::begin() { ready_ = true; return Status::Ok; }
Status EnvSensor::startMeasurement() { return Status::Ok; }
Status EnvSensor::read()
{
  data_.valid = true;
  data_.updatedMs = fakeNow;
  data_.altitudeM = fakeNow >= world.climbAtMs ? world.groundM + world.climbM : world.groundM;
  return Status::Ok;
}
bool EnvSensor::restart() { return true; }

FuelGauge::FuelGauge(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status FuelGauge::begin() { ready_ = true; return Status::Ok; }
Status FuelGauge::read()
{
  data_.valid = world.gaugeValid;
  data_.updatedMs = fakeNow;
  data_.cellPercentage = world.gaugePercent;
  data_.cellVoltage = 3.9f;
  return Status::Ok;
}
bool FuelGauge::restart() { return true; }

Radio::Radio(ChipSatPlatform::System &system) : system_(system) {}
Status Radio::begin() { return Status::Ok; }
Status Radio::setCurrentLimit() { return Status::Ok; }
float Radio::currentLimitMa() { return 140.0f; }
Status Radio::setOutputPower() { return Status::Ok; }
Status Radio::applyPaConfig() { return Status::Ok; }
Status Radio::startTransmit(const uint8_t *, size_t)
{
  world.txStartedMs = fakeNow;
  world.txStart.push_back(fakeNow);
  return Status::Ok;
}
bool Radio::transmitDone() { return fakeNow - world.txStartedMs >= world.toaMs; }
Status Radio::finishTransmit()
{
  world.txEnd.push_back(fakeNow);
  return Status::Ok;
}
uint32_t Radio::timeOnAirMs(size_t) { return world.toaMs; }

void Led::begin() {}
void Led::transmitStarted() {}
void Led::transmitEnded() {}

Flash::Flash(ChipSatPlatform::System &system) : system_(system) {}
Status Flash::begin() { ready_ = true; paused_ = false; return Status::Ok; }
bool Flash::busy() { return !paused_ && (eraseStuck || fakeNow < eraseDoneMs); }
Status Flash::read(uint32_t address, uint8_t *buffer, size_t length)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  memcpy(buffer, &memory[address], length);
  return Status::Ok;
}
Status Flash::program(uint32_t address, const uint8_t *data, size_t length)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  ++programs;
  for (size_t i = 0; i < length; ++i) {
    memory[address + i] &= data[i];
  }
  return Status::Ok;
}
Status Flash::startSectorErase(uint32_t address)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  memset(&memory[address], 0xFF, ChipSatConstants::kFlashSectorSize);
  eraseDoneMs = fakeNow + 36;
  eraseStuck = nextEraseHangs;
  return Status::Ok;
}
Status Flash::pause() { paused_ = true; return Status::Ok; }
Status Flash::resume() { paused_ = false; return Status::Ok; }
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

static size_t countOf(const char *text)
{
  size_t n = 0;
  for (size_t at = captured.find(text); at != std::string::npos; at = captured.find(text, at + 1)) {
    ++n;
  }
  return n;
}

// One board, fresh chip and fresh fakes
struct Board
{
  System system;
  I2cBus i2c;
  Uart console{UartPort::Console};
  Uart gpsPort{UartPort::Gps};
  ChipSatDevices::Imu imu{i2c, system};
  ChipSatDevices::Gps gps{gpsPort, system};
  ChipSatDevices::EnvSensor env{i2c, system};
  ChipSatDevices::FuelGauge gauge{i2c, system};
  ChipSatDevices::Radio radio{system};
  ChipSatDevices::Led led;
  ChipSatDevices::Flash flash{system};
  FlashLog log{flash, system};
  FlightController flight{system, i2c, console, gpsPort, imu, gps, env, gauge, radio, led, flash, log};
};

static std::unique_ptr<Board> boot(uint32_t climbAtMs)
{
  fakeNow = 0;
  captured.clear();
  world = World();
  world.climbAtMs = climbAtMs;
  std::fill(memory.begin(), memory.end(), 0xFF);
  eraseDoneMs = 0;
  eraseStuck = false;
  nextEraseHangs = false;
  programs = 0;
  std::unique_ptr<Board> board(new Board());
  board->flight.setup();
  return board;
}

static void runUntil(Board &board, uint32_t untilMs)
{
  while (fakeNow < untilMs) {
    board.flight.loop();
    ++fakeNow;
  }
}

// uptimes of this boot's packet records on the chip, sorted
static std::vector<uint32_t> packetUptimes()
{
  std::vector<uint32_t> out;
  LogRecord record;
  for (uint32_t index = 0; index < ChipSatApp::kLogRecordCount; ++index) {
    if (ChipSatApp::decodeRecord(&memory[index * ChipSatApp::kLogRecordSize], record) &&
        record.type == ChipSatApp::kLogTypePacket) {
      out.push_back(record.uptimeMs);
    }
  }
  std::sort(out.begin(), out.end());
  return out;
}

// the first transmission that started after ms, as an index
static size_t txAfter(uint32_t ms)
{
  for (size_t i = 0; i < world.txStart.size(); ++i) {
    if (world.txStart[i] > ms) {
      return i;
    }
  }
  return world.txStart.size();
}

// The ground gap is 60 s, so the first cycle's packet goes out at 60 s and is on air 60.0-60.5 s
static void testChangeDuringTx()
{
  std::unique_ptr<Board> board = boot(60100);
  runUntil(*board, 63000);
  const size_t during = txAfter(59000);
  check(during + 1 < world.txStart.size() && world.txStart[during] < 60100 && world.txEnd[during] > 60100,
        "the climb lands inside a transmission");
  if (during + 1 < world.txStart.size()) {
    const uint32_t wait = world.txStart[during + 1] - world.txEnd[during];
    printf("     next packet %u ms after that one ended\n", static_cast<unsigned int>(wait));
    check(wait <= 20, "a phase change during the TX wait still makes the next packet due at once");
  }
}

static void testChangeBetweenTx()
{
  std::unique_ptr<Board> board = boot(70000);
  runUntil(*board, 72000);
  const size_t next = txAfter(61000);
  check(next < world.txStart.size() && world.txStart[next] >= 70000 && world.txStart[next] <= 70400,
        "a phase change between packets makes the next one due at once");
}

// boot-air ends about 30 s in (IMU quiet, barometer still), then the ground keeps one record per
// 10 s. The take-off at 60.1 s puts the ring's last 30 s on the chip
static void testRingOnTakeoff()
{
  std::unique_ptr<Board> board = boot(60100);
  runUntil(*board, 66000);
  const std::vector<uint32_t> ups = packetUptimes();
  uint32_t worstGap = 0;
  uint32_t inWindow = 0;
  for (size_t i = 1; i < ups.size(); ++i) {
    if (ups[i - 1] >= 60100 - 28000 && ups[i] <= 65000) {
      worstGap = std::max(worstGap, ups[i] - ups[i - 1]);
      ++inWindow;
    }
  }
  printf("     %u packet records, %u from 28 s before take-off to 65 s, worst gap %u ms, dropped %u\n",
         static_cast<unsigned int>(ups.size()), static_cast<unsigned int>(inWindow),
         static_cast<unsigned int>(worstGap), static_cast<unsigned int>(board->log.dropped()));
  check(inWindow > 600 && worstGap <= 60, "every 50 ms record from 28 s before take-off on is on the chip");
  check(board->log.dropped() == 0, "nothing dropped through the TX waits");

  uint32_t groundKept = 0;
  for (uint32_t up : ups) {
    groundKept += up >= 31000 && up < 60100 - 30000 ? 1 : 0;
  }
  check(groundKept == 0, "the ground before the ring's 30 s is only the trickle");
}

// The block is flashed before boot on a real board. Here the board has booted on the defaults and
// the rate is changed under it, which active() would not allow outside a test
static ChipSatSettings::Rate &logRate(ChipSatApp::Phase phase)
{
  auto &settings = const_cast<ChipSatSettings::Settings &>(ChipSatSettings::active());
  return settings.log[static_cast<size_t>(phase)];
}

// at or above 20 % one gap, below it another
static void followBattery(ChipSatSettings::Rate &rate, uint32_t chargedMs, uint32_t lowMs)
{
  rate.fromSoc = true;
  for (size_t i = 0; i < ChipSatSettings::kSocBands - 1; ++i) {
    rate.socIntervalMs[i] = chargedMs;
  }
  rate.socIntervalMs[ChipSatSettings::kSocBands - 1] = lowMs;
}

// gaps between packet records with uptimes in [fromMs, toMs]
static void gapsBetween(uint32_t fromMs, uint32_t toMs, uint32_t &least, uint32_t &most, uint32_t &count)
{
  const std::vector<uint32_t> ups = packetUptimes();
  least = 0xFFFFFFFF;
  most = 0;
  count = 0;
  for (size_t i = 1; i < ups.size(); ++i) {
    if (ups[i - 1] >= fromMs && ups[i] <= toMs) {
      least = std::min(least, ups[i] - ups[i - 1]);
      most = std::max(most, ups[i] - ups[i - 1]);
      ++count;
    }
  }
  printf("     %u to %u ms: %u gaps, %u to %u ms\n", static_cast<unsigned int>(fromMs), static_cast<unsigned int>(toMs),
         static_cast<unsigned int>(count), static_cast<unsigned int>(least), static_cast<unsigned int>(most));
}

// In the air the band for the charge is the gap, and it moves at the next cycle
static void testAirLogBands()
{
  std::unique_ptr<Board> board = boot(60100);
  followBattery(logRate(ChipSatApp::Phase::Air), 200, 500);
  runUntil(*board, 64000);
  uint32_t least, most, count;
  gapsBetween(61000, 64000, least, most, count);
  check(count > 10 && least >= 195 && most <= 260, "air log follows the battery: 200 ms at 80 %");
  check(countOf(" logms=200") > 0, "the Cyc line says the log gap");
  world.gaugePercent = 10.0f;
  runUntil(*board, 72000);
  gapsBetween(67000, 72000, least, most, count);
  check(count > 5 && least >= 495 && most <= 560, "and 500 ms once the charge is under 20 %");
  check(countOf(" logms=500") > 0, "the Cyc line follows it");
}

// On the ground the ground slot sets what reaches the chip
static void testGroundLogBands()
{
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  followBattery(logRate(ChipSatApp::Phase::Ground), 2000, 500);
  runUntil(*board, 115000);   // the first cycle is at 60 s, and the ring holds 29 s back
  uint32_t least, most, count;
  gapsBetween(62000, 85000, least, most, count);
  check(count > 8 && least >= 2000 && most <= 2100, "ground log follows the battery: one kept per 2 s");
  check(countOf(" logms=2000") > 0, "the Cyc line says the ground gap on the ground");
}

// The same rule as transmit: no gauge, the fixed gap
static void testLogGaugeUnread()
{
  std::unique_ptr<Board> board = boot(60100);
  world.gaugeValid = false;
  followBattery(logRate(ChipSatApp::Phase::Air), 200, 500);
  runUntil(*board, 66000);   // HP pauses the flash through each TX, so give the ring time
  uint32_t least, most, count;
  gapsBetween(61000, 64000, least, most, count);
  check(count > 50 && most <= 60, "with the gauge unread the air logs at its fixed gap");
}

// The generator keeps the air's fixed gap on the ground whatever the air's bands say. The ring only
// reaches the chip at 580 pending, so a slow gap here would hold the wrong 30 s at take-off
static void testRingKeepsFixedGapOnGround()
{
  std::unique_ptr<Board> board = boot(100000);
  followBattery(logRate(ChipSatApp::Phase::Air), 2000, 2000);
  runUntil(*board, 102000);
  uint32_t least, most, count;
  gapsBetween(100000 - 27000, 100000, least, most, count);
  check(count > 500 && most <= 60, "the ring on the ground ignores the air's bands");
}

// Boot-air is the ground to the radio and the air to the log, so its bands apply. It can end between
// two cycles, and the ring needs the fixed gap from that moment and not from the next cycle
static void testBootAirLogBands()
{
  std::unique_ptr<Board> board = boot(115000);
  world.imuAccel = 5.0f;   // handled, so boot-air doesn't end at rest
  followBattery(logRate(ChipSatApp::Phase::Air), 400, 400);
  runUntil(*board, 70000);
  check(board->log.air(), "still boot-air at 70 s");
  uint32_t least, most, count;
  gapsBetween(62000, 70000, least, most, count);
  check(count > 10 && least >= 395 && most <= 460, "boot-air logs at the air's band, the phase being ground");
  world.imuAccel = 0.1f;
  while (board->log.air() && fakeNow < 113000) {
    board->flight.loop();
    ++fakeNow;
  }
  const uint32_t endedMs = fakeNow;
  check(!board->log.air() && endedMs < 110000, "boot-air ended at rest, between two cycles");
  runUntil(*board, 118000);
  gapsBetween(endedMs + 100, 115000, least, most, count);
  check(count > 20 && most <= 60, "the fixed gap is back as soon as boot-air ends");
}

// an erase that never finishes stops logging for the boot, said once
static void testStuckErase()
{
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  nextEraseHangs = true;
  runUntil(*board, 20000);
  const int programsThen = programs;
  check(board->log.stopped(), "an erase past its timeout stops the log");
  runUntil(*board, 30000);
  check(countOf(": stopped records=") == 1, "said once on the console as stopped");
  check(countOf(": full") == 0 && countOf("full=") == 0 && countOf(" stopped=1") == 0, "never called full, and not stopped at boot");
  check(programs == programsThen, "nothing programmed after it stopped");
}

int main()
{
  printf("module %s\n", ChipSatSettings::active().module == ChipSatSettings::kModuleHp ? "hp" : "le");
  testChangeDuringTx();
  testChangeBetweenTx();
  testRingOnTakeoff();
  testAirLogBands();
  testGroundLogBands();
  testLogGaugeUnread();
  testRingKeepsFixedGapOnGround();
  testBootAirLogBands();
  testStuckErase();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
