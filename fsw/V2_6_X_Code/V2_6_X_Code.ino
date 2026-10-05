// ChipSat flight software. Board settings are in src/config.h

#include "src/config.h"
#include "src/platform/I2cBus.h"
#include "src/platform/System.h"
#include "src/platform/Uart.h"
#include "src/devices/EnvSensor.h"
#include "src/devices/Flash.h"
#include "src/devices/FuelGauge.h"
#include "src/devices/Gps.h"
#include "src/devices/Imu.h"
#include "src/devices/Led.h"
#include "src/devices/Radio.h"
#include "src/app/FlashLog.h"
#include "src/app/FlightController.h"

// Constructors only store references, the hardware gets set up in flight.setup()
ChipSatPlatform::System chipSystem;
ChipSatPlatform::I2cBus i2cBus;
ChipSatPlatform::Uart consolePort(ChipSatPlatform::UartPort::Console);
ChipSatPlatform::Uart gpsPort(ChipSatPlatform::UartPort::Gps);

ChipSatDevices::Imu imu(i2cBus, chipSystem);
ChipSatDevices::Gps gps(gpsPort, chipSystem);
ChipSatDevices::EnvSensor envSensor(i2cBus, chipSystem);
ChipSatDevices::FuelGauge fuelGauge(i2cBus, chipSystem);
ChipSatDevices::Radio radio(chipSystem);
ChipSatDevices::Led led;
ChipSatDevices::Flash flash(chipSystem);
ChipSatApp::FlashLog flashLog(flash, chipSystem);

ChipSatApp::FlightController flight(chipSystem, i2cBus, consolePort, gpsPort,
                                    imu, gps, envSensor, fuelGauge, radio, led, flash, flashLog);

void serviceGps()
{
  gps.service();
}

void setup()
{
  chipSystem.onWait(serviceGps);
  flight.setup();
}

void loop()
{
  flight.loop();
}
