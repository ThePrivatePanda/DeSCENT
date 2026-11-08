// FlightController's loop on the laptop: the real phase, watchdogs, flash log and settings, fake
// drivers and a chip kept in RAM, and the landed states down to the sleep between packets. Once per
// module
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
  uint32_t moveAtMs = 0xFFFFFFFF;    // and the IMU a running drone from here on
  uint32_t landAtMs = 0xFFFFFFFF;    // both back on the ground and still from here on
  uint32_t fixFromMs = 0xFFFFFFFF;   // a trusted fix every second from here
  uint32_t fixHaccMm = 5000;         // plus 100 mm per second past each fifth, so the run's best is the fifth
  uint32_t fixMissingMs = 0xFFFFFFFF;   // this second's NAV-PVT never comes
  int standbys = 0;
  int wakes = 0;
  int gpsTouchedAsleep = 0;   // restarts while in standby
  uint32_t standbyMs = 0;
  std::vector<uint8_t> chipAtStandby;
  std::vector<std::pair<uint32_t, uint16_t>> intervals;   // when the IMU was set to which interval
  bool radioAsleep = false;
  int radioSleeps = 0;
  int txFromAwake = 0;   // transmissions after the first that found the radio not asleep
  std::vector<std::vector<uint8_t>> sent;
  std::vector<uint32_t> sleeps;
  int sleepsWithRing = 0;
  int sleepsFlashOn = 0;
  uint32_t toaMs = 500;
  std::vector<uint32_t> txStart;
  std::vector<uint32_t> txEnd;
  uint32_t txStartedMs = 0;
  float imuAccel = 0.1f;     // above 2 m/s2 is moving
  float gaugePercent = 80.0f;
  bool gaugeValid = true;
  uint32_t imuStopsAtMs = 0xFFFFFFFF;   // no reports from here on, the hub still answers
  bool startFails = false;
  std::string boot;   // the IMU's pin, Wire and the IMU's begin, in the order they ran
};
static World world;

// the chip, as in test_flash_log
static std::vector<uint8_t> memory(ChipSatConstants::kFlashSize, 0xFF);
static uint32_t eraseDoneMs = 0;
static bool eraseStuck = false;
static bool nextEraseHangs = false;
static int programs = 0;
static ChipSatApp::FlashLog *currentLog = nullptr;
static ChipSatDevices::Flash *currentFlash = nullptr;

// the packet records of one chip image between two uptimes, as uptimes in chip order
static std::vector<uint32_t> packetsIn(const std::vector<uint8_t> &chip, uint32_t fromMs, uint32_t toMs)
{
  std::vector<uint32_t> out;
  LogRecord record;
  for (uint32_t index = 0; index < ChipSatApp::kLogRecordCount; ++index) {
    if (ChipSatApp::decodeRecord(&chip[index * ChipSatApp::kLogRecordSize], record) &&
        record.type == ChipSatApp::kLogTypePacket && record.uptimeMs >= fromMs && record.uptimeMs <= toMs) {
      out.push_back(record.uptimeMs);
    }
  }
  return out;
}

namespace ChipSatPlatform
{
uint32_t System::nowMs() const { return fakeNow; }
void System::waitMs(uint32_t ms) const { fakeNow += ms; }
void System::idleUntilTick() const {}
uint32_t System::sleepFor(uint32_t ms) const
{
  world.sleeps.push_back(ms);
  world.sleepsWithRing += currentLog->waiting() > 0 ? 1 : 0;
  world.sleepsFlashOn += currentFlash->paused() ? 0 : 1;
  fakeNow += ms;
  return ms;
}
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

Status I2cBus::begin(uint32_t, uint32_t) { world.boot += "wire "; return Status::Ok; }
bool I2cBus::recover(uint32_t) { return false; }
bool I2cBus::faulted(uint32_t) const { return false; }
bool I2cBus::imuResetDue(uint32_t) const { return false; }
}

