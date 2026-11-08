// Streams the IMU's four flight reports at kRateHz on the flight driver, so its current can be read off
// a bench supply. Change kRateHz and reflash for another rate. The GPS goes into standby at boot so
// it doesn't move the reading.
// ./build.sh flash --sketch imu_rate_power_test
// Once a second: RATE,<ms>,la=,gy=,mg=,rv=   (reports that arrived in that second)

#include <stdio.h>

#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/I2cWatch.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/I2cBusArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/devices/own/ImuSh2.cpp"

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

constexpr uint16_t kRateHz = 1;   // flight is 20. 1 to 1000

ChipSatPlatform::System chipSystem;
ChipSatPlatform::I2cBus i2cBus;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatPlatform::Uart gpsPort(ChipSatPlatform::UartPort::Gps);
ChipSatDevices::Imu imu(i2cBus, chipSystem);

uint32_t lastMs = 0;
uint32_t lastCounts[4] = {0, 0, 0, 0};

static void print(const char *text)
{
  console.write(text);
  console.write("\r\n");
}

// UBX-RXM-PMREQ, 16 byte version: no timeout, backup and force set, woken only by UART RX. Nothing
// writes to the GPS after this, so it stays in standby until the next power cycle
static void parkGps()
{
  uint8_t frame[8 + 16] = {0xB5, 0x62, 0x02, 0x41, 16, 0};
  uint8_t *payload = frame + 6;
  payload[8] = 0x06;    // flags: backup, force
  payload[12] = 0x08;   // wakeupSources: uartrx
  uint8_t a = 0, b = 0;
  for (size_t i = 2; i < 6 + 16; ++i) {
    a = static_cast<uint8_t>(a + frame[i]);
    b = static_cast<uint8_t>(b + a);
  }
  frame[22] = a;
  frame[23] = b;
  gpsPort.write(frame, sizeof frame);
  gpsPort.flush();
}

void setup()
{
  console.begin(kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  i2cBus.begin(kI2cSdaPin, kI2cSclPin);
  gpsPort.begin(kGpsBaud);
  chipSystem.waitMs(1000);   // the receiver takes about a second after power-on to listen
  parkGps();
  char line[48];
  snprintf(line, sizeof line, "imu_rate_power_test %u Hz", static_cast<unsigned int>(kRateHz));
  print(line);
  if (imu.begin(static_cast<uint16_t>(1000 / kRateHz)) != Status::Ok) {
    print("IMU did not start");
  }
  lastMs = chipSystem.nowMs();
}

void loop()
{
  imu.service();
  const uint32_t now = chipSystem.nowMs();
  if (now - lastMs < 1000) {
    return;
  }
  const uint32_t *counts = imu.eventCounts();
  char line[96];
  snprintf(line, sizeof line, "RATE,%lu,la=%lu,gy=%lu,mg=%lu,rv=%lu", static_cast<unsigned long>(now),
           static_cast<unsigned long>(counts[0] - lastCounts[0]), static_cast<unsigned long>(counts[1] - lastCounts[1]),
           static_cast<unsigned long>(counts[2] - lastCounts[2]), static_cast<unsigned long>(counts[3] - lastCounts[3]));
  print(line);
  for (uint8_t i = 0; i < 4; ++i) {
    lastCounts[i] = counts[i];
  }
  lastMs = now;
}
