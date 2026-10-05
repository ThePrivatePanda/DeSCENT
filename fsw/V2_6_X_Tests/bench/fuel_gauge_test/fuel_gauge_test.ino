// Reads the MAX17048 over I2C and prints charge and cell voltage once a second. Console PB7/PB6 at 115200.

#include <Wire.h>

HardwareSerial console(PB7, PB6);

constexpr uint8_t kAddress = 0x36;
constexpr uint8_t kRegVcell = 0x02;
constexpr uint8_t kRegSoc = 0x04;
constexpr uint8_t kRegVersion = 0x08;
constexpr uint8_t kRegStatus = 0x1A;

bool read16(uint8_t reg, uint16_t &value)
{
  Wire.beginTransmission(kAddress);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) {
    return false;
  }
  if (Wire.requestFrom(kAddress, (uint8_t)2) != 2) {
    return false;
  }
  value = (uint16_t)Wire.read() << 8;
  value |= Wire.read();
  return true;
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  uint16_t version = 0, status = 0;
  const bool found = read16(kRegVersion, version);
  read16(kRegStatus, status);

  console.print("gauge found=");
  console.print(found);
  console.print(" version=0x");
  console.print(version, HEX);
  console.print(" status=0x");
  console.print(status, HEX);
  console.print(" reset_indicator=");
  console.println(((status >> 8) & 0x01) != 0);
  console.println("seconds  percent  volts");
}

void loop()
{
  uint16_t soc = 0, vcell = 0;
  const bool socOk = read16(kRegSoc, soc);
  const bool vcellOk = read16(kRegVcell, vcell);

  console.print(millis() / 1000);
  console.print("  ");
  if (socOk) {
    console.print(soc / 256.0f, 2);
  } else {
    console.print("read failed");
  }
  console.print("  ");
  if (vcellOk) {
    console.println(vcell * 78.125e-6f, 3);
  } else {
    console.println("read failed");
  }

  delay(1000);
}
