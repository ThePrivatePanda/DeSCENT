// Telemetry.cpp and TxInterval.cpp on the laptop. Writes 20000 packets to the file you name, diff
// that against test_v25_telemetry's to check the ground station can still decode us
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -I../V2_6_X_Code/src/app test/test_telemetry.cpp ../V2_6_X_Code/src/app/{Telemetry,TxInterval}.cpp -o /tmp/enc && /tmp/enc /tmp/enc.txt
#include <stdint.h>
#include <cstdio>
#include <cstring>
#include <cmath>
#include <string>
#include "Telemetry.h"


#include "TxInterval.h"
using ChipSatApp::txIntervalFromSoc;

using ChipSatSensors::SensorData;
using ChipSatTelemetry::TelemetryPacket;

static int failures = 0, checks = 0;
static void check(bool ok, const char *what) {
  ++checks; if (!ok) ++failures;
  printf("%s %s\n", ok ? "ok  " : "FAIL", what);
}
static std::string hexOf(const TelemetryPacket &p) {
  const uint8_t *b = reinterpret_cast<const uint8_t *>(&p); std::string s; char t[4];
  for (size_t i = 0; i < sizeof p; ++i) { snprintf(t, sizeof t, i ? " %02X" : "%02X", b[i]); s += t; }
  return s;
}

// Packet test vector from the software report
static const char *kVectorHex =
  "5C 57 4C 19 50 BD 66 D2 90 D0 03 00 B4 5F 00 00 7B 00 38 FE D4 03 0A 00 EC FF 1E 00 FA 00 83 FF "
  "F4 01 00 00 00 00 00 00 FF 7F 29 09 94 27 C6 11 2A 00 09 96 FF 93 93";

static SensorData vectorInputs() {
  SensorData d;                                   // inputs chosen to decode to the report's values
  d.gps.latitudeE7 = 424433500; d.gps.longitudeE7 = -765018800; d.gps.altitudeMSLmm = 250000; d.gps.valid = true;
  d.environmental.altitudeM = 245.0f; d.environmental.temperatureC = 23.45f;
  d.environmental.pressurePa = 101320.0f; d.environmental.humidityPercent = 45.5f; d.environmental.valid = true;
  d.imu.linearAccelerationMps2 = {1.23f, -4.56f, 9.80f};
  const float degToRad = 0.017453292519943295f;   // packet gyro is 0.1 deg/s; SensorData is rad/s
  d.imu.gyroscopeRadPerSec = {1.0f * degToRad, -2.0f * degToRad, 3.0f * degToRad};
  d.imu.magnetometerMicroTesla = {25.0f, -12.5f, 50.0f};
  d.imu.orientation.i = 0; d.imu.orientation.j = 0; d.imu.orientation.k = 0; d.imu.orientation.real = 1.0f;
  d.imu.linearAccelerationValid = d.imu.gyroscopeValid = d.imu.magnetometerValid = d.imu.orientationValid = true;
  d.stateOfCharge.cellPercentage = 75.0f; d.stateOfCharge.cellVoltage = 4.00f; d.stateOfCharge.valid = true;   // 4.00 V -> byte 150, as 75 % x2 was
  return d;
}

// A packet the board encoded, data/2026-09-09_arrival_firmware/20260909_195429_baseline_prior_fw.log lines 6-34
// Inputs are the readable values the same firmware printed just before the hex dump.
static const char *kTargetHex =
  "00 00 00 00 00 00 00 00 98 BD FF FF 23 5B 00 00 FD FF 00 00 FD FF 01 00 FF FF FF FF AC FF 18 FF "
  "D4 FD 4A 06 8E 02 ED 82 56 1A 7A 0A 7F 26 0F 11 04 00 09 6D 6F 09 23";
