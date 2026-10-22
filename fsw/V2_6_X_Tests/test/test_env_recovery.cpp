// EnvSensorBme280.cpp against a fake BME280, paced like FlightController::sampleSensors: a chip
// missing at boot has to come back once it answers
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock test/test_env_recovery.cpp ../V2_6_X_Code/src/log/Log.cpp -o /tmp/envrec && /tmp/envrec
#include <cstdio>
#include <cstring>
#include <string>
#include "../../V2_6_X_Code/src/devices/own/EnvSensorBme280.cpp"

using ChipSatPlatform::Status;

static uint32_t fakeNow = 0;
static bool present = false;
static int transfers = 0;

// the plausible calibration of test_env_compensation.cpp
static const uint8_t kCalib1[26] = {
  0x70, 0x6B, 0x43, 0x67, 0x18, 0xFC,
  0x9A, 0x8E, 0xD8, 0xD6, 0xD0, 0x0B, 0x27, 0x0B, 0x8C, 0x00,
  0xF9, 0xFF, 0x8C, 0x3C, 0xF8, 0xC6, 0x70, 0x17,
  0x00, 0x4B,
};
static const uint8_t kCalib2[7] = {0x62, 0x01, 0x00, 0x15, 0x2D, 0x03, 0x1E};

static void put20(uint8_t *at, uint32_t v)
{
  at[0] = static_cast<uint8_t>(v >> 12);
  at[1] = static_cast<uint8_t>(v >> 4);
  at[2] = static_cast<uint8_t>((v & 0x0F) << 4);
}

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

Status I2cBus::write(uint8_t, const uint8_t *, size_t)
{
  ++transfers;
  return present ? Status::Ok : Status::NoAck;
}

Status I2cBus::writeThenRead(uint8_t, const uint8_t *tx, size_t, uint8_t *rx, size_t rxLength)
{
  ++transfers;
  if (!present) {
    return Status::NoAck;
  }
  memset(rx, 0, rxLength);
  switch (tx[0]) {
    case ChipSatConstants::kEnvRegChipId: rx[0] = ChipSatConstants::kEnvChipIdValue; break;
    case ChipSatConstants::kEnvRegCalib1: memcpy(rx, kCalib1, sizeof kCalib1); break;
    case ChipSatConstants::kEnvRegCalib2: memcpy(rx, kCalib2, sizeof kCalib2); break;
    case ChipSatConstants::kEnvRegData:
      put20(&rx[0], 415148);
      put20(&rx[3], 519888);
      rx[6] = 0x66;
      rx[7] = 0x00;
      break;
    default: break;   // status: idle
  }
  return Status::Ok;
}
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

static ChipSatPlatform::System chipSystem;
static ChipSatPlatform::I2cBus bus;

// FlightController::sampleSensors' barometer part, a pass every 10 ms. Returns when it first read valid
static uint32_t fly(ChipSatDevices::EnvSensor &env, uint32_t forMs)
{
  uint32_t startedMs = fakeNow;
  bool measuring = false;
  const uint32_t until = fakeNow + forMs;
  while (fakeNow < until) {
    if (!measuring) {
      if (fakeNow - startedMs >= ChipSatConfig::kEnvIntervalMs) {
        measuring = env.startMeasurement() == Status::Ok;
        startedMs = fakeNow;
      }
    } else if (fakeNow - startedMs >= ChipSatConfig::kEnvConversionMs) {
      env.read();
      measuring = false;
    }
    if (env.data().valid) {
      return fakeNow;
    }
    fakeNow += 10;
  }
  return 0;
}

int main()
{
  {
    fakeNow = 1000;
    present = false;
    ChipSatDevices::EnvSensor env(bus, chipSystem);
    check(env.begin() != Status::Ok && !env.ready(), "boot: no BME280, init fails");
    fly(env, 4000);
    present = true;
    const uint32_t up = fakeNow;
    const uint32_t valid = fly(env, 2000);
    check(valid != 0 && valid - up <= 600, "answers 5 s after boot: reading valid within 0.6 s (fast retry)");
    check(env.data().updatedMs >= up && env.data().pressurePa >= 30000.0f && env.data().pressurePa <= 110000.0f,
          "and it is a fresh reading from the chip");
  }
  {
    fakeNow = 1000;
    present = false;
    ChipSatDevices::EnvSensor env(bus, chipSystem);
    env.begin();
    transfers = 0;
    fly(env, 59000);
    check(transfers < 140, "a missing chip a minute: one unanswered address per try, not per pass");
    present = true;
    const uint32_t up = fakeNow;
    const uint32_t valid = fly(env, 12000);
    check(valid != 0 && valid - up <= 10500, "answers after a minute: valid within the 10 s slow retry");
  }
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
