// Our GPS driver as the flight code runs it: NAV-PVT streaming, serviced from every wait, one read a
// second. Power-cycle only the receiver to see the settings resent. Console PB7/PB6 at 115200

#define CHIPSAT_GPS_DRIVER 2   // ours

#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/devices/own/GpsMaxM10s.cpp"

using ChipSatPlatform::Status;

ChipSatPlatform::System chipSystem;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatPlatform::Uart gpsPort(ChipSatPlatform::UartPort::Gps);
ChipSatDevices::Gps gps(gpsPort, chipSystem);

static void serviceGps()
{
  gps.service();
}

void setup()
{
  console.begin(ChipSatConfig::kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  LOG_I(Boot, "start") { line.field("test", "gpsstream"); }

  chipSystem.onWait(serviceGps);
  gpsPort.begin(ChipSatConstants::kGpsBaud);
  gps.begin();
}

void loop()
{
  const uint32_t cycleStartMs = chipSystem.nowMs();

  // read() clears the driver's counters, so take them first. The few bytes read() itself services
  // are missing from these
  const uint16_t bytes = ChipSatDevices::bytesIn;
  const uint16_t frames = ChipSatDevices::framesIn;
  const uint16_t bad = ChipSatDevices::badSums;
  const uint16_t notUbx = ChipSatDevices::notUbx;
  const uint16_t queued = ChipSatDevices::mostQueued;

  const Status status = gps.read();
  const ChipSatSensors::GPSData &data = gps.data();

  LOG_I(Gps, "bench") {
    line.field("ok", status == Status::Ok);
    line.field("status", ChipSatPlatform::statusName(status));
    line.field("fix", gps.fixType());
    line.field("sats", gps.satellites());
    line.field("valid", data.valid);
    line.fieldFixed("lat", data.latitudeE7, 7);
    line.fieldFixed("lon", data.longitudeE7, 7);
    line.field("altmm", static_cast<long>(data.altitudeMSLmm));
    line.field("bytes", bytes);
    line.field("frames", frames);
    line.field("bad", bad);
    line.field("notubx", notUbx);
    line.field("queued", queued);
  }

  const uint32_t elapsedMs = chipSystem.nowMs() - cycleStartMs;
  if (elapsedMs < 1000) {
    chipSystem.waitMs(1000 - elapsedMs);
  }
}