namespace ChipSatDevices
{
// lying still, a report every interval
Imu::Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status Imu::begin(uint16_t ms) { world.boot += "imu "; reportIntervalMs_ = ms; ready_ = true; return Status::Ok; }
Status Imu::setReportInterval(uint16_t ms)
{
  reportIntervalMs_ = ms;
  world.intervals.push_back({fakeNow, ms});
  return Status::Ok;
}
void Imu::service()
{
  if (!ready_ || sleeping_ || fakeNow >= world.imuStopsAtMs) {
    return;
  }
  const uint32_t at = fakeNow - fakeNow % reportIntervalMs_;
  if (at == data_.linearAccelerationUpdatedMs) {
    return;
  }
  data_.linearAccelerationValid = data_.gyroscopeValid = data_.magnetometerValid = data_.orientationValid = true;
  data_.linearAccelerationUpdatedMs = data_.gyroscopeUpdatedMs = data_.magnetometerUpdatedMs = data_.orientationUpdatedMs = at;
  data_.linearAccelerationMps2.x = fakeNow >= world.moveAtMs && fakeNow < world.landAtMs ? 5.0f : world.imuAccel;
  for (uint8_t i = 0; i < 4; ++i) {
    ++events_[i];
  }
}
Status Imu::sleep() { sleeping_ = true; return Status::Ok; }
Status Imu::wake() { sleeping_ = false; return Status::Ok; }
Status Imu::resetHub() { return Status::Ok; }
Status Imu::fullRestart() { return Status::Ok; }
void Imu::pinReset(uint8_t reason) { world.boot += "pin" + std::to_string(reason) + " "; }

// a NAV-PVT a second from fixFromMs, each a trusted fix, its latitude the second it came. Never UTC
Gps::Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system) : port_(port), system_(system) {}
Status Gps::begin() { ready_ = true; return Status::Ok; }
Status Gps::read() { return Status::Ok; }
void Gps::service()
{
  const uint32_t second = fakeNow - fakeNow % 1000;
  if (standby_ || fakeNow < world.fixFromMs || (pvtSeen_ && second == lastPvtMs_) || second == world.fixMissingMs) {
    return;
  }
  pvtSeen_ = byteSeen_ = true;
  lastPvtMs_ = lastByteMs_ = second;
  data_.valid = true;
  data_.updatedMs = second;
  data_.latitudeE7 = 370000000 + static_cast<int32_t>(second / 1000);
  data_.horizontalAccuracyMm = world.fixHaccMm + (second / 1000 % 5) * 100;
}
void Gps::restartPort() { world.gpsTouchedAsleep += standby_ ? 1 : 0; }
void Gps::hotStart() { world.gpsTouchedAsleep += standby_ ? 1 : 0; }
void Gps::standby()
{
  standby_ = true;
  data_.valid = false;
  ++world.standbys;
  world.standbyMs = fakeNow;
  world.chipAtStandby = memory;
}
Status Gps::wake()
{
  standby_ = false;
  ++world.wakes;
  return Status::Ok;
}