static SensorData targetInputs() {
  SensorData d;
  d.imu.linearAccelerationMps2 = {-0.027f, 0.004f, -0.027f};
  d.imu.gyroscopeRadPerSec = {0.0020f, -0.0020f, -0.0020f};
  d.imu.magnetometerMicroTesla = {-8.44f, -23.19f, -55.56f};
  d.imu.orientation.i = 0.04913f; d.imu.orientation.j = 0.01996f; d.imu.orientation.k = -0.97717f; d.imu.orientation.real = 0.20575f;
  d.imu.linearAccelerationValid = d.imu.gyroscopeValid = d.imu.magnetometerValid = d.imu.orientationValid = true;
  d.gps.latitudeE7 = 0; d.gps.longitudeE7 = 0; d.gps.altitudeMSLmm = -17000; d.gps.valid = false;
  d.stateOfCharge.cellPercentage = 54.73f; d.stateOfCharge.valid = true;
  d.stateOfCharge.cellVoltage = 3.59f;   // the board sent 109 as percent x2; as volts that is 3.59, so the vector stands
  d.environmental.temperatureC = 26.82f; d.environmental.pressurePa = 98554.0f;
  d.environmental.humidityPercent = 43.67f; d.environmental.altitudeM = 233.31f; d.environmental.valid = true;
  return d;
}

// Golden corpus: fixed inputs including NaN, inf, saturation and half-way cases
static uint64_t rng = 0x5EED5EED12345678ULL;
static uint32_t next32() { rng = rng * 6364136223846793005ULL + 1442695040888963407ULL; return (uint32_t)(rng >> 32); }
static float pick(float lo, float hi) {
  static const float specials[] = {NAN, INFINITY, -INFINITY, 0.0f, -0.0f, 0.005f, -0.005f, 0.125f, -0.125f,
    0.5f, -0.5f, 1.0f, -1.0f, 1.5f, 327.675f, -327.685f, 327.68f, 1e9f, -1e9f, 100.0f, 100.001f, 75.0f,
    50.0f, 35.0f, 20.0f, 6553.5f, 6553.6f, 0.0049999999f, 1e-30f};
  uint32_t r = next32();
  if (r % 4 == 0) return specials[next32() % (sizeof specials / sizeof specials[0])];
  return lo + (hi - lo) * (float)(next32() / 4294967296.0);
}
static bool coin() { return next32() & 1; }

