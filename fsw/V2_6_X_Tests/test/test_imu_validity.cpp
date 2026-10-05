// The stale IMU report rule, on the laptop
//   g++ -std=gnu++17 -Wall -Wextra -I../V2_6_X_Code/src/app test/test_imu_validity.cpp ../V2_6_X_Code/src/app/ImuValidity.cpp -o /tmp/stale && /tmp/stale
#include <cstdio>
#include "ImuValidity.h"

using ChipSatApp::clearStaleImuReports;
using ChipSatSensors::IMUData;

static int checks = 0, failures = 0;

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
  }
  printf("%s %s\n", ok ? "ok  " : "FAIL", what);
}

static IMUData allValid(uint32_t a, uint32_t g, uint32_t m, uint32_t q)
{
  IMUData imu;
  imu.linearAccelerationValid = imu.gyroscopeValid = imu.magnetometerValid = imu.orientationValid = true;
  imu.valid = true;
  imu.linearAccelerationUpdatedMs = a;
  imu.gyroscopeUpdatedMs = g;
  imu.magnetometerUpdatedMs = m;
  imu.orientationUpdatedMs = q;
  return imu;
}

int main()
{
  constexpr uint32_t kStale = 3000;

  IMUData imu = allValid(98700, 98710, 98720, 98730);
  uint8_t cleared = clearStaleImuReports(imu, 100000, kStale);
  check(cleared == 0 && imu.valid, "normal cycle, every report ~1.3 s old: nothing cleared");

  imu = allValid(97000, 97000, 97000, 97000);
  cleared = clearStaleImuReports(imu, 100000, kStale);
  check(cleared == 0 && imu.valid, "exactly 3000 ms old: still valid");

  imu = allValid(96999, 99000, 99000, 99000);
  cleared = clearStaleImuReports(imu, 100000, kStale);
  check(cleared == 0x01 && !imu.linearAccelerationValid && imu.gyroscopeValid && !imu.valid,
        "3001 ms old linaccel cleared, the rest kept, overall valid false");

  // the bench freeze: linaccel, gyro, quat 472 s old, mag fresh
  imu = allValid(465151, 465130, 936578, 465134);
  cleared = clearStaleImuReports(imu, 937527, kStale);
  check(cleared == 0x0B && !imu.linearAccelerationValid && !imu.gyroscopeValid && imu.magnetometerValid &&
          !imu.orientationValid && !imu.valid,
        "bench freeze (linaccel/gyro/quat 472 s old, mag fresh): bits 1011 cleared, mag kept");

  imu = allValid(0, 0, 0, 0);
  imu.orientationValid = false;
  imu.valid = false;
  cleared = clearStaleImuReports(imu, 100000, kStale);
  check(cleared == 0x07 && !imu.orientationValid, "an already invalid report isnt reported as cleared");

  imu = allValid(0xFFFFFFF0u, 0xFFFFFFF0u, 0xFFFFFFF0u, 0xFFFFF000u);
  cleared = clearStaleImuReports(imu, 0x00000010u, kStale);
  check(cleared == 0x08 && imu.linearAccelerationValid && !imu.orientationValid,
        "across the millis wrap: 32 ms old kept, 4112 ms old cleared");

  IMUData never;   // defaults: nothing ever arrived
  cleared = clearStaleImuReports(never, 100000, kStale);
  check(cleared == 0 && !never.valid, "nothing ever received: nothing to clear, stays invalid");

  printf("%s: %d checks, %d failed\n", failures ? "FAILED" : "PASSED", checks, failures);
  return failures ? 1 : 0;
}
