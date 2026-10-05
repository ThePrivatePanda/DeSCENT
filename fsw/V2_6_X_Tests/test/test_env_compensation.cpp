// Our BME280 compensation against Adafruit's, on the same calibration and the same raw readings.
// Both sets of formulas come from Bosch's datasheet section 4.2.3, so they have to agree exactly.
// Altitude is the one deliberate difference: we use powf where the library used double pow.
//   g++ -std=gnu++17 -Wall -Wextra -DCHIPSAT_ENV_DRIVER=2 -Itest/mock -ffunction-sections -Wl,--gc-sections test/test_env_compensation.cpp -o /tmp/env && /tmp/env
#include <cmath>
#include <cstdint>
#include <cstdio>
// the driver itself, compiled into this test. Its compensation functions are file-static, so the
// only way to reach them is from inside the same file. Nothing here calls the I2C parts, and
// --gc-sections drops them
#include "../../V2_6_X_Code/src/devices/own/EnvSensorBme280.cpp"

using ChipSatDevices::Bme280Calibration;

static int checks = 0;
static int failures = 0;

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
    printf("FAIL %s\n", what);
  }
}

// Adafruit_BME280 2.3.0 readTemperature/readPressure/readHumidity (Adafruit_BME280.cpp), with the
// reads replaced by the raw value and t_fine_adjust removed (we never set it)
namespace library {

struct Calib {
  uint16_t dig_T1; int16_t dig_T2, dig_T3;
  uint16_t dig_P1; int16_t dig_P2, dig_P3, dig_P4, dig_P5, dig_P6, dig_P7, dig_P8, dig_P9;
  uint8_t dig_H1; int16_t dig_H2; uint8_t dig_H3; int16_t dig_H4, dig_H5; int8_t dig_H6;
};

static int32_t t_fine;

static float readTemperature(const Calib &_bme280_calib, int32_t adc_T)
{
  int32_t var1, var2;
  var1 = (int32_t)((adc_T / 8) - ((int32_t)_bme280_calib.dig_T1 * 2));
  var1 = (var1 * ((int32_t)_bme280_calib.dig_T2)) / 2048;
  var2 = (int32_t)((adc_T / 16) - ((int32_t)_bme280_calib.dig_T1));
  var2 = (((var2 * var2) / 4096) * ((int32_t)_bme280_calib.dig_T3)) / 16384;

  t_fine = var1 + var2;

  int32_t T = (t_fine * 5 + 128) / 256;

  return (float)T / 100;
}

static float readPressure(const Calib &_bme280_calib, int32_t adc_P)
{
  int64_t var1, var2, var3, var4;

  var1 = ((int64_t)t_fine) - 128000;
  var2 = var1 * var1 * (int64_t)_bme280_calib.dig_P6;
  var2 = var2 + ((var1 * (int64_t)_bme280_calib.dig_P5) * 131072);
  var2 = var2 + (((int64_t)_bme280_calib.dig_P4) * 34359738368);
  var1 = ((var1 * var1 * (int64_t)_bme280_calib.dig_P3) / 256) +
         ((var1 * ((int64_t)_bme280_calib.dig_P2) * 4096));
  var3 = ((int64_t)1) * 140737488355328;
  var1 = (var3 + var1) * ((int64_t)_bme280_calib.dig_P1) / 8589934592;

  if (var1 == 0) {
    return 0;
  }

  var4 = 1048576 - adc_P;
  var4 = (((var4 * 2147483648) - var2) * 3125) / var1;
  var1 = (((int64_t)_bme280_calib.dig_P9) * (var4 / 8192) * (var4 / 8192)) / 33554432;
  var2 = (((int64_t)_bme280_calib.dig_P8) * var4) / 524288;
  var4 = ((var4 + var1 + var2) / 256) + (((int64_t)_bme280_calib.dig_P7) * 16);

  float P = var4 / 256.0;

  return P;
}

static float readHumidity(const Calib &_bme280_calib, int32_t adc_H)
{
  int32_t var1, var2, var3, var4, var5;

  var1 = t_fine - ((int32_t)76800);
  var2 = (int32_t)(adc_H * 16384);
  var3 = (int32_t)(((int32_t)_bme280_calib.dig_H4) * 1048576);
  var4 = ((int32_t)_bme280_calib.dig_H5) * var1;
  var5 = (((var2 - var3) - var4) + (int32_t)16384) / 32768;
  var2 = (var1 * ((int32_t)_bme280_calib.dig_H6)) / 1024;
  var3 = (var1 * ((int32_t)_bme280_calib.dig_H3)) / 2048;
  var4 = ((var2 * (var3 + (int32_t)32768)) / 1024) + (int32_t)2097152;
  var2 = ((var4 * ((int32_t)_bme280_calib.dig_H2)) + 8192) / 16384;
  var3 = var5 * var2;
  var4 = ((var3 / 32768) * (var3 / 32768)) / 128;
  var5 = var3 - ((var4 * ((int32_t)_bme280_calib.dig_H1)) / 16);
  var5 = (var5 < 0 ? 0 : var5);
  var5 = (var5 > 419430400 ? 419430400 : var5);
  uint32_t H = (uint32_t)(var5 / 4096);

  return (float)H / 1024.0;
}

static float readAltitude(float pressurePa, float seaLevel)
{
  float atmospheric = pressurePa / 100.0F;
  return 44330.0 * (1.0 - pow(atmospheric / seaLevel, 0.1903));
}

} // namespace library

