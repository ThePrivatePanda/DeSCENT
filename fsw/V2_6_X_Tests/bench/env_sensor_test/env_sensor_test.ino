// Reads the BME280 the way our driver does: reset, read the calibration, then one forced measurement
// a second. Prints temperature, pressure, humidity and altitude. Console PB7/PB6 at 115200.

#include <Wire.h>

HardwareSerial console(PB7, PB6);

constexpr uint8_t kAddress = 0x77;
constexpr uint8_t kOversampling = 0x05;   // x16 on all three, same as config.h
constexpr float kSeaLevelHpa = 1013.25f;

uint16_t t1, p1;
int16_t t2, t3, p2, p3, p4, p5, p6, p7, p8, p9, h2, h4, h5;
uint8_t h1, h3;
int8_t h6;

bool readRegs(uint8_t reg, uint8_t *buffer, uint8_t length)
{
  Wire.beginTransmission(kAddress);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) {
    return false;
  }
  if (Wire.requestFrom(kAddress, length) != length) {
    return false;
  }
  for (uint8_t i = 0; i < length; i++) {
    buffer[i] = Wire.read();
  }
  return true;
}

bool writeReg(uint8_t reg, uint8_t value)
{
  Wire.beginTransmission(kAddress);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

// Waits for a status bit to clear. A failed read is a failure, not "still busy"
bool waitClear(uint8_t mask, uint16_t timeoutMs)
{
  const uint32_t start = millis();
  while (true) {
    uint8_t status = 0;
    if (!readRegs(0xF3, &status, 1)) {
      return false;
    }
    if ((status & mask) == 0) {
      return true;
    }
    if (millis() - start >= timeoutMs) {
      return false;
    }
    delay(5);
  }
}

int16_t s16(const uint8_t *b) { return (int16_t)(b[0] | b[1] << 8); }
uint16_t u16(const uint8_t *b) { return (uint16_t)(b[0] | b[1] << 8); }

bool begin()
{
  uint8_t chipId = 0;
  if (!readRegs(0xD0, &chipId, 1)) {
    console.println("no answer at 0x77");
    return false;
  }
  console.print("chip_id=0x");
  console.println(chipId, HEX);
  if (chipId != 0x60 || !writeReg(0xE0, 0xB6)) {
    return false;
  }
  delay(10);

  uint8_t c1[26], c2[7];
  if (!waitClear(0x01, 50) || !readRegs(0x88, c1, 26) || !readRegs(0xE1, c2, 7)) {
    console.println("calibration read failed");
    return false;
  }
  t1 = u16(&c1[0]); t2 = s16(&c1[2]); t3 = s16(&c1[4]);
  p1 = u16(&c1[6]); p2 = s16(&c1[8]); p3 = s16(&c1[10]); p4 = s16(&c1[12]); p5 = s16(&c1[14]);
  p6 = s16(&c1[16]); p7 = s16(&c1[18]); p8 = s16(&c1[20]); p9 = s16(&c1[22]);
  h1 = c1[25]; h2 = s16(&c2[0]); h3 = c2[2];
  h4 = (int16_t)((int8_t)c2[3] << 4 | (c2[4] & 0x0F));
  h5 = (int16_t)((int8_t)c2[5] << 4 | (c2[4] >> 4));
  h6 = (int8_t)c2[6];

  console.print("t1=");
  console.print(t1);
  console.print(" p1=");
  console.print(p1);
  console.print(" h1=");
  console.println(h1);

  // humidity oversampling only takes effect on the next ctrl_meas write, so it goes first
  return writeReg(0xF2, kOversampling) && writeReg(0xF5, 0x00) &&
         writeReg(0xF4, kOversampling << 5 | kOversampling << 2 | 0x00);
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  const bool found = begin();
  console.print("env found=");
  console.println(found);
  console.println("seconds  celsius  hpa  humidity  metres  ms");
}

void loop()
{
  const uint32_t start = millis();
  uint8_t d[8];
  // forced. The measuring bit isn't set straight away, wait the worst case first (112.8 ms at x16)
  bool ok = writeReg(0xF4, kOversampling << 5 | kOversampling << 2 | 0x01);
  delay(113);
  ok = ok && waitClear(0x08, 100) && readRegs(0xF7, d, 8);
  const uint32_t took = millis() - start;

  console.print(millis() / 1000);
  console.print("  ");
  if (!ok) {
    console.println("read failed");
    delay(1000);
    return;
  }

  const int32_t adcP = (int32_t)d[0] << 12 | (int32_t)d[1] << 4 | d[2] >> 4;
  const int32_t adcT = (int32_t)d[3] << 12 | (int32_t)d[4] << 4 | d[5] >> 4;
  const int32_t adcH = (int32_t)d[6] << 8 | d[7];

  // Bosch datasheet 4.2.3, same as the driver
  int32_t v1 = ((adcT / 8) - ((int32_t)t1 * 2)) * t2 / 2048;
  int32_t v2 = (adcT / 16) - (int32_t)t1;
  v2 = ((v2 * v2) / 4096) * t3 / 16384;
  const int32_t tFine = v1 + v2;
  const float celsius = ((tFine * 5 + 128) / 256) / 100.0f;

  int64_t a = (int64_t)tFine - 128000;
  int64_t b = a * a * p6;
  b = b + ((a * p5) * 131072);
  b = b + ((int64_t)p4 * 34359738368);
  a = ((a * a * p3) / 256) + (a * p2 * 4096);
  a = (((int64_t)1 * 140737488355328) + a) * p1 / 8589934592;
  float pascals = 0.0f;
  if (a != 0) {
    int64_t p = 1048576 - adcP;
    p = (((p * 2147483648) - b) * 3125) / a;
    a = ((int64_t)p9 * (p / 8192) * (p / 8192)) / 33554432;
    b = ((int64_t)p8 * p) / 524288;
    p = ((p + a + b) / 256) + ((int64_t)p7 * 16);
    pascals = p / 256.0f;
  }

  int32_t x1 = tFine - 76800;
  int32_t x2 = adcH * 16384;
  int32_t x3 = (int32_t)h4 * 1048576;
  int32_t x4 = (int32_t)h5 * x1;
  int32_t x5 = (((x2 - x3) - x4) + 16384) / 32768;
  x2 = (x1 * h6) / 1024;
  x3 = (x1 * h3) / 2048;
  x4 = ((x2 * (x3 + 32768)) / 1024) + 2097152;
  x2 = ((x4 * h2) + 8192) / 16384;
  x3 = x5 * x2;
  x4 = ((x3 / 32768) * (x3 / 32768)) / 128;
  x5 = x3 - ((x4 * h1) / 16);
  x5 = x5 < 0 ? 0 : x5;
  x5 = x5 > 419430400 ? 419430400 : x5;
  const float humidity = (uint32_t)(x5 / 4096) / 1024.0f;

  const float metres = 44330.0f * (1.0f - powf(pascals / 100.0f / kSeaLevelHpa, 0.1903f));

  console.print(celsius, 2);
  console.print("  ");
  console.print(pascals / 100.0f, 2);
  console.print("  ");
  console.print(humidity, 2);
  console.print("  ");
  console.print(metres, 2);
  console.print("  ");
  console.println(took);

  delay(1000);
}