EnvSensor::EnvSensor(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system) : bus_(bus), system_(system) {}
Status EnvSensor::begin() { ready_ = true; return Status::Ok; }
Status EnvSensor::startMeasurement() { return Status::Ok; }
Status EnvSensor::read()
{
  data_.valid = true;
  data_.updatedMs = fakeNow;
  data_.altitudeM = fakeNow >= world.climbAtMs && fakeNow < world.landAtMs ? world.groundM + world.climbM : world.groundM;
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
Status Radio::startTransmit(const uint8_t *data, size_t length)
{
  if (world.startFails) {
    return Status::Failed;
  }
  world.txFromAwake += !world.txStart.empty() && !world.radioAsleep ? 1 : 0;
  world.radioAsleep = false;   // the HAL's wake, before the first command
  world.sent.emplace_back(data, data + length);
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
Status Radio::sleep()
{
  world.radioAsleep = true;
  ++world.radioSleeps;
  return Status::Ok;
}

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

// The air at airAtMs: the IMU moves from 12 s before, enough at 1 Hz to wake it and fill the window
// at 20 Hz, and the barometer climbs from the whole second before, so that second's lowest is the
// climb on the first reading after airAtMs's whole second starts
static std::unique_ptr<Board> boot(uint32_t airAtMs)
{
  fakeNow = 0;
  captured.clear();
  world = World();
  if (airAtMs != 0xFFFFFFFF) {
    world.climbAtMs = airAtMs - airAtMs % 1000 - 1000;
    world.moveAtMs = airAtMs - 12000;
  }
  std::fill(memory.begin(), memory.end(), 0xFF);
  eraseDoneMs = 0;
  eraseStuck = false;
  nextEraseHangs = false;
  programs = 0;
  std::unique_ptr<Board> board(new Board());
  currentLog = &board->log;
  currentFlash = &board->flash;
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
  followBattery(logRate(ChipSatApp::Phase::Air), 400, 400);
  // handled 5 s in every 10: never at rest, never the air
  for (uint32_t ms = 5000; ms <= 70000; ms += 5000) {
    world.imuAccel = ms % 10000 ? 5.0f : 0.1f;
    runUntil(*board, ms);
  }
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

// The air at SF12 with no gap: on air 3187 ms of every cycle (config.h), the way the new boards fly.
// Every 50 ms record still has to reach the chip and the IMU has to be read through each
// transmission. The drone keeps flying, so the board stays in the air throughout
static void testBackToBackSf12()
{
  std::unique_ptr<Board> board = boot(40000);
  world.toaMs = 3187;
  auto &settings = const_cast<ChipSatSettings::Settings &>(ChipSatSettings::active());
  settings.transmit[static_cast<size_t>(ChipSatApp::Phase::Air)].fixedMs = 0;
  runUntil(*board, 200000);

  uint32_t records = 0, imuValid = 0, worstGap = 0, last = 0;
  LogRecord record;
  for (uint32_t index = 0; index < ChipSatApp::kLogRecordCount; ++index) {
    if (!ChipSatApp::decodeRecord(&memory[index * ChipSatApp::kLogRecordSize], record) ||
        record.type != ChipSatApp::kLogTypePacket || record.uptimeMs < 45000 || record.uptimeMs > 195000) {
      continue;
    }
    ++records;
    imuValid += record.packet.sensorValidity & 1;
  }
  const std::vector<uint32_t> ups = packetUptimes();
  for (uint32_t up : ups) {
    if (up >= 45000 && up <= 195000) {
      worstGap = last ? std::max(worstGap, up - last) : 0;
      last = up;
    }
  }
  const size_t packets = txAfter(45000) < world.txStart.size() ? world.txStart.size() - txAfter(45000) : 0;
  printf("     45-195 s: %u records, IMU valid on %u, worst gap %u ms, %u packets, dropped %u\n",
         static_cast<unsigned int>(records), static_cast<unsigned int>(imuValid),
         static_cast<unsigned int>(worstGap), static_cast<unsigned int>(packets),
         static_cast<unsigned int>(board->log.dropped()));
  check(packets >= 40, "back to back at SF12: a packet about every 3.2 s");
  check(board->log.dropped() == 0 && records >= 2990 && worstGap <= 60, "back to back at SF12: every 50 ms record is on the chip");
  check(imuValid == records, "back to back at SF12: the IMU is read through every transmission");
}

// this boot's system records on the chip, as source, what, part, reason, count
struct SystemRecord
{
  uint8_t source, what, part, reason;
  uint16_t count;
};

static std::vector<SystemRecord> systemOnChip()
{
  std::vector<SystemRecord> out;
  LogRecord r;
  for (uint32_t index = 0; index < ChipSatApp::kLogRecordCount; ++index) {
    if (ChipSatApp::decodeRecord(&memory[index * ChipSatApp::kLogRecordSize], r) && r.type == ChipSatApp::kLogTypeEvent &&
        r.eventKind == ChipSatApp::kLogEventSystem) {
      out.push_back({r.eventData[1], r.eventData[2], r.eventData[3], r.eventData[4],
                     static_cast<uint16_t>(r.eventData[5] | r.eventData[6] << 8)});
    }
  }
  return out;
}

static bool onChip(uint8_t source, uint8_t what, uint8_t part, uint8_t reason)
{
  for (const SystemRecord &r : systemOnChip()) {
    if (r.source == source && r.what == what && r.part == part && r.reason == reason && r.count > 0) {
      return true;
    }
  }
  return false;
}

// A reflash leaves the IMU as it was, so it gets its NRST before Wire starts and before it is started
static void testImuPinAtBoot()
{
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  check(world.boot == "pin" + std::to_string(ChipSatLog::kWhyBoot) + " wire imu ", "boot: IMU pin reset, then Wire, then the IMU's begin");
}

// The IMU's reports stop on the ground: the ladder's steps reach the chip, and the console as before
static void testSystemSensor()
{
  using namespace ChipSatLog;
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  world.imuStopsAtMs = 40000;
  runUntil(*board, 120000);
  check(onChip(kSourceSensor, kSensorRetry, kPartImu, kWhySilent) && onChip(kSourceSensor, kSensorReset, kPartImu, kWhySilent),
        "sensor: the IMU's retry and soft reset are on the chip, on the ground");
  check(countOf("recover step=retry reason=silent") > 0 && countOf("recover step=reset how=softreset") > 0,
        "sensor: and on the console as before");
}

// The take-off and the end of the boot-air
static void testSystemPhase()
{
  using namespace ChipSatLog;
  std::unique_ptr<Board> board = boot(60100);
  runUntil(*board, 70000);
  check(onChip(kSourcePhase, kPhaseBootAirEnd, kPartImu, kWhyQuiet), "phase: boot-air ending at rest is on the chip");
  check(onChip(kSourcePhase, kPhaseToAir, kPartImu, kWhyImuMotion), "phase: the take-off on IMU motion is on the chip");
}

// A radio that refuses to start: set up again once, then the packet is lost
static void testSystemRadio()
{
  using namespace ChipSatLog;
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  runUntil(*board, 50000);
  world.startFails = true;
  runUntil(*board, 110000);
  check(onChip(kSourceRadio, kRadioResetup, kPartRadio, kWhySetupOk) &&
        onChip(kSourceRadio, kRadioTxFail, kPartRadio, kWhyStartRefused),
        "radio: the set up again and the refused start are on the chip");
  check(countOf(": done ok=0") > 0, "radio: the console still says the TX failed");
}

// Off the air records come at the faster of the ground gap and the air's fixed gap, and the ground
// keeps one per its own gap (0 every record). Board 18 at 20 ms ground, 100 s air made one per 100 s
static void testGroundGapRules()
{
  ChipSatSettings::Rate &ground = logRate(ChipSatApp::Phase::Ground);
  ChipSatSettings::Rate &air = logRate(ChipSatApp::Phase::Air);
  const ChipSatSettings::Rate groundWas = ground, airWas = air;
  struct Case { uint32_t groundMs, airMs; };
  for (Case c : {Case{20, 100000}, Case{0, 100000}}) {
    // setup() reads the defaults again, and the ground gap moves at the first cycle, 60 s
    std::unique_ptr<Board> board = boot(0xFFFFFFFF);
    ground.fromSoc = air.fromSoc = false;
    ground.fixedMs = c.groundMs;
    air.fixedMs = c.airMs;
    runUntil(*board, 225000);   // the ring holds the last 580 back
    uint32_t least, most, count;
    gapsBetween(62000, 182000, least, most, count);
    if (c.groundMs == 20) {
      check(count >= 5900 && count <= 6000 && most <= 25 && board->log.dropped() == 0,
            "ground 20 ms, air 100 s: two minutes on the ground are about 6000 records, one per 20 ms");
    } else {
      check(count >= 2350 && count <= 2400 && least >= 45 && most <= 55 && board->log.dropped() == 0,
            "ground 0, air 100 s: two minutes on the ground are one record per IMU reading");
    }
  }

  std::unique_ptr<Board> board = boot(130000);   // the defaults, 10 s and 50 ms
  runUntil(*board, 140000);
  uint32_t least, most, count;
  gapsBetween(40000, 130000 - 31000, least, most, count);
  check(count >= 4 && least >= 10000 && most <= 10100, "ground 10 s, air 50 ms: one record per 10 s on the chip on the ground");
  gapsBetween(130000 - 28000, 138000, least, most, count);
  check(count > 700 && most <= 60 && board->log.dropped() == 0, "ground 10 s, air 50 ms: the take-off writes the last 30 s at 50 ms");
  ground = groundWas;
  air = airWas;
}

// Board 18 on 2026-11-06, ground 0 and air 200 s, pulled 9 s after boot-air ended: nothing on the chip
// but its events, because the ground held every record in RAM until 580 were pending. With a ground
// gap of 0 they go to the chip as they come
static void testGroundZeroReachesTheChipAtOnce()
{
  ChipSatSettings::Rate &ground = logRate(ChipSatApp::Phase::Ground);
  ChipSatSettings::Rate &air = logRate(ChipSatApp::Phase::Air);
  const ChipSatSettings::Rate groundWas = ground, airWas = air;
  std::unique_ptr<Board> board = boot(0xFFFFFFFF);
  ground.fromSoc = air.fromSoc = false;
  ground.fixedMs = 0;
  air.fixedMs = 200000;
  runUntil(*board, 72000);   // the first cycle, at 60 s, puts these gaps in force
  uint32_t least, most, count;
  gapsBetween(61000, 71000, least, most, count);
  check(count >= 190 && most <= 60, "ground 0: the last 10 s are on the chip already, one record per IMU reading");
  ground = groundWas;
  air = airWas;
}

// when the IMU went to 1 Hz after the flight: the landing
static uint32_t landedMs()
{
  for (const auto &i : world.intervals) {
    if (i.first > 70000 && i.second == ChipSatConfig::kImuGroundIntervalMs) {
      return i.first;
    }
  }
  return 0;
}

// Up at 60 s, down at 120 s, still from there. Fixes from fixFromMs with hAcc around haccMm
static std::unique_ptr<Board> flyAndLand(uint32_t fixFromMs, uint32_t haccMm)
{
  std::unique_ptr<Board> board = boot(60100);
  world.landAtMs = 120000;
  world.fixFromMs = fixFromMs;
  world.fixHaccMm = haccMm;
  return board;
}

// The fix to hold: five trusted fixes in a row under 10 m, the first 30 s after landing; under 20 m
// once 5 min have passed; a missing second starts the five again; 30 m never does
static void testGoodFix()
{
  std::unique_ptr<Board> board = flyAndLand(100000, 5000);
  runUntil(*board, 230000);
  const uint32_t landed = landedMs();
  const uint32_t good = world.standbyMs;
  printf("     landed %u ms, asleep %u ms\n", static_cast<unsigned int>(landed), static_cast<unsigned int>(good));
  check(landed > 145000 && landed < 155000, "landing 30 s after the board came to rest below half the climb");
  check(world.standbys == 1 && good >= landed + 34000 && good <= landed + 36000,
        "5 m fixes all along: asleep on the fifth from 30 s after landing");

  board = flyAndLand(100000, 5000);
  world.fixMissingMs = good - good % 1000 - 2000;   // the third of the five
  runUntil(*board, 230000);
  check(world.standbys == 1 && world.standbyMs >= good + 2000, "a missing second inside the five starts them again");

  board = flyAndLand(100000, 15000);
  runUntil(*board, landed + 290000);
  check(world.standbys == 0, "15 m fixes: not good enough for 5 min");
  runUntil(*board, landed + 320000);
  check(world.standbys == 1 && world.standbyMs >= landed + 300000 && world.standbyMs <= landed + 306000,
        "and held once 5 min have passed, under 20 m");

  board = flyAndLand(100000, 30000);
  runUntil(*board, landed + 400000);
  check(world.standbys == 0, "30 m fixes: never held, the GPS keeps trying");
}

// Landed with a fix held, then left ten minutes, then flown again
static void testAsleep()
{
  std::unique_ptr<Board> board = flyAndLand(100000, 5000);
  runUntil(*board, 230000);
  const uint32_t landed = landedMs();
  const uint32_t asleepMs = world.standbyMs;
  const std::vector<uint32_t> fall = packetsIn(world.chipAtStandby, 62000, landed - 1000);
  uint32_t worst = 0;
  for (size_t i = 1; i < fall.size(); ++i) {
    worst = std::max(worst, fall[i] - fall[i - 1]);
  }
  printf("     fall on the chip at standby: %u records, worst gap %u ms\n", static_cast<unsigned int>(fall.size()),
         static_cast<unsigned int>(worst));
  check(fall.size() >= (landed - 1000 - 62000) / 50 && worst <= 50, "the whole fall, every 50 ms, is on the chip before anything sleeps");

  const size_t sentBefore = world.sent.size();
  const size_t txBefore = world.txStart.size();
  const size_t capturedAt = captured.size();
  world.sleeps.clear();
  runUntil(*board, asleepMs + 600000);
  check(world.standbys == 1 && world.wakes == 0 && world.gpsTouchedAsleep == 0, "asleep: standby once, the GPS left alone after");
  check(captured.find("recover", capturedAt) == std::string::npos, "asleep ten minutes: Health touched nothing");

  bool held = world.sent.size() > sentBefore + 5;
  for (size_t i = sentBefore; i < world.sent.size(); ++i) {
    ChipSatTelemetry::TelemetryPacket p;
    memcpy(&p, world.sent[i].data(), sizeof p);
    held = held && (p.sensorValidity & ChipSatTelemetry::kGPSValidBit) && (p.latitudeE7 - 370000000) % 5 == 0 &&
           p.latitudeE7 > 370000000 + static_cast<int32_t>(landed / 1000);
  }
  check(held, "asleep: every packet carries the best fix of the run, GPS valid");

  uint32_t least = 0xFFFFFFFF, most = 0;
  bool moved = false;
  for (size_t i = txBefore + 1; i < world.txStart.size(); ++i) {
    const uint32_t gap = world.txStart[i] - world.txEnd[i - 1];
    least = std::min(least, gap);
    most = std::max(most, gap);
    moved = moved || gap != world.txStart[txBefore + 1] - world.txEnd[txBefore];
  }
  printf("     asleep: %u packets, gaps %u to %u ms, %u sleeps\n", static_cast<unsigned int>(world.txStart.size() - txBefore),
         static_cast<unsigned int>(least), static_cast<unsigned int>(most), static_cast<unsigned int>(world.sleeps.size()));
  check(least >= 58000 && most <= 62200 && moved, "asleep: the ground gap give or take 2 s, not the same each time");
  check(countOf("asleep nexts=") >= world.txStart.size() - txBefore - 1, "one console line per packet while asleep");

  const std::vector<uint32_t> records = packetsIn(memory, asleepMs + 1000, asleepMs + 600000);
  bool tenS = records.size() >= 58;
  for (size_t i = 1; i < records.size(); ++i) {
    tenS = tenS && records[i] > records[i - 1] && records[i] - records[i - 1] >= 10000 && records[i] - records[i - 1] <= 10100;
  }
  check(tenS, "asleep: a record per 10 s, in time order on the chip");

  uint32_t longest = 0;
  for (uint32_t ms : world.sleeps) {
    longest = std::max(longest, ms);
  }
  check(world.sleeps.size() > 500 && longest <= 1000 && world.sleepsWithRing == 0 && world.sleepsFlashOn == 0,
        "Stop2 between the IMU's polls, never with the ring holding records or the chip awake");

  // picked up and flown again
  const uint32_t movedMs = fakeNow + 500;
  world.moveAtMs = movedMs;
  world.climbAtMs = movedMs + 11000;
  world.landAtMs = 0xFFFFFFFF;
  const size_t txMoved = world.txStart.size();
  runUntil(*board, movedMs + 30000);
  uint32_t fastMs = 0;
  for (const auto &i : world.intervals) {
    fastMs = fastMs == 0 && i.first > movedMs && i.second == ChipSatConfig::kImuReportIntervalMs ? i.first : fastMs;
  }
  size_t fastTx = 0;
  for (size_t i = txMoved + 1; i < world.txStart.size(); ++i) {
    fastTx += world.txStart[i] - world.txEnd[i - 1] < 2000 ? 1 : 0;
  }
  check(world.wakes == 1 && fastMs > movedMs && fastMs < movedMs + 4000, "moved: woken at the 20 Hz step, a few seconds in");
  check(!board->flash.paused() && fastTx > 5, "and then the air: the chip awake, packets at the air's gap");
  check(world.txFromAwake == 0 && world.radioSleeps == static_cast<int>(world.txStart.size()),
        "the radio asleep after every packet, in every state, woken for the next");
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
  testBackToBackSf12();
  testGroundZeroReachesTheChipAtOnce();
  testImuPinAtBoot();
  testSystemSensor();
  testSystemPhase();
  testSystemRadio();
  testGroundGapRules();
  testGoodFix();
  testAsleep();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