// A plausible set of calibration bytes: Bosch's own datasheet example values where they are given,
// otherwise values in the range real parts return. The test is that both sides agree, not that
// these match any particular chip
static const uint8_t kCalib1[26] = {
  0x70, 0x6B, 0x43, 0x67, 0x18, 0xFC,                          // T1 27504, T2 26435, T3 -1000
  0x9A, 0x8E, 0xD8, 0xD6, 0xD0, 0x0B, 0x27, 0x0B, 0x8C, 0x00,  // P1..P5
  0xF9, 0xFF, 0x8C, 0x3C, 0xF8, 0xC6, 0x70, 0x17,              // P6..P9
  0x00, 0x4B,                                                   // (pad, H1 = 0x4B)
};
static const uint8_t kCalib2[7] = {0x62, 0x01, 0x00, 0x15, 0x2D, 0x03, 0x1E};

static void toLibrary(const Bme280Calibration &ours, library::Calib &theirs)
{
  theirs.dig_T1 = ours.t1; theirs.dig_T2 = ours.t2; theirs.dig_T3 = ours.t3;
  theirs.dig_P1 = ours.p1; theirs.dig_P2 = ours.p2; theirs.dig_P3 = ours.p3;
  theirs.dig_P4 = ours.p4; theirs.dig_P5 = ours.p5; theirs.dig_P6 = ours.p6;
  theirs.dig_P7 = ours.p7; theirs.dig_P8 = ours.p8; theirs.dig_P9 = ours.p9;
  theirs.dig_H1 = ours.h1; theirs.dig_H2 = ours.h2; theirs.dig_H3 = ours.h3;
  theirs.dig_H4 = ours.h4; theirs.dig_H5 = ours.h5; theirs.dig_H6 = ours.h6;
}

int main()
{
  Bme280Calibration cal;
  ChipSatDevices::parseCalibration(kCalib1, kCalib2, cal);

  // the unpacking, against the datasheet example and the two nibble-sharing values
  check(cal.t1 == 27504, "T1 unpacked");
  check(cal.t2 == 26435, "T2 unpacked");
  check(cal.t3 == -1000, "T3 unpacked");
  check(cal.h1 == 0x4B, "H1 comes from the end of the first block");
  check(cal.h3 == 0x00, "H3 unpacked");
  check(cal.h4 == 349, "H4 takes the low nibble of 0xE5");   // 0x15 << 4 | 0xD
  check(cal.h5 == 50, "H5 takes the high nibble of 0xE5");    // 0x03 << 4 | 0x2
  check(cal.h6 == 0x1E, "H6 unpacked");

  library::Calib theirs;
  toLibrary(cal, theirs);

  // sweep the full raw range of all three, including the ends
  int compared = 0;
  for (int32_t adcT = 0; adcT <= 1048575; adcT += 4093) {
    const int32_t tFine = ChipSatDevices::temperatureFine(cal, adcT);
    const float ourT = ChipSatDevices::temperature(tFine);
    const float theirT = library::readTemperature(theirs, adcT);
    if (ourT != theirT || tFine != library::t_fine) {
      check(false, "temperature matches the library");
      printf("  adcT=%d ours=%.6f theirs=%.6f\n", adcT, ourT, theirT);
      break;
    }

    for (int32_t adcP = 0; adcP <= 1048575; adcP += 262143) {
      const float ourP = ChipSatDevices::pressure(cal, adcP, tFine);
      const float theirP = library::readPressure(theirs, adcP);
      if (ourP != theirP) {
        check(false, "pressure matches the library");
        printf("  adcT=%d adcP=%d ours=%.6f theirs=%.6f\n", adcT, adcP, ourP, theirP);
        break;
      }
      ++compared;
    }

    for (int32_t adcH = 0; adcH <= 65535; adcH += 16383) {
      const float ourH = ChipSatDevices::humidity(cal, adcH, tFine);
      const float theirH = library::readHumidity(theirs, adcH);
      if (ourH != theirH) {
        check(false, "humidity matches the library");
        printf("  adcT=%d adcH=%d ours=%.6f theirs=%.6f\n", adcT, adcH, ourH, theirH);
        break;
      }
      ++compared;
    }
  }
  check(true, "temperature, pressure and humidity match the library across the raw range");
  printf("ok   %d value pairs compared\n", compared);

  // altitude: float against the library's double, so they differ, but only just
  float worst = 0.0f;
  for (float pressurePa = 30000.0f; pressurePa <= 110000.0f; pressurePa += 137.0f) {
    const float ours = ChipSatDevices::altitude(pressurePa, 1013.25f);
    const float theirs2 = library::readAltitude(pressurePa, 1013.25f);
    const float diff = fabsf(ours - theirs2);
    if (diff > worst) {
      worst = diff;
    }
  }
  check(worst < 0.01f, "altitude within 1 cm of the double version across 300 to 1100 hPa");
  printf("ok   altitude worst case differs by %.4f m\n", worst);

  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
