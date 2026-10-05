#pragma once
// same structs as V2.5's Sensors.h, the packet encoder depends on these fields

#include <stdint.h>

namespace ChipSatSensors
{

struct Vector3f
{
  float x = 0.0f;
  float y = 0.0f;
  float z = 0.0f;
};

struct Quaternionf
{
  float i = 0.0f;
  float j = 0.0f;
  float k = 0.0f;
  float real = 1.0f;
};

struct IMUData
{
  // Gravity already taken out by the BNO085
  Vector3f linearAccelerationMps2;
  Vector3f gyroscopeRadPerSec;
  Vector3f magnetometerMicroTesla;
  Quaternionf orientation;

  bool linearAccelerationValid = false;
  bool gyroscopeValid = false;
  bool magnetometerValid = false;
  bool orientationValid = false;
  bool valid = false;

  uint32_t linearAccelerationUpdatedMs = 0;
  uint32_t gyroscopeUpdatedMs = 0;
  uint32_t magnetometerUpdatedMs = 0;
  uint32_t orientationUpdatedMs = 0;
};

struct GPSData
{
  // NAV-PVT's own units: degrees x 10^7, altitude above sea level in mm
  int32_t latitudeE7 = 0;
  int32_t longitudeE7 = 0;
  int32_t altitudeMSLmm = 0;

  bool valid = false;
  uint32_t updatedMs = 0;
};

struct StateOfChargeData
{
  float cellPercentage = 0.0f;

  bool valid = false;
  uint32_t updatedMs = 0;
};

struct EnvironmentalData
{
  float temperatureC = 0.0f;
  float pressurePa = 0.0f;
  float humidityPercent = 0.0f;
  float altitudeM = 0.0f;

  bool valid = false;
  uint32_t updatedMs = 0;
};

struct SensorData
{
  IMUData imu;
  GPSData gps;
  StateOfChargeData stateOfCharge;
  EnvironmentalData environmental;
};

} // namespace ChipSatSensors