int main(int argc, char **argv) {
  const char *vecPath = argc > 1 ? argv[1] : "vectors.txt";
  printf("sizeof(TelemetryPacket)=%zu offsetof(sensorValidity)=%zu offsetof(crc16)=%zu\n",
         sizeof(TelemetryPacket), offsetof(TelemetryPacket, sensorValidity), offsetof(TelemetryPacket, crc16));

  TelemetryPacket p;
  ChipSatTelemetry::encodePacket(vectorInputs(), 42, 9, true, p);
  printf("vector : %s\n", hexOf(p).c_str());
  check(hexOf(p) == kVectorHex, "report test vector: all 55 bytes identical");
  check(p.crc16 == 0x9393, "report test vector: CRC 0x9393");
  check(ChipSatTelemetry::packetCRCIsValid(p), "packetCRCIsValid(vector)");
  check(ChipSatTelemetry::calculateCRC16((const uint8_t *)"123456789", 9) == 0x29B1,
        "CRC-16/CCITT-FALSE catalogue check '123456789' = 0x29B1");

  ChipSatTelemetry::encodePacket(targetInputs(), 4, 9, false, p);
  printf("target : %s\n", hexOf(p).c_str());
  check(hexOf(p) == kTargetHex, "on-target packet (baseline log line 34) reproduced byte-for-byte from its printed inputs");


  struct { float soc; uint32_t ms; } iv[] = {
    {100.0f, 10000}, {75.01f, 10000}, {75.0f, 10000}, {74.99f, 30000}, {50.0f, 30000}, {49.99f, 60000},
    {35.0f, 60000}, {34.99f, 120000}, {20.0f, 120000}, {19.99f, 180000}, {0.0f, 180000}, {NAN, 180000}};
  // the table as the build ships it; a charge at exactly a threshold is in the band above it
  ChipSatSettings::Rate table{};
  const uint8_t pct[4] = {75, 50, 35, 20};
  const uint32_t ms[5] = {10000, 30000, 60000, 120000, 180000};
  for (size_t i = 0; i < 4; i++) table.socPercent[i] = pct[i];
  for (size_t i = 0; i < 5; i++) table.socIntervalMs[i] = ms[i];
  for (auto &c : iv) {
    char what[96]; uint32_t got = txIntervalFromSoc(table, c.soc);
    snprintf(what, sizeof what, "txIntervalFromSoc(%g) = %u ms (want %u)", c.soc, got, c.ms);
    check(got == c.ms, what);
  }

  FILE *f = fopen(vecPath, "w");
  const int N = 20000;
  for (int n = 0; n < N; ++n) {
    SensorData d;
    d.gps.latitudeE7 = (int32_t)next32(); d.gps.longitudeE7 = (int32_t)next32(); d.gps.altitudeMSLmm = (int32_t)next32();
    d.environmental.altitudeM = pick(-50000.f, 50000.f); d.environmental.temperatureC = pick(-400.f, 400.f);
    d.environmental.pressurePa = pick(-1000.f, 800000.f); d.environmental.humidityPercent = pick(-20.f, 150.f);
    d.imu.linearAccelerationMps2 = {pick(-400.f, 400.f), pick(-400.f, 400.f), pick(-400.f, 400.f)};
    d.imu.gyroscopeRadPerSec = {pick(-70.f, 70.f), pick(-70.f, 70.f), pick(-70.f, 70.f)};
    d.imu.magnetometerMicroTesla = {pick(-4000.f, 4000.f), pick(-4000.f, 4000.f), pick(-4000.f, 4000.f)};
    d.imu.orientation.i = pick(-1.5f, 1.5f); d.imu.orientation.j = pick(-1.5f, 1.5f);
    d.imu.orientation.k = pick(-1.5f, 1.5f); d.imu.orientation.real = pick(-1.5f, 1.5f);
    d.stateOfCharge.cellPercentage = pick(-20.f, 200.f); d.stateOfCharge.cellVoltage = pick(1.f, 6.f);
    d.imu.linearAccelerationValid = coin(); d.imu.gyroscopeValid = coin(); d.imu.magnetometerValid = coin();
    d.imu.orientationValid = coin(); d.gps.valid = coin(); d.stateOfCharge.valid = coin(); d.environmental.valid = coin();
    if (n % 3 == 0) {  // make "everything valid" common so bit 7 is exercised
      d.imu.linearAccelerationValid = d.imu.gyroscopeValid = d.imu.magnetometerValid = d.imu.orientationValid = true;
      d.gps.valid = d.stateOfCharge.valid = d.environmental.valid = true;
    }
    const uint16_t counter = (uint16_t)next32(); const uint8_t csid = (uint8_t)next32(); const bool fresh = coin();
    ChipSatTelemetry::encodePacket(d, counter, csid, fresh, p);
    // One line: counter csid fresh validbits(7) | 3 GPS ints | 19 floats as C99 hex floats (exact) | expected bytes
    fprintf(f, "%u %u %d %d%d%d%d%d%d%d | %d %d %d |", counter, csid, fresh,
            d.imu.linearAccelerationValid, d.imu.gyroscopeValid, d.imu.magnetometerValid, d.imu.orientationValid,
            d.gps.valid, d.stateOfCharge.valid, d.environmental.valid,
            d.gps.latitudeE7, d.gps.longitudeE7, d.gps.altitudeMSLmm);
    const float fl[] = {d.environmental.altitudeM, d.imu.linearAccelerationMps2.x, d.imu.linearAccelerationMps2.y,
      d.imu.linearAccelerationMps2.z, d.imu.gyroscopeRadPerSec.x, d.imu.gyroscopeRadPerSec.y, d.imu.gyroscopeRadPerSec.z,
      d.imu.magnetometerMicroTesla.x, d.imu.magnetometerMicroTesla.y, d.imu.magnetometerMicroTesla.z,
      d.imu.orientation.i, d.imu.orientation.j, d.imu.orientation.k, d.imu.orientation.real,
      d.environmental.temperatureC, d.environmental.pressurePa, d.environmental.humidityPercent,
      d.stateOfCharge.cellPercentage, d.stateOfCharge.cellVoltage};
    for (float x : fl) fprintf(f, " %a", (double)x);
    fprintf(f, " | %s\n", hexOf(p).c_str());
    if (!ChipSatTelemetry::packetCRCIsValid(p)) { ++failures; }
  }
  fclose(f);
  ++checks;
  printf("ok   golden corpus: %d vectors written to %s (every packet CRC-valid unless FAIL above)\n", N, vecPath);

  printf("%s: %d checks, %d failed\n", failures ? "FAILED" : "PASSED", checks, failures);
  return failures ? 1 : 0;
}
